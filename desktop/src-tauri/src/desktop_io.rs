use arboard::{Clipboard, ImageData};
use base64::Engine;
use image::ImageReader;
use serde_json::Value;
use std::borrow::Cow;
use std::fs;
use std::path::{Path, PathBuf};

use crate::{background_command, normalize_base_url, now_iso, payload_string, AdvisorVideoRecord};

pub(crate) fn write_base64_payload_to_file(
    encoded: &str,
    output_path: &Path,
) -> Result<(), String> {
    let normalized = encoded
        .trim()
        .split_once(',')
        .map(|(_, payload)| payload.trim())
        .filter(|payload| !payload.is_empty())
        .unwrap_or_else(|| encoded.trim());
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(normalized)
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(normalized))
        .map_err(|error| format!("base64 decode failed: {error}"))?;
    fs::write(output_path, bytes).map_err(|error| error.to_string())?;
    Ok(())
}

/// ElevenLabs 的 STT 协议与 OpenAI 完全不同，靠 endpoint 域名自动识别，无需额外配置项：
/// - 路径：`/v1/speech-to-text`（不是 `/audio/transcriptions`）
/// - 认证：`xi-api-key: <key>`（不是 `Authorization: Bearer`）
/// - 模型参数：`model_id`（不是 `model`）
/// - 响应：`{ language_code, text, words[] }`，没有 srt / verbose_json
pub(crate) fn is_elevenlabs_endpoint(endpoint: &str) -> bool {
    endpoint.to_ascii_lowercase().contains("elevenlabs.io")
}

pub(crate) fn normalize_transcription_url(endpoint: &str) -> String {
    let normalized = normalize_base_url(endpoint);
    if is_elevenlabs_endpoint(&normalized) {
        return if normalized.ends_with("/speech-to-text") {
            normalized
        } else {
            format!("{normalized}/speech-to-text")
        };
    }
    if normalized.ends_with("/audio/transcriptions") {
        normalized
    } else {
        format!("{normalized}/audio/transcriptions")
    }
}

pub(crate) fn run_curl_transcription(
    endpoint: &str,
    api_key: Option<&str>,
    model_name: &str,
    file_path: &Path,
    mime_type: &str,
) -> Result<String, String> {
    run_curl_transcription_with_response_format(
        endpoint, api_key, model_name, file_path, mime_type, None,
    )
}

pub(crate) fn run_curl_transcription_with_response_format(
    endpoint: &str,
    api_key: Option<&str>,
    model_name: &str,
    file_path: &Path,
    mime_type: &str,
    response_format: Option<&str>,
) -> Result<String, String> {
    run_curl_transcription_with_parse_format(
        endpoint,
        api_key,
        model_name,
        file_path,
        mime_type,
        response_format,
        None,
    )
}

pub(crate) fn run_curl_transcription_with_parse_format(
    endpoint: &str,
    api_key: Option<&str>,
    model_name: &str,
    file_path: &Path,
    mime_type: &str,
    response_format: Option<&str>,
    parse_format: Option<&str>,
) -> Result<String, String> {
    let mut last_error = None;
    for attempt in 1..=3 {
        match run_curl_transcription_request(
            endpoint,
            api_key,
            model_name,
            file_path,
            mime_type,
            response_format,
        ) {
            Ok(stdout) => {
                let preferred_format = parse_format
                    .or(response_format)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or("json");
                match parse_transcription_response(&stdout, preferred_format) {
                    Ok(transcript) => return Ok(transcript),
                    Err(error) if attempt < 3 && is_retryable_transcription_error(&error) => {
                        last_error = Some(error);
                        std::thread::sleep(std::time::Duration::from_millis(match attempt {
                            1 => 1_000,
                            2 => 3_000,
                            _ => 0,
                        }));
                    }
                    Err(error) => return Err(error),
                }
            }
            Err(error) if attempt < 3 && is_retryable_transcription_error(&error) => {
                last_error = Some(error);
                std::thread::sleep(std::time::Duration::from_millis(match attempt {
                    1 => 1_000,
                    2 => 3_000,
                    _ => 0,
                }));
            }
            Err(error) => return Err(error),
        }
    }
    Err(format!(
        "转写接口连续重试失败：{}",
        last_error.unwrap_or_else(|| "unknown transcription error".to_string())
    ))
}

