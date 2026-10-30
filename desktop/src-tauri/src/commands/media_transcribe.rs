use crate::cli_runtime::CliVerifyRule;
use crate::command_execution::{run_app_managed_argv, AppManagedArgvRequest};
use crate::desktop_io::{
    is_service_unavailable_error, resolve_transcription_settings,
    run_curl_transcription_with_parse_format,
};
use crate::store::settings as settings_store;
use crate::{ffmpeg_program, make_id, now_ms, payload_string, workspace_root, AppState};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, State};

const OFFICIAL_AGENT_SRT_TRANSCRIPTION_MODEL: &str = "fun-asr";

#[path = "media_transcribe/subtitles.rs"]
pub(crate) mod subtitles;

use subtitles::{render_estimated_subtitles, wav_duration_seconds};

fn resolve_media_transcribe_path(
    state: &State<'_, AppState>,
    raw_path: &str,
) -> Result<PathBuf, String> {
    let normalized = raw_path.trim().replace('\\', "/");
    if normalized.is_empty() {
        return Err("media.transcribe requires a non-empty sourcePath".to_string());
    }
    let candidate = PathBuf::from(&normalized);
    let resolved = if candidate.is_absolute() {
        candidate
    } else {
        workspace_root(state)
            .map_err(|error| error.to_string())?
            .join(normalized)
    };
    if !resolved.is_file() {
        return Err(format!(
            "media.transcribe source file not found: {}",
            resolved.display()
        ));
    }
    Ok(resolved)
}

fn normalize_transcript_format(value: Option<&str>) -> (&'static str, &'static str) {
    match value.unwrap_or("srt").trim().to_ascii_lowercase().as_str() {
        "txt" | "text" => ("text", "txt"),
        "vtt" | "webvtt" => ("vtt", "vtt"),
        "json" => ("json", "json"),
        "verbose_json" | "verbose-json" => ("verbose_json", "json"),
        _ => ("srt", "srt"),
    }
}

fn transcription_route_uses_official_source(settings: &Value) -> bool {
    crate::ai_model_manager::AiModelManager::resolve(
        settings,
        crate::ai_model_manager::AiModelScope::Transcription,
        None,
    )
    .map(|route| route.is_official)
    .unwrap_or(false)
}

fn resolve_agent_srt_transcription_model(
    settings: &Value,
    endpoint: &str,
    configured_model: &str,
    response_format: &str,
) -> String {
    if response_format == "srt"
        && (crate::media_generation::is_redbox_official_endpoint(endpoint)
            || transcription_route_uses_official_source(settings))
    {
        // Temporary official-source compatibility shim: agent-generated SRT must use fun-asr
        // until the official ASR routing/model matrix is updated.
        OFFICIAL_AGENT_SRT_TRANSCRIPTION_MODEL.to_string()
    } else {
        configured_model.to_string()
    }
}

