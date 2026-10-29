use std::fs;
use std::path::Path;

pub(super) fn wav_duration_seconds(path: &Path) -> Option<f64> {
    let metadata = fs::metadata(path).ok()?;
    let bytes = metadata.len().saturating_sub(44);
    if bytes == 0 {
        return None;
    }
    Some(bytes as f64 / 32_000.0)
}

fn split_transcript_cues(text: &str) -> Vec<String> {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return Vec::new();
    }
    let mut cues = Vec::new();
    let mut current = String::new();
    for ch in normalized.chars() {
        current.push(ch);
        let punctuation = matches!(ch, '。' | '！' | '？' | '.' | '!' | '?' | ';' | '；');
        if current.chars().count() >= 32 || (punctuation && current.chars().count() >= 12) {
            cues.push(current.trim().to_string());
            current.clear();
        }
    }
    if !current.trim().is_empty() {
        cues.push(current.trim().to_string());
    }
    cues
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

pub(super) fn render_estimated_subtitles(
    text: &str,
    duration_seconds: f64,
    format: &str,
) -> Result<String, String> {
    reject_invalid_estimated_subtitle_text(text)?;
    let cues = split_transcript_cues(text);
    if cues.is_empty() {
        return Err("转写接口只返回了空文本，无法生成字幕".to_string());
    }
    let safe_duration = duration_seconds.max(cues.len() as f64 * 1.2);
    let cue_duration = safe_duration / cues.len() as f64;
    let mut output = String::new();
    if format == "vtt" {
        output.push_str("WEBVTT\n\n");
    }
    for (index, cue) in cues.iter().enumerate() {
        let start = index as f64 * cue_duration;
        let end = if index + 1 == cues.len() {
            safe_duration
        } else {
            (index + 1) as f64 * cue_duration
        };
        if format == "srt" {
            output.push_str(&format!(
                "{}\n{} --> {}\n{}\n\n",
                index + 1,
                format_srt_time(start),
                format_srt_time(end.max(start + 0.5)),
                cue
            ));
        } else {
            output.push_str(&format!(
                "{} --> {}\n{}\n\n",
                format_vtt_time(start),
                format_vtt_time(end.max(start + 0.5)),
                cue
            ));
        }
    }
    Ok(output)
}

/// 用**真实的词级时间戳**渲染 SRT/VTT（ElevenLabs `words[]` 提供 start/end）。
///
/// 与 `render_estimated_subtitles`（按字数均分时间）的区别：这里每个 cue 的起止都来自 ASR，
/// 字幕与画面完全对齐。切句规则：
/// - 遇到中英文句末/句中标点（。！？；，、.!?;）断句
/// - 或单句累计超过 18 字符（画面上一行放得下）
/// - 或单句时长超过 6 秒（避免一句长期停留）
///
/// `words` 里的 `type` 可能是 `word` / `spacing` / `audio_event`，只有 `word` 才作为新的起点，
/// 其余（空格、笑声标记）直接拼进当前句。返回 `None` 表示拿不到可用时间戳，调用方应回退估算。
/// 整段只有标点（如断句残留下来的单个「。」），不应作为独立字幕。
fn is_punctuation_only(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|c| "。！？；，、.!?;,、".contains(c))
}

pub(crate) fn render_subtitles_from_words(
    words: &[serde_json::Value],
    format: &str,
) -> Option<String> {
    const MAX_CUE_CHARS: usize = 18;
    const MAX_CUE_SECONDS: f64 = 6.0;

    struct Cue {
        start: f64,
        end: f64,
        text: String,
    }

    let mut cues: Vec<Cue> = Vec::new();
    let mut current = Cue {
        start: 0.0,
        end: 0.0,
        text: String::new(),
    };
    let mut started = false;

    for word in words {
        let kind = word.get("type").and_then(|v| v.as_str()).unwrap_or("word");
        let text = word.get("text").and_then(|v| v.as_str()).unwrap_or("");
        if text.is_empty() {
            continue;
        }
        if kind == "word" {
            if !started {
                current.start = word.get("start").and_then(|v| v.as_f64()).unwrap_or(0.0);
                started = true;
            }
            if let Some(end) = word.get("end").and_then(|v| v.as_f64()) {
                current.end = end;
            }
        }
        current.text.push_str(text);

        let char_count = current.text.chars().count();
        let duration = current.end - current.start;
        // 只在**句末**标点断句。中文口语里逗号极密（「哇，不错，很有…」），
        // 按逗号切会把字幕碎成两三个字一条，反而影响阅读；逗号留给长度/时长规则去兜。
        let hit_sentence_end = ["。", "！", "？", "；", ".", "!", "?", ";"]
            .iter()
            .any(|p| text.ends_with(p));
        if hit_sentence_end || char_count >= MAX_CUE_CHARS || duration >= MAX_CUE_SECONDS {
            let trimmed = current.text.trim().to_string();
            if !trimmed.is_empty() {
                // 断句可能恰好发生在句号前一个字（例如时长/长度先触发），
                // 于是「。」被孤立成一条只有标点的字幕。这类残片并入上一条。
                if is_punctuation_only(&trimmed) {
                    if let Some(last) = cues.last_mut() {
                        last.text.push_str(&trimmed);
                        last.end = last.end.max(current.end);
                    }
                } else {
                    cues.push(Cue {
                        start: current.start,
                        end: current.end,
                        text: trimmed,
                    });
                }
            }
            current = Cue {
                start: 0.0,
                end: 0.0,
                text: String::new(),
            };
            started = false;
        }
    }

    let tail = current.text.trim().to_string();
    if !tail.is_empty() {
        cues.push(Cue {
            start: current.start,
            end: (current.end).max(current.start + 0.5),
            text: tail,
        });
    }
    if cues.is_empty() {
        return None;
    }

    // 时间轴单调化：上游偶尔给出重叠时间戳，字幕里必须保证后一条不早于前一条结束
    for index in 1..cues.len() {
        if cues[index].start < cues[index - 1].end {
            cues[index].start = cues[index - 1].end;
        }
        if cues[index].end <= cues[index].start {
            cues[index].end = cues[index].start + 0.5;
        }
    }

    let mut output = String::new();
    if format == "vtt" {
        output.push_str("WEBVTT\n\n");
    }
    for (index, cue) in cues.iter().enumerate() {
        let (start, end) = if format == "vtt" {
            (format_vtt_time(cue.start), format_vtt_time(cue.end))
        } else {
            (format_srt_time(cue.start), format_srt_time(cue.end))
        };
        if format == "vtt" {
            output.push_str(&format!("{start} --> {end}\n{}\n\n", cue.text));
        } else {
            output.push_str(&format!("{}\n{start} --> {end}\n{}\n\n", index + 1, cue.text));
        }
    }
    Some(output.trim_end().to_string() + "\n")
}

fn reject_invalid_estimated_subtitle_text(text: &str) -> Result<(), String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("转写接口只返回了空文本，无法生成字幕".to_string());
    }
    let normalized = trimmed.to_ascii_lowercase();
    let is_provider_error = matches!(
        normalized.as_str(),
        "bad gateway" | "gateway timeout" | "service unavailable" | "upstream timeout"
    ) || normalized.contains("502 bad gateway")
        || normalized.contains("503 service unavailable")
        || normalized.contains("504 gateway timeout")
        || normalized.contains("upstream request timeout");
    if is_provider_error {
        return Err(format!("转写接口上游错误：{trimmed}"));
    }
    if trimmed.chars().count() < 2 {
        return Err("转写接口返回内容过短，无法生成字幕".to_string());
    }
    Ok(())
}