pub(crate) fn run_curl_transcription_request(
    endpoint: &str,
    api_key: Option<&str>,
    model_name: &str,
    file_path: &Path,
    mime_type: &str,
    response_format: Option<&str>,
) -> Result<String, String> {
    const HTTP_STATUS_MARKER: &str = "\n__redbox_http_status__:";
    let eleven = is_elevenlabs_endpoint(endpoint);
    let mut command = background_command("curl");
    command
        .arg("-sS")
        // 没有超时上限时，服务端卡死会让请求一直挂着（网关 60s 掐断 + 内部重试会把单条视频拖到几分钟）。
        // 120s 足够覆盖长音频，同时保证失败能被及时看到。
        .arg("--max-time")
        .arg("120")
        .arg("--connect-timeout")
        .arg("10")
        .arg("-X")
        .arg("POST")
        .arg(normalize_transcription_url(endpoint))
        // 模型字段名两套协议不同：ElevenLabs 用 model_id，OpenAI 兼容接口用 model
        .arg("-F")
        .arg(if eleven {
            format!("model_id={model_name}")
        } else {
            format!("model={model_name}")
        })
        .arg("-F")
        .arg(format!("file=@{};type={mime_type}", file_path.display()))
        .arg("-w")
        .arg(format!("{HTTP_STATUS_MARKER}%{{http_code}}"));
    if eleven {
        // 我们的音频是 16kHz/单声道/16bit PCM，正好命中 ElevenLabs 的低延迟输入通道
        command.arg("-F").arg("file_format=pcm_s16le_16");
    } else if let Some(format) = response_format
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        // response_format 只有 OpenAI 兼容接口认；给 ElevenLabs 传会直接 422
        command.arg("-F").arg(format!("response_format={format}"));
    }
    if let Some(key) = api_key.map(str::trim).filter(|value| !value.is_empty()) {
        command.arg("-H").arg(if eleven {
            format!("xi-api-key: {key}")
        } else {
            format!("Authorization: Bearer {key}")
        });
    }
    let output = command.output().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            "转写接口不可用：未找到 curl 程序".to_string()
        } else {
            error.to_string()
        }
    })?;
    let raw_stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !output.status.success() {
        return Err(if stderr.is_empty() {
            format!("curl failed with status {}", output.status)
        } else {
            explain_curl_failure(&stderr)
        });
    }
    let (stdout, http_status) = split_curl_http_status(&raw_stdout, HTTP_STATUS_MARKER)?;
    if !(200..300).contains(&http_status) {
        // 原样抛 `HTTP 504: Gateway Timeout` 用户无从判断该找谁；按状态码给出可执行结论。
        let hint = match http_status {
            499 | 504 => {
                "（网关超时：服务端约 60 秒未返回，通常是转写服务商上游故障，重试无效 —— 请联系服务商或更换 STT 服务）"
            }
            401 | 403 => {
                "（鉴权失败：请求用的 key 无效 —— 请核对「设置 > 语音转写」里配置的 \
                 transcription endpoint 与 key 是否属于同一个服务；官方登录态过期也会这样）"
            }
            404 => "（接口不存在：检查 transcription endpoint 是否该带 /v1 或模型是否支持该接口）",
            413 => "（音频过大：触发服务端请求体上限）",
            429 => "（触发限流：稍后再试或降低转写并发）",
            502 | 503 => "（服务端暂时不可用）",
            _ => "",
        };
        // 把**实际请求的地址**带上：401 时用户最需要知道的是「打到了哪个服务」——
        // 打到官方服务却以为是自己的 STT key 失效，是最容易白折腾一轮的情况。
        // 模型名一并带上：ElevenLabs 的 400 unsupported_model 只说"这个名字不认识"，
        // 不说"我们发的是哪个名字"。
        return Err(format!(
            "转写接口 HTTP {http_status}{hint}；请求地址：{}；模型：{model_name}；响应：{}",
            normalize_transcription_url(endpoint),
            stdout.trim()
        ));
    }
    if stdout.trim().is_empty() {
        return Err("转写接口返回了空结果：服务响应 stdout 为空".to_string());
    }
    Ok(stdout.trim().to_string())
}

fn split_curl_http_status(stdout: &str, marker: &str) -> Result<(String, u16), String> {
    let Some((body, status)) = stdout.rsplit_once(marker) else {
        return Err("转写接口响应缺少 HTTP 状态码".to_string());
    };
    let status = status
        .trim()
        .parse::<u16>()
        .map_err(|error| format!("转写接口 HTTP 状态码无效：{error}"))?;
    Ok((body.to_string(), status))
}

/// 把 curl 的裸错误翻译成可执行结论。
///
/// 此前这类错误原样抛给前端，用户只能看到 `curl: (55) Send failure: Connection was reset`，
/// 既不知道是自己配错了还是服务商挂了，也不知道要不要重试。
fn explain_curl_failure(stderr: &str) -> String {
    let lower = stderr.to_ascii_lowercase();
    let hint = if lower.contains("connection was reset")
        || lower.contains("send failure")
        || lower.contains("recv failure")
        || lower.contains("(55)")
        || lower.contains("(56)")
    {
        "连接被服务端中途重置 —— 通常是转写服务商上游卡死或被网关掐断，重试一般无效，请联系服务商或更换 STT 服务"
    } else if lower.contains("(28)") || lower.contains("timed out") || lower.contains("timeout") {
        "请求超时（服务端迟迟不返回）—— 转写服务商上游可能已故障"
    } else if lower.contains("could not resolve") || lower.contains("(6)") {
        "转写服务域名解析失败 —— 检查 endpoint 拼写或本机 DNS/网络"
    } else if lower.contains("failed to connect") || lower.contains("(7)") {
        "无法连接到转写服务 —— 请检查设置里的 transcription endpoint 是否可访问"
    } else if lower.contains("ssl") || lower.contains("(35)") || lower.contains("(60)") {
        "TLS 握手失败 —— 检查 endpoint 是否为 https 且证书有效"
    } else {
        return stderr.to_string();
    };
    format!("{hint}（原始错误：{stderr}）")
}