fn output_dir_for_request(
    state: &State<'_, AppState>,
    request: &Value,
    job_id: &str,
) -> Result<PathBuf, String> {
    let base = request
        .pointer("/output/directory")
        .or_else(|| request.get("outputDirectory"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .map(|path| {
            if path.is_absolute() {
                path
            } else {
                workspace_root(state)
                    .unwrap_or_else(|_| PathBuf::from("."))
                    .join(path)
            }
        })
        .unwrap_or_else(|| {
            workspace_root(state)
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(".redbox")
                .join("media-transcripts")
        });
    let dir = base.join(job_id);
    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    Ok(dir)
}

fn extract_audio_for_transcription(
    app: &AppHandle,
    state: &State<'_, AppState>,
    session_id: Option<&str>,
    source_path: &Path,
    output_dir: &Path,
) -> Result<PathBuf, String> {
    let audio_path = output_dir.join(format!("audio-{}.wav", now_ms()));

    // 两套参数，从严到宽。
    //
    // 第一套是常规参数；第二套是**兜底**：显式指定 `-f wav`、放宽到任意采样率、
    // 并加 `-err_detect ignore_err` 让 ffmpeg 对局部损坏的帧宽容一些。
    //
    // 为什么需要兜底：源文件是从平台流式下载来的 mp4，网络抖动可能留下
    // 「容器不完整 / moov 索引异常 / 某些帧解不出来」的文件。严格模式下 ffmpeg
    // 直接退出 255，实测一批 41 条里会有个别条栽在这里 —— 而它其实仍然有可用的音频。
    let attempts: Vec<Vec<String>> = vec![
        vec![
            "-y".to_string(),
            "-i".to_string(),
            source_path.display().to_string(),
            "-vn".to_string(),
            "-ac".to_string(),
            "1".to_string(),
            "-ar".to_string(),
            "16000".to_string(),
            "-acodec".to_string(),
            "pcm_s16le".to_string(),
            audio_path.display().to_string(),
        ],
        vec![
            "-y".to_string(),
            "-err_detect".to_string(),
            "ignore_err".to_string(),
            "-i".to_string(),
            source_path.display().to_string(),
            "-vn".to_string(),
            "-f".to_string(),
            "wav".to_string(),
            "-ac".to_string(),
            "1".to_string(),
            "-ar".to_string(),
            "16000".to_string(),
            "-acodec".to_string(),
            "pcm_s16le".to_string(),
            audio_path.display().to_string(),
        ],
    ];

    let mut last_error = String::new();
    for (index, args) in attempts.iter().enumerate() {
        // 每次重试都换文件名，避免上一轮的半截产物被 FileExists 规则误判为成功。
        let attempt_path = if index == 0 {
            audio_path.clone()
        } else {
            output_dir.join(format!("audio-{}-retry.wav", now_ms()))
        };
        let mut argv_args = args.clone();
        if index > 0 {
            // 替换输出路径参数（最后一个）。
            if let Some(last) = argv_args.last_mut() {
                *last = attempt_path.display().to_string();
            }
        }
        let argv = std::iter::once(ffmpeg_program(Some(app))?)
            .chain(argv_args)
            .collect::<Vec<_>>();
        let result = run_app_managed_argv(
            app,
            state,
            AppManagedArgvRequest {
                session_id: session_id.map(ToString::to_string),
                runtime_id: Some("media-transcribe".to_string()),
                tool_id: Some("ffmpeg".to_string()),
                argv,
                cwd: Some(output_dir.to_string_lossy().to_string()),
                verification_rules: vec![
                    CliVerifyRule::ExitCode { expected: Some(0) },
                    CliVerifyRule::FileExists {
                        path: attempt_path.to_string_lossy().to_string(),
                    },
                ],
                ..AppManagedArgvRequest::default()
            },
            8_000,
        );
        match result {
            Ok(_) => {
                // FileExists 只验「文件存在」，不验「有内容」。
                // ffmpeg 退出 0 但产出 0 字节（源文件损坏但容器仍可解析）时，
                // 会拿到一个空 wav 送去 ASR —— 结果必然是空转写或下游报错。
                // 这里显式检查大小，为 0 视同失败、走降级重试。
                let size = std::fs::metadata(&attempt_path).map(|m| m.len()).unwrap_or(0);
                if size == 0 {
                    let _ = std::fs::remove_file(&attempt_path);
                    last_error = format!(
                        "ffmpeg 抽音频退出码为 0 但产出 0 字节（源文件可能损坏）：{}",
                        source_path.display()
                    );
                    if index + 1 < attempts.len() {
                        println!("[media-transcribe] {}", last_error);
                        continue;
                    }
                    break;
                }
                return Ok(attempt_path);
            }
            Err(error) => {
                let text = error.to_string();
                // 第一套参数失败才试第二套；两次都失败才报错。
                if index + 1 < attempts.len() {
                    println!(
                        "[media-transcribe] ffmpeg 抽音频第 {} 次失败，降级重试：{}",
                        index + 1,
                        text.lines()
                            .rev()
                            .filter(|line| !line.trim().is_empty())
                            .take(2)
                            .collect::<Vec<_>>()
                            .join(" | ")
                    );
                    last_error = text;
                    continue;
                }
                last_error = text;
            }
        }
    }

    Err(describe_audio_extract_failure(&last_error, source_path))
}

/// 把 ffmpeg 抽音频的失败原因翻译成人话。
///
/// 历史上这里只能拿到「退出码校验失败：期望 0，实际 255」——
/// 验证器把命令自己的 stderr 吞了，ffmpeg 真正报的那行完全看不到。
/// 现在 stderr 会一并冒泡上来，这里据此分类。
fn describe_audio_extract_failure(detail: &str, source_path: &Path) -> String {
    let text = detail.to_string();
    let lower = text.to_ascii_lowercase();
    let file = source_path.display().to_string();
    // 常见且极难定位的失败：源视频是抖音 DASH 的**纯视频轨**，抽不出音频。
    if lower.contains("does not contain any stream")
        || lower.contains("does not contain a stream")
        || lower.contains("output file does not contain")
    {
        return format!("该视频没有音轨，无法转写（源文件只含视频轨，抖音 DASH 需先补音轨）：{text}");
    }
    // 文件本身读不出来：下载中断 / 容器损坏。
    if lower.contains("invalid data found")
        || lower.contains("moov atom not found")
        || lower.contains("end of file")
        || lower.contains("no such file or directory")
        || lower.contains("invalid argument")
    {
        return format!(
            "视频文件不完整或已损坏，无法抽取音频（多为下载中断所致，建议重新下载）：{file}；{text}"
        );
    }
    format!("ffmpeg audio extraction failed: {file}；{text}")
}

fn text_preview(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn transcribe_with_subtitle_fallback(
    endpoint: &str,
    api_key: Option<&str>,
    model_name: &str,
    audio_path: &Path,
    response_format: &'static str,
) -> Result<(String, &'static str), String> {
    let mut errors = Vec::new();
    // 每种格式都带「内部 3 次重试」，三种格式全试一遍就是 9 次请求。
    // 一旦判定为服务端不可用（网关超时/连接重置），换格式必然同样失败，必须立刻中止，
    // 否则单条视频最坏要等 9×60s ≈ 9 分钟才报错。
    match run_curl_transcription_with_parse_format(
        endpoint,
        api_key,
        model_name,
        audio_path,
        "audio/wav",
        Some(response_format),
        Some(response_format),
    ) {
        Ok(transcript) => return Ok((transcript, "provider")),
        Err(error) if is_service_unavailable_error(&error) => {
            return Err(format!("{response_format}: {error}"));
        }
        Err(error) => errors.push(format!("{response_format}: {error}")),
    }
    match run_curl_transcription_with_parse_format(
        endpoint,
        api_key,
        model_name,
        audio_path,
        "audio/wav",
        Some("verbose_json"),
        Some(response_format),
    ) {
        Ok(transcript) => return Ok((transcript, "segments")),
        Err(error) if is_service_unavailable_error(&error) => {
            return Err(format!("verbose_json: {error}"));
        }
        Err(error) => errors.push(format!("verbose_json: {error}")),
    }
    match run_curl_transcription_with_parse_format(
        endpoint,
        api_key,
        model_name,
        audio_path,
        "audio/wav",
        None,
        Some("text"),
    ) {
        Ok(text) => {
            let duration = wav_duration_seconds(audio_path).unwrap_or(0.0);
            return render_estimated_subtitles(&text, duration, response_format)
                .map(|subtitles| (subtitles, "estimated"));
        }
        Err(error) if is_service_unavailable_error(&error) => {
            return Err(format!("text: {error}"));
        }
        Err(error) => errors.push(format!("text: {error}")),
    }
    Err(format!("无法生成字幕：{}", errors.join("；")))
}

pub(crate) fn execute_media_transcribe(
    app: &AppHandle,
    state: &State<'_, AppState>,
    session_id: Option<&str>,
    request: &Value,
) -> Result<Value, String> {
    let source_path = request
        .get("sourcePath")
        .or_else(|| request.get("path"))
        .or_else(|| request.get("toolPath"))
        .and_then(Value::as_str)
        .ok_or_else(|| "media.transcribe requires sourcePath".to_string())?;
    let source_path = resolve_media_transcribe_path(state, source_path)?;
    let (response_format, extension) = normalize_transcript_format(
        request
            .get("format")
            .or_else(|| request.get("responseFormat"))
            .and_then(Value::as_str),
    );
    let settings_snapshot = crate::persistence::with_store(state, |store| {
        Ok(settings_store::settings_snapshot(&store))
    })?;
    let Some((endpoint, api_key, model_name)) = resolve_transcription_settings(&settings_snapshot)
    else {
        return Err(
            "未配置音频转写服务，请先在设置中填写 transcription endpoint/model。".to_string(),
        );
    };
    let effective_model_name = resolve_agent_srt_transcription_model(
        &settings_snapshot,
        &endpoint,
        &model_name,
        response_format,
    );

    let job_id = make_id("media-transcribe-job");
    let output_dir = output_dir_for_request(state, request, &job_id)?;
    let audio_path =
        extract_audio_for_transcription(app, state, session_id, &source_path, &output_dir)?;
    let (transcript, timing_mode) = if matches!(response_format, "srt" | "vtt") {
        transcribe_with_subtitle_fallback(
            &endpoint,
            api_key.as_deref(),
            &effective_model_name,
            &audio_path,
            response_format,
        )?
    } else {
        (
            run_curl_transcription_with_parse_format(
                &endpoint,
                api_key.as_deref(),
                &effective_model_name,
                &audio_path,
                "audio/wav",
                Some(response_format),
                Some(response_format),
            )?,
            "provider",
        )
    };

    let output_path = output_dir.join(format!("transcript.{extension}"));
    fs::write(&output_path, &transcript).map_err(|error| error.to_string())?;

    let language = payload_string(request, "language");
    // 字幕格式（srt/vtt）的产物带序号和时间轴。写进字幕文件是对的，但**不能**原样放进
    // `transcript` 字段 —— 详情页会显示成一堆 `00:00:00,800 --> ...`，下游 LLM 摘要也会被时间轴污染。
    // 这里额外准备一份纯文本；需要时间轴的调用方读 `subtitleText` 或 `subtitlePath`。
    let plain_transcript = if matches!(response_format, "srt" | "vtt") {
        strip_subtitle_markup(&transcript)
    } else {
        transcript.clone()
    };
    Ok(json!({
        "success": true,
        "jobId": job_id,
        "sourcePath": source_path.display().to_string(),
        "audioPath": audio_path.display().to_string(),
        "format": response_format,
        "language": language,
        "subtitlePath": output_path.display().to_string(),
        "transcriptPath": output_path.display().to_string(),
        "outputDir": output_dir.display().to_string(),
        "contentPreview": text_preview(&plain_transcript, 4000),
        "contentChars": plain_transcript.chars().count(),
        "timingMode": timing_mode,
        "modelName": effective_model_name,
        // 全文纯文本（未截断）。调用方要完整内容时直接取它，别用 `contentPreview`（那只有 4000 字）。
        "transcript": plain_transcript,
        // 原始字幕（含序号与时间轴）
        "subtitleText": transcript,
    }))
}

/// 从 SRT/VTT 里剥出纯文本：去掉 `WEBVTT` 头、纯数字序号行、`-->` 时间轴行。
///
/// 每条 cue 保留为一行 —— 前端是 `whitespace-pre-wrap` 渲染，分行比拼成一整段更易读。
fn strip_subtitle_markup(subtitles: &str) -> String {
    let mut lines: Vec<&str> = Vec::new();
    for line in subtitles.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("WEBVTT") {
            continue;
        }
        if trimmed.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        if trimmed.contains("-->") {
            continue;
        }
        lines.push(trimmed);
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcript_format_defaults_to_srt() {
        assert_eq!(normalize_transcript_format(None), ("srt", "srt"));
        assert_eq!(normalize_transcript_format(Some("txt")), ("text", "txt"));
        assert_eq!(
            normalize_transcript_format(Some("verbose-json")),
            ("verbose_json", "json")
        );
    }

    #[test]
    fn strips_srt_timeline_from_transcript() {
        // 字幕文件保留时间轴，但 transcript 字段必须是纯文本
        let srt = "1\n00:00:00,800 --> 00:00:06,840\n我和我的祖国，一刻也不能分割。\n\n2\n00:00:08,640 --> 00:00:14,220\n无论我走到哪里。\n";
        assert_eq!(
            strip_subtitle_markup(srt),
            "我和我的祖国，一刻也不能分割。\n无论我走到哪里。"
        );
    }

    #[test]
    fn strips_vtt_header_and_timeline() {
        let vtt = "WEBVTT\n\n00:00:00.000 --> 00:00:01.000\n你好\n\n00:00:01.000 --> 00:00:02.000\n世界\n";
        assert_eq!(strip_subtitle_markup(vtt), "你好\n世界");
    }

    #[test]
    fn renders_estimated_srt_from_plain_text() {
        let rendered =
            render_estimated_subtitles("第一句内容比较长。第二句内容也比较长。", 10.0, "srt")
                .expect("estimated subtitles");

        assert!(rendered.contains("1\n00:00:00,000 --> "));
        assert!(rendered.contains("--> 00:00:10,000"));
    }

    #[test]
    fn rejects_gateway_error_as_estimated_subtitle_text() {
        let error = render_estimated_subtitles("Bad Gateway", 93.0, "srt").expect_err("gateway");

        assert!(error.contains("上游错误"));
    }

    #[test]
    fn official_agent_srt_transcription_forces_fun_asr() {
        let settings = json!({
            "ai_model_routes_json": serde_json::to_string(&json!({
                "transcription": { "sourceId": "redbox_official_auto" }
            })).unwrap()
        });

        let model = resolve_agent_srt_transcription_model(
            &settings,
            "https://example.com/v1",
            "user-selected-asr",
            "srt",
        );

        assert_eq!(model, "fun-asr");
    }

    #[test]
    fn official_agent_non_srt_transcription_preserves_user_model() {
        let settings = json!({
            "ai_model_routes_json": serde_json::to_string(&json!({
                "transcription": { "sourceId": "redbox_official_auto" }
            })).unwrap()
        });

        let model = resolve_agent_srt_transcription_model(
            &settings,
            "https://example.com/v1",
            "user-selected-asr",
            "text",
        );

        assert_eq!(model, "user-selected-asr");
    }

    #[test]
    fn custom_agent_srt_transcription_preserves_user_model() {
        let settings = json!({
            "ai_model_routes_json": serde_json::to_string(&json!({
                "transcription": { "sourceId": "custom-source" }
            })).unwrap(),
            "ai_sources_json": serde_json::to_string(&json!([
                { "id": "custom-source", "presetId": "custom" }
            ])).unwrap()
        });

        let model = resolve_agent_srt_transcription_model(
            &settings,
            "https://example.com/v1",
            "user-selected-asr",
            "srt",
        );

        assert_eq!(model, "user-selected-asr");
    }
}