fn is_retryable_transcription_error(error: &str) -> bool {
    let normalized = error.to_ascii_lowercase();
    // 只对「可能是瞬时抖动」的错误重试。
    // 504 / 499 / 连接重置属于服务端卡死，重试只会把等待时间翻倍（60s → 180s），必须排除。
    normalized.contains("http 502")
        || normalized.contains("http 503")
        || normalized.contains("empty")
        || normalized.contains("空结果")
}

/// 服务端「不可用」类错误：换一种 response_format 再请求也必然同样失败，
/// 调用方应据此立刻中止回退链，而不是把三种格式各试一遍、把等待时间乘三。
pub(crate) fn is_service_unavailable_error(error: &str) -> bool {
    let normalized = error.to_ascii_lowercase();
    normalized.contains("http 499")
        || normalized.contains("http 500")
        || normalized.contains("http 502")
        || normalized.contains("http 503")
        || normalized.contains("http 504")
        || normalized.contains("connection was reset")
        || normalized.contains("send failure")
        || normalized.contains("timed out")
        || normalized.contains("请求超时")
        || normalized.contains("连接被服务端中途重置")
        || normalized.contains("无法连接到转写服务")
}

pub(crate) fn parse_transcription_response(
    stdout: &str,
    preferred_format: &str,
) -> Result<String, String> {
    let stdout = stdout.trim();
    if stdout.is_empty() {
        return Err("转写接口返回了空结果：服务响应 stdout 为空".to_string());
    }
    if looks_like_transcription_gateway_error(stdout) {
        return Err(format!("转写接口上游错误：{stdout}"));
    }
    if preferred_format != "json" && !stdout.starts_with('{') && !stdout.starts_with('[') {
        if is_subtitle_format(preferred_format) && !looks_like_subtitle(stdout, preferred_format) {
            return Err(format!(
                "转写接口返回了纯文本，不是 {} 字幕：缺少字幕序号/时间轴",
                preferred_format.to_ascii_uppercase()
            ));
        }
        return Ok(stdout.to_string());
    }

    let value: Value =
        serde_json::from_str(&stdout).map_err(|error| format!("Invalid JSON response: {error}"))?;
    if let Some(message) = transcription_error_message(&value) {
        return Err(format!("转写接口返回错误：{message}"));
    }
    let transcript = extract_transcription_text(&value, preferred_format).ok_or_else(|| {
        format!(
            "转写接口返回了空结果：未在响应中找到可用转写字段（{}）",
            transcription_response_shape(&value)
        )
    })?;
    if is_subtitle_format(preferred_format) && !looks_like_subtitle(&transcript, preferred_format) {
        return Err(format!(
            "转写接口返回了转录文本，但不是 {} 字幕：缺少字幕序号/时间轴（{}）",
            preferred_format.to_ascii_uppercase(),
            transcription_response_shape(&value)
        ));
    }
    Ok(transcript)
}

fn looks_like_transcription_gateway_error(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return false;
    }
    matches!(
        normalized.as_str(),
        "bad gateway" | "gateway timeout" | "service unavailable" | "upstream timeout"
    ) || normalized.contains("502 bad gateway")
        || normalized.contains("503 service unavailable")
        || normalized.contains("504 gateway timeout")
        || normalized.contains("upstream request timeout")
}

fn extract_transcription_text(value: &Value, preferred_format: &str) -> Option<String> {
    if is_subtitle_format(preferred_format) {
        if let Some(text) = value
            .get(preferred_format)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|item| !item.is_empty())
        {
            return Some(text.to_string());
        }
        if let Some(data) = value.get("data") {
            if let Some(text) = extract_transcription_text(data, preferred_format) {
                return Some(text);
            }
        }
        // ElevenLabs(scribe) 返回的是**词级时间戳** `words[]`，既不是 segments 也不是 srt/vtt，
        // 通用分支认不出 → 这里走词级渲染（真实时间戳，与画面精确对齐）。
        if let Some(words) = value.get("words").and_then(Value::as_array) {
            if let Some(rendered) =
                crate::commands::media_transcribe::subtitles::render_subtitles_from_words(
                    words,
                    preferred_format,
                )
            {
                return Some(rendered);
            }
        }
        return extract_timed_segments(value, preferred_format);
    }

    for key in ["text", "transcript", "srt", "vtt", "content", "result"] {
        if let Some(text) = value
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|item| !item.is_empty())
        {
            return Some(text.to_string());
        }
    }
    if let Some(data) = value.get("data") {
        if let Some(text) = extract_transcription_text(data, preferred_format) {
            return Some(text);
        }
    }
    if let Some(text) = extract_timed_segments(value, preferred_format) {
        return Some(text);
    }
    None
}

fn extract_timed_segments(value: &Value, preferred_format: &str) -> Option<String> {
    let segments = value
        .get("segments")
        .or_else(|| value.get("sentences"))
        .or_else(|| value.pointer("/data/segments"))
        .or_else(|| value.pointer("/data/sentences"))
        .and_then(Value::as_array)?;
    if segments.is_empty() {
        return None;
    }
    if matches!(preferred_format, "srt" | "vtt") {
        let rendered = render_timed_segments(segments, preferred_format);
        if !rendered.trim().is_empty() {
            return Some(rendered);
        }
    }
    let text = segments
        .iter()
        .filter_map(segment_text)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn segment_text(segment: &Value) -> Option<String> {
    if let Some(text) = segment
        .as_str()
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        return Some(text.to_string());
    }
    for key in ["text", "transcript", "content", "sentence"] {
        if let Some(text) = segment
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|item| !item.is_empty())
        {
            return Some(text.to_string());
        }
    }
    None
}

fn segment_time_seconds(segment: &Value, keys: &[&str]) -> Option<f64> {
    for key in keys {
        if let Some(number) = segment.get(key).and_then(Value::as_f64) {
            return Some(if number > 10_000.0 {
                number / 1000.0
            } else {
                number
            });
        }
        if let Some(text) = segment.get(key).and_then(Value::as_str) {
            if let Ok(number) = text.trim().parse::<f64>() {
                return Some(if number > 10_000.0 {
                    number / 1000.0
                } else {
                    number
                });
            }
        }
    }
    None
}

fn segment_duration_seconds(segment: &Value) -> Option<f64> {
    segment_time_seconds(segment, &["duration", "duration_s", "durationSeconds"])
}

fn format_srt_time(seconds: f64) -> String {
    let millis = (seconds.max(0.0) * 1000.0).round() as u64;
    let hours = millis / 3_600_000;
    let minutes = (millis % 3_600_000) / 60_000;
    let seconds = (millis % 60_000) / 1000;
    let millis = millis % 1000;
    format!("{hours:02}:{minutes:02}:{seconds:02},{millis:03}")
}

fn format_vtt_time(seconds: f64) -> String {
    format_srt_time(seconds).replace(',', ".")
}

fn render_timed_segments(segments: &[Value], preferred_format: &str) -> String {
    let mut output = String::new();
    if preferred_format == "vtt" {
        output.push_str("WEBVTT\n\n");
    }
    let mut cue_index = 1usize;
    for segment in segments {
        let Some(text) = segment_text(segment) else {
            continue;
        };
        let start = segment_time_seconds(segment, &["start", "start_time", "startTime", "begin"])
            .or_else(|| segment_time_seconds(segment, &["from", "from_time", "fromTime"]));
        let Some(start) = start else {
            continue;
        };
        let end = segment_time_seconds(
            segment,
            &["end", "end_time", "endTime", "to", "to_time", "toTime"],
        )
        .or_else(|| segment_duration_seconds(segment).map(|duration| start + duration));
        let Some(end) = end else {
            continue;
        };
        if preferred_format == "srt" {
            output.push_str(&format!(
                "{}\n{} --> {}\n{}\n\n",
                cue_index,
                format_srt_time(start),
                format_srt_time(end.max(start + 0.1)),
                text
            ));
        } else {
            output.push_str(&format!(
                "{} --> {}\n{}\n\n",
                format_vtt_time(start),
                format_vtt_time(end.max(start + 0.1)),
                text
            ));
        }
        cue_index += 1;
    }
    output
}

fn is_subtitle_format(format: &str) -> bool {
    matches!(format, "srt" | "vtt")
}

fn looks_like_subtitle(value: &str, format: &str) -> bool {
    let trimmed = value.trim_start();
    if format == "vtt" && trimmed.starts_with("WEBVTT") {
        return true;
    }
    if format == "srt"
        && trimmed
            .lines()
            .any(|line| line.contains(" --> ") && line.contains(','))
    {
        return true;
    }
    trimmed.lines().any(|line| line.contains(" --> "))
}

fn transcription_response_shape(value: &Value) -> String {
    match value {
        Value::Object(object) => {
            let keys = object
                .keys()
                .take(12)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");
            let segment_count = value
                .get("segments")
                .or_else(|| value.get("sentences"))
                .or_else(|| value.pointer("/data/segments"))
                .or_else(|| value.pointer("/data/sentences"))
                .and_then(Value::as_array)
                .map(|items| items.len());
            match segment_count {
                Some(count) => format!("topLevelKeys=[{keys}], segmentCount={count}"),
                None => format!("topLevelKeys=[{keys}]"),
            }
        }
        Value::Array(items) => format!("arrayLength={}", items.len()),
        _ => format!("jsonType={}", value_type_name(value)),
    }
}

fn transcription_error_message(value: &Value) -> Option<String> {
    let error = value
        .get("error")
        .or_else(|| value.pointer("/data/error"))?;
    if let Some(text) = error
        .as_str()
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        return Some(text.to_string());
    }
    if let Some(message) = error
        .get("message")
        .or_else(|| error.get("msg"))
        .or_else(|| error.get("detail"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        return Some(message.to_string());
    }
    Some(error.to_string())
}

fn value_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// 转写配置的来源标记 —— 用来说明「这次转写到底用了谁的 key」。
///
/// 401 时用户最需要知道的就是这个：请求打到了哪个服务、用的谁的凭据。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TranscriptionSettingsSource {
    /// endpoint 来自哪里
    pub endpoint_from: String,
    /// api_key 来自哪里
    pub key_from: String,
    /// endpoint 是否来自用户的**专属配置**（而非 AI 路由/默认源回退）。
    /// 决定「是不是官方服务」该看 endpoint 还是看路由指针。
    pub endpoint_from_scope_config: bool,
    /// 人类可读的一句话
    pub summary: String,
}

pub(crate) fn resolve_transcription_settings(
    settings: &Value,
) -> Option<(String, Option<String>, String)> {
    resolve_transcription_settings_with_source(settings)
        .map(|(endpoint, api_key, model, _)| (endpoint, api_key, model))
}

/// 同 `resolve_transcription_settings`，但额外返回「配置来源」。
pub(crate) fn resolve_transcription_settings_with_source(
    settings: &Value,
) -> Option<(String, Option<String>, String, TranscriptionSettingsSource)> {
    let flat_endpoint = payload_string(settings, "transcription_endpoint");
    let flat_key = payload_string(settings, "transcription_key");
    let flat_model = payload_string(settings, "transcription_model");
    // 转写 scope 是否被**显式**路由（route 里指定了 sourceId 或 mode）。
    let route_explicit = transcription_route_is_explicit(settings);
    let resolved = crate::ai_model_manager::AiModelManager::resolve(
        settings,
        crate::ai_model_manager::AiModelScope::Transcription,
        None,
    );
    // 取值优先级：**专属配置 > 显式路由 > 默认源回退**。
    //
    // 关键认知：`transcription_endpoint` / `transcription_key` 存的是**前端算出的
    // 最终生效值**（保存设置时由「用户为该 scope 选中的源」或「用户手填的值」决定），
    // 而 `ai_model_routes_json.transcription.sourceId` 只是**来源指针**。
    //
    // 指针有个陷阱：用户在「语音转写」里**手填**了地址（没为该 scope 选源）时，
    // 前端 `resolveLinkedSourceIdFromList` 匹配不上任何 AI 源 → 回退填上**默认源**
    // （实测是官方源）→ 保存时把用户手填的地址/key 整个覆盖成官方源的值。
    // 于是请求打到官方网关（api.ziz.hk/beav/v1），用官方 access token ——
    // 官方 token 一过期就是 401 "Invalid access token"，而用户明明填的是 ElevenLabs。
    //
    // 所以以「专属配置」为准（它是最终值），路由只在它缺失时兜底。
    //
    // 另外：**「默认源回退」不采用官方源**。
    // 转写是唯一会自己上传音频的服务，回退到官方源意味着「用户没配转写」
    // 会被悄悄变成「用官方 access token 调官方 ASR」——官方 token 一过期就是
    // 401，而错误提示只说「检查 transcription key」，用户完全看不出来。
    // 与其静默走官方，不如明确报错，让他去配自己的 STT（如 ElevenLabs）。
    let resolved_is_official = resolved
        .as_ref()
        .map(|route| route.is_official)
        .unwrap_or(false);
    let (endpoint, endpoint_from, endpoint_from_scope_config) = match flat_endpoint.clone() {
        Some(value) if !value.trim().is_empty() => (value, "专属配置", true),
        _ if route_explicit => (
            resolved
                .as_ref()
                .map(|route| route.base_url.clone())
                .unwrap_or_default(),
            "AI 路由",
            false,
        ),
        _ if resolved_is_official => (
            String::new(),
            "未配置（官方源不用于转写）",
            false,
        ),
        _ => (
            resolved
                .as_ref()
                .map(|route| route.base_url.clone())
                .unwrap_or_default(),
            "AI 源（默认回退）",
            false,
        ),
    };
    let endpoint = endpoint.trim().to_string();
    let endpoint = if endpoint.is_empty() {
        payload_string(settings, "api_endpoint").unwrap_or_default()
    } else {
        endpoint
    };
    if endpoint.trim().is_empty() {
        return None;
    }
    let model_name = match flat_model.clone() {
        Some(value) if !value.trim().is_empty() => value,
        _ => resolved
            .as_ref()
            .map(|route| route.model_name.clone())
            .filter(|value| !value.trim().is_empty())?,
    };
    let (api_key, key_from) = match flat_key.clone() {
        Some(value) if !value.trim().is_empty() => (Some(value), "专属配置"),
        _ if route_explicit => (
            resolved.as_ref().and_then(|route| route.api_key.clone()),
            "AI 路由",
        ),
        _ => (
            resolved.as_ref().and_then(|route| route.api_key.clone()),
            "AI 源（默认回退）",
        ),
    };
    let api_key = api_key.or_else(|| payload_string(settings, "api_key"));
    let endpoint_from = endpoint_from.to_string();
    let key_from = key_from.to_string();
    let source = TranscriptionSettingsSource {
        summary: format!("endpoint 来自{endpoint_from}，key 来自{key_from}"),
        endpoint_from,
        key_from,
        endpoint_from_scope_config,
    };
    Some((endpoint, api_key, model_name, source))
}

/// transcription scope 是否被**显式**路由（`ai_model_routes_json.transcription`
/// 里指定了 sourceId 或 mode）。
///
/// 「disabled」不算显式路由 —— 那表示用户主动关掉了这个 scope 的路由，
/// 此时应该回落到 scope 专属配置（transcription_endpoint / transcription_key）。
fn transcription_route_is_explicit(settings: &Value) -> bool {
    let scope = crate::ai_model_manager::AiModelScope::Transcription.as_str();
    // 注意：`ai_model_routes_json` 存的是 **JSON 字符串**，不是对象 ——
    // 直接 .get() 永远拿不到路由，会把「有显式路由」误判成「没有」。
    let routes = payload_string(settings, "ai_model_routes_json")
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| Value::Object(Default::default()));
    let Some(route) = routes.get(scope) else {
        return false;
    };
    let has_source = route
        .get("sourceId")
        .or_else(|| route.get("source_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    let mode = route
        .get("mode")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    // 只有**真的指向某个源**才算显式路由：
    // - 有 sourceId → 明确指向
    // - mode==='official' → 明确要用官方
    // mode==='custom' 但 sourceId 为空 = 什么都没指向（前端在用户没为该 scope
    // 选源、又不允许回退时写的就是这个），不算显式。
    has_source || mode == "official"
}

const YTDLP_DISABLED_MESSAGE: &str = "内置 yt-dlp 服务已移除。";

fn is_probable_youtube_video_id(value: &str) -> bool {
    value.len() == 11
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

pub(crate) fn detect_ytdlp() -> Option<(String, String)> {
    None
}

pub(crate) fn fetch_ytdlp_channel_info(channel_url: &str, limit: i64) -> Result<Value, String> {
    let _ = (channel_url, limit);
    Err(YTDLP_DISABLED_MESSAGE.to_string())
}

pub(crate) fn parse_ytdlp_videos(
    advisor_id: &str,
    channel_id: Option<&str>,
    value: &Value,
) -> Vec<AdvisorVideoRecord> {
    value
        .get("entries")
        .and_then(|item| item.as_array())
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|entry| {
            let id = entry
                .get("id")
                .and_then(|item| item.as_str())
                .map(|item| item.trim().to_string())
                .filter(|item| !item.is_empty())?;
            if !is_probable_youtube_video_id(&id) {
                return None;
            }
            let title = entry
                .get("title")
                .and_then(|item| item.as_str())
                .map(|item| item.to_string())
                .unwrap_or_else(|| format!("Video {}", id));
            let published_at = entry
                .get("release_timestamp")
                .or_else(|| entry.get("timestamp"))
                .and_then(|item| item.as_i64())
                .map(|item| item.to_string())
                .or_else(|| {
                    entry
                        .get("upload_date")
                        .and_then(|item| item.as_str())
                        .map(|item| item.to_string())
                })
                .unwrap_or_else(now_iso);
            let video_url = entry
                .get("url")
                .or_else(|| entry.get("webpage_url"))
                .and_then(|item| item.as_str())
                .map(|item| item.to_string())
                .filter(|item| item.starts_with("http"))
                .unwrap_or_else(|| format!("https://www.youtube.com/watch?v={id}"));
            Some(AdvisorVideoRecord {
                id,
                advisor_id: advisor_id.to_string(),
                title,
                published_at,
                status: "pending".to_string(),
                retry_count: 0,
                error_message: None,
                subtitle_file: None,
                video_url: Some(video_url),
                channel_id: channel_id.map(|item| item.to_string()),
                created_at: now_iso(),
                updated_at: now_iso(),
            })
        })
        .collect()
}

pub(crate) fn download_ytdlp_subtitle(
    video_url: &str,
    target_dir: &Path,
    file_prefix: &str,
) -> Result<PathBuf, String> {
    let _ = (video_url, target_dir, file_prefix);
    Err(YTDLP_DISABLED_MESSAGE.to_string())
}

pub(crate) fn copy_image_to_clipboard(path: &Path) -> Result<(), String> {
    if let Ok(reader) = ImageReader::open(path) {
        if let Ok(decoded) = reader.decode() {
            let rgba = decoded.to_rgba8();
            let width = usize::try_from(rgba.width())
                .map_err(|_| "image width is too large for clipboard".to_string())?;
            let height = usize::try_from(rgba.height())
                .map_err(|_| "image height is too large for clipboard".to_string())?;
            Clipboard::new()
                .and_then(|mut clipboard| {
                    clipboard.set_image(ImageData {
                        width,
                        height,
                        bytes: Cow::Owned(rgba.into_raw()),
                    })
                })
                .map_err(|error| error.to_string())?;
            return Ok(());
        }
    }

    let ext = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_lowercase();
    let image_class = match ext.as_str() {
        "png" => Some("PNG picture"),
        "jpg" | "jpeg" => Some("JPEG picture"),
        "gif" => Some("GIF picture"),
        _ => None,
    };
    if let Some(image_class) = image_class {
        let script = format!(
            "set the clipboard to (read (POSIX file {}) as {})",
            format!("{:?}", path.display().to_string()),
            image_class
        );
        let mut command = background_command("osascript");
        let output = command
            .arg("-e")
            .arg(script)
            .output()
            .map_err(|error| error.to_string())?;
        if output.status.success() {
            return Ok(());
        }
    }
    Err(format!(
        "无法将图片复制到剪贴板：暂不支持该图片格式 ({})",
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or("unknown")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// ElevenLabs(scribe) 返回词级时间戳 `words[]`：必须渲染成带真实时间戳的字幕，
    /// 而不是退化成按字数估算（否则字幕与画面对不齐）。
    #[test]
    fn renders_srt_from_elevenlabs_words() {
        let value = json!({
            "text": "你好世界",
            "words": [
                { "text": "你好", "start": 0.0, "end": 0.5, "type": "word" },
                { "text": "世界", "start": 0.5, "end": 1.2, "type": "word" }
            ]
        });

        let srt = extract_transcription_text(&value, "srt").expect("应渲染出字幕");
        assert!(
            srt.contains("00:00:00,000 --> 00:00:01,200"),
            "应使用 words 里的真实时间戳，实际：\n{srt}"
        );
        assert!(srt.contains("你好世界"), "字幕文本应完整：\n{srt}");
    }

    /// 词级渲染失败时（如 words 结构异常）不应报错，应回退到纯文本。
    #[test]
    fn falls_back_to_text_when_words_are_unusable() {
        let value = json!({
            "text": "只有纯文本",
            "words": [{ "text": "只有纯文本" }]
        });

        assert_eq!(
            extract_transcription_text(&value, "text").as_deref(),
            Some("只有纯文本")
        );
    }

    #[test]
    fn extracts_plain_text_from_provider_json() {
        let value = json!({ "text": "  hello world  " });

        assert_eq!(
            extract_transcription_text(&value, "json").as_deref(),
            Some("hello world")
        );
    }

    #[test]
    fn renders_srt_from_segment_json() {
        let value = json!({
            "segments": [
                { "start": 0.0, "end": 1.5, "text": "第一句" },
                { "start": 1.5, "end": 3.0, "text": "第二句" }
            ]
        });
        let rendered = extract_transcription_text(&value, "srt").expect("srt");

        assert!(rendered.contains("1\n00:00:00,000 --> 00:00:01,500\n第一句"));
        assert!(rendered.contains("2\n00:00:01,500 --> 00:00:03,000\n第二句"));
    }

    #[test]
    fn rejects_plain_text_when_srt_was_requested() {
        let error = parse_transcription_response("hello world", "srt").expect_err("plain text");

        assert!(error.contains("不是 SRT 字幕"));
    }

    #[test]
    fn rejects_gateway_error_text_before_format_fallback() {
        let error = parse_transcription_response("Bad Gateway", "text").expect_err("gateway");

        assert!(error.contains("上游错误"));
    }

    #[test]
    fn parses_curl_http_status_marker() {
        let (body, status) = split_curl_http_status(
            "Bad Gateway\n__redbox_http_status__:502",
            "\n__redbox_http_status__:",
        )
        .expect("status marker");

        assert_eq!(body, "Bad Gateway");
        assert_eq!(status, 502);
    }

    /// 回归守卫：scope 专属配置（transcription_endpoint / transcription_key）
    /// **不能被「默认源回退」静默遮蔽**。
    ///
    /// 实测踩坑：用户在「语音转写」里手填了 ElevenLabs 的地址与 key，但前端保存时
    /// 把「回退到默认源（官方源）」的值写了进去 —— 请求因此打到官方网关
    /// api.ziz.hk/beav/v1、用官方 access token，官方 token 一过期就是
    /// 401 "Invalid access token"，与「STT key 无效」的提示完全对不上号。
    #[test]
    fn transcription_settings_prefer_scope_config_over_default_source() {
        let settings = json!({
            "transcription_endpoint": "https://api.elevenlabs.io/v1/speech-to-text",
            "transcription_key": "sk-elevenlabs",
            "transcription_model": "scribe_v1",
            "default_ai_source_id": "src-official",
            "ai_sources_json": "[{\"id\":\"src-official\",\"baseURL\":\"https://api.ziz.hk/beav/v1\",\"apiKey\":\"official-token\",\"model\":\"gpt-x\"}]",
            "ai_model_routes_json": "{}",
        });
        let (endpoint, key, model, source) =
            resolve_transcription_settings_with_source(&settings).expect("settings");
        assert_eq!(endpoint, "https://api.elevenlabs.io/v1/speech-to-text");
        assert_eq!(key.as_deref(), Some("sk-elevenlabs"));
        assert_eq!(model, "scribe_v1");
        assert_eq!(source.key_from, "专属配置");
    }

    /// 专属配置缺失时，才回退到 AI 路由 / 默认源。
    #[test]
    fn transcription_settings_fall_back_to_route_source() {
        let settings = json!({
            "transcription_model": "whisper-1",
            "default_ai_source_id": "src-custom",
            "ai_sources_json": "[{\"id\":\"src-custom\",\"baseURL\":\"https://custom.example.com\",\"apiKey\":\"sk-custom\",\"model\":\"m-custom\"}]",
            "ai_model_routes_json": "{\"transcription\":{\"sourceId\":\"src-custom\"}}",
        });
        let (endpoint, key, _, source) =
            resolve_transcription_settings_with_source(&settings).expect("settings");
        assert_eq!(endpoint, "https://custom.example.com");
        assert_eq!(key.as_deref(), Some("sk-custom"));
        assert_eq!(source.endpoint_from, "AI 路由");
    }

    /// 回归守卫：转写路由是 `custom` 但**没有 sourceId** 时，不算显式路由 ——
    /// 什么都没指向，应落回用户的专属配置（ElevenLabs）。
    #[test]
    fn transcription_route_without_source_is_not_explicit() {
        let settings = json!({
            "transcription_endpoint": "https://api.elevenlabs.io/v1/speech-to-text",
            "transcription_key": "sk-elevenlabs",
            "transcription_model": "scribe_v2",
            "default_ai_source_id": "src-official",
            "ai_sources_json": "[{\"id\":\"src-official\",\"baseURL\":\"https://api.ziz.hk/beav/v1\",\"apiKey\":\"official-token\",\"model\":\"fun-asr\"}]",
            "ai_model_routes_json": "{\"transcription\":{\"mode\":\"custom\",\"sourceId\":\"\"}}",
        });
        let (endpoint, key, model, source) =
            resolve_transcription_settings_with_source(&settings).expect("settings");
        assert_eq!(endpoint, "https://api.elevenlabs.io/v1/speech-to-text");
        assert_eq!(key.as_deref(), Some("sk-elevenlabs"));
        assert_eq!(model, "scribe_v2");
        assert!(source.endpoint_from_scope_config);
    }

    /// 回归守卫：转写**不采用官方源兜底**。
    ///
    /// 转写是唯一会上传音频的服务；「用户没配转写」被悄悄变成「用官方
    /// access token 调官方 ASR」后，官方 token 一过期就是 401，而错误提示只说
    /// 「检查 transcription key」，用户完全看不出来。
    #[test]
    fn transcription_settings_never_fall_back_to_official_source() {
        let settings = json!({
            "default_ai_source_id": "src-official",
            "ai_sources_json": "[{\"id\":\"src-official\",\"baseURL\":\"https://api.ziz.hk/beav/v1\",\"apiKey\":\"official-token\",\"model\":\"fun-asr\",\"isOfficial\":true}]",
            "ai_model_routes_json": "{}",
        });
        assert!(
            resolve_transcription_settings_with_source(&settings).is_none(),
            "转写不应回退到官方源"
        );
    }

    #[test]
    fn rejects_text_field_when_srt_was_requested() {
        let value = json!({ "text": "hello world" });
        let error = parse_transcription_response(&value.to_string(), "srt").expect_err("text");

        assert!(error.contains("未在响应中找到可用转写字段"));
    }

    #[test]
    fn reports_provider_error_message() {
        let value = json!({ "error": { "message": "unsupported response_format" } });
        let error = parse_transcription_response(&value.to_string(), "srt").expect_err("provider");

        assert!(error.contains("unsupported response_format"));
    }

    #[test]
    fn skips_segments_without_timing_for_srt() {
        let value = json!({
            "segments": [
                { "text": "第一句" }
            ]
        });
        let error = parse_transcription_response(&value.to_string(), "srt").expect_err("timing");

        assert!(error.contains("不是 SRT 字幕"));
    }

    #[test]
    fn reports_shape_when_transcription_fields_are_missing() {
        let value = json!({ "segments": [], "request_id": "abc" });
        let shape = transcription_response_shape(&value);

        assert!(shape.contains("topLevelKeys=[request_id, segments]"));
        assert!(shape.contains("segmentCount=0"));
    }
}
