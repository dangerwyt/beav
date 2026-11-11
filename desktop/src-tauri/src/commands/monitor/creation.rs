use super::*;

/// 从模型输出里稳健抽取 JSON 对象（兼容 ```json 围栏与前后多余文本）。
pub(super) fn extract_json_object(text: &str) -> Option<Value> {
    let trimmed = text.trim();
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        if value.is_object() {
            return Some(value);
        }
    }
    let body = strip_code_fence(trimmed);
    let start = body.find('{')?;
    let end = body.rfind('}')?;
    if start <= end {
        if let Ok(value) = serde_json::from_str::<Value>(&body[start..=end]) {
            if value.is_object() {
                return Some(value);
            }
        }
    }
    None
}

/// 去掉 markdown 代码围栏（```json ... ``` 或 ``` ... ```）。
pub(super) fn strip_code_fence(s: &str) -> &str {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix("```") {
        let rest = if let Some(idx) = rest.find('\n') {
            &rest[idx + 1..]
        } else {
            rest
        };
        if let Some(end) = rest.rfind("```") {
            return rest[..end].trim();
        }
        return rest.trim();
    }
    s
}

/// 从 JSON 值里取字符串数组字段（缺省 / 类型不符返回空 Vec）。
pub(super) fn string_array_field(value: &Value, key: &str) -> Option<Vec<String>> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|array| {
            array
                .iter()
                .filter_map(|item| item.as_str().map(String::from))
                .collect()
        })
}

/// 爆文拆解：把一篇对标帖的文案交给 LLM 拆成可复用的方法论要素，落库到 `post_deconstructions`。
///
/// 复用 `ai_model_manager` 解析 AI 配置（用户自配或官方）+ `official_support` 的同步结构化调用
/// （按 provider 自动分发 OpenAI/Anthropic/Gemini）。前端已抓取的全文在 `content` 字段，无需再抽文案。
/// 拆解输入的字符预算（按 Unicode 字符计，不是字节）。
pub(super) const DECONSTRUCT_BODY_CLIP: usize = 1500;
pub(super) const DECONSTRUCT_TRANSCRIPT_CLIP: usize = 2000;
/// 口播大纲的抽段步长与上限：一小时视频的 SRT 有几万字，必须压成能塞进上下文的骨架。
pub(super) const DECONSTRUCT_OUTLINE_STEP_SECS: i64 = 90;
pub(super) const DECONSTRUCT_OUTLINE_MAX: usize = 20;

pub(super) fn clip_chars(text: &str, max: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max {
        return trimmed.to_string();
    }
    let mut out: String = trimmed.chars().take(max).collect();
    out.push('…');
    out
}

pub(super) fn format_hms(total: i64) -> String {
    let secs = total.max(0);
    format!("{:02}:{:02}:{:02}", secs / 3600, (secs % 3600) / 60, secs % 60)
}

/// 解析 SRT 成 `(起始秒, 文本)`。脏行直接跳过——拆解不该因为字幕格式问题整条失败。
pub(super) fn srt_cues(srt: &str) -> Vec<(i64, String)> {
    let mut cues = Vec::new();
    for block in srt.split("\n\n") {
        let lines: Vec<&str> = block.lines().filter(|line| !line.trim().is_empty()).collect();
        if lines.is_empty() {
            continue;
        }
        let Some(timing_index) = lines.iter().position(|line| line.contains("-->")) else {
            continue;
        };
        let start_raw = lines[timing_index].split("-->").next().unwrap_or("").trim();
        let start = start_raw
            .split(':')
            .collect::<Vec<_>>();
        let start_secs = if start.len() == 3 {
            let secs_part = start[2].split(',').next().unwrap_or("0");
            let h = start[0].trim().parse::<i64>().unwrap_or(0);
            let m = start[1].trim().parse::<i64>().unwrap_or(0);
            let s = secs_part.trim().parse::<i64>().unwrap_or(0);
            h * 3600 + m * 60 + s
        } else {
            continue;
        };
        let text = lines[timing_index + 1..].join(" ").trim().to_string();
        if text.is_empty() {
            continue;
        }
        cues.push((start_secs, text));
    }
    cues
}

/// 把字幕压成「每约 step 秒一行」的大纲，时间码保留真实值。
pub(super) fn srt_outline(cues: &[(i64, String)], step_secs: i64, max_lines: usize) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut next_from = 0i64;
    for (start, text) in cues {
        if lines.len() >= max_lines {
            break;
        }
        if *start < next_from {
            continue;
        }
        lines.push(format!("【{}】{}", format_hms(*start), clip_chars(text, 60)));
        next_from = *start + step_secs.max(1);
    }
    lines.join("\n")
}

/// 拆解用的系统提示：五层框架（钩子—结构—论证—情绪—转化），输出字段与旧版保持一致，
/// 额外给一个结构化 `structure`，供「结构模板库」检索。
/// beav-content-remix 技能的方法论，原样内嵌：拆解/分镜/改写三条链路各自追加。
/// 直接 include_str! 是为了让提示词可追溯（改口径 = 改这几个 md），不引入运行时读文件。
pub(super) const DECONSTRUCT_GUIDE: &str = include_str!("../remix_prompts/deconstruction.md");
pub(super) const STORYBOARD_GUIDE: &str = include_str!("../remix_prompts/storyboard.md");
pub(super) const REWRITE_GUIDE: &str = include_str!("../remix_prompts/rewrite-playbook.md");

pub(super) const DECONSTRUCT_SYSTEM_PROMPT: &str = "你是资深内容拆解专家，负责把已验证的爆款内容拆成可复用的方法论。\
按五层分析，只输出 JSON，不要任何解释性文字：\
1) 钩子：开头用什么把人留住（反常识 / 利益承诺 / 身份代入 / 悬念 / 冲突 / 故事切入），并摘出原句；\
2) 结构：按段落标出各自功能（钩住 → 立论 → 论证 → 转折 → 升华 → 转化），给出每段起始时间码与大致占比；\
3) 论证：用了什么材料（故事 / 数据 / 类比 / 权威），哪里证据偏薄；\
4) 情绪：情绪如何推进，金句出现在什么位置、为什么能被记住；\
5) 转化：结尾做了哪种引导（关注 / 收藏 / 评论 / 私域），是否自然。\
硬性约束：时间码**只能**取自用户给出的大纲里的时间码，禁止编造；大纲没给时间码就留空字符串。\
画面/镜头细节没有依据就不要写。数字未经核实不得写成结论。\
输出字段：topicAngle（选题角度，1-2 句）、copyStructure（结构说明，需带时间码）、\
opening（开场钩子，注明钩子类型）、titlePatterns（标题句式数组）、hooks（可脱离上下文传播的金句数组）、\
reusableTakeaway（可复用的方法论点，1-2 句）、\
structure（数组，每项 {role, start, weight, gist}：role 为段落功能，start 为时间码，weight 为占全长百分比，gist 为该段做了什么）。";

pub(super) fn deconstruct(state: &State<'_, AppState>, payload: &Value) -> Result<Value, String> {
    let post_id = str_field(payload, "postId")
        .or_else(|| str_field(payload, "post_id"))
        .ok_or_else(|| "缺少 postId".to_string())?;

    // 取帖子文案与元数据（只读）
    let post = with_store(state, |store| {
        Ok(store
            .monitored_posts
            .iter()
            .find(|item| item.id == post_id)
            .cloned())
    })?;
    let post = post.ok_or_else(|| "未找到该动态，请先抓取".to_string())?;

    // 转写与字幕：视频帖的「正文」通常只有一句标题，真正的内容在口播里。
    // 只喂正文会让拆解凭空发挥（旧实现的根本问题）。
    let analysis = with_store(state, |store| {
        Ok(store
            .post_video_analyses
            .iter()
            .find(|item| item.post_id == post_id)
            .cloned())
    })?;
    let transcript = analysis
        .as_ref()
        .map(|item| item.transcript.trim().to_string())
        .unwrap_or_default();
    let srt = analysis
        .as_ref()
        .map(|item| item.transcript_srt.clone())
        .unwrap_or_default();
    let duration_secs = analysis.as_ref().and_then(|item| item.duration_secs);

    if post.content.trim().is_empty() && transcript.is_empty() {
        return Err(
            "这条既没有正文也没有转写：请先「重新抓正文」，或对视频跑一次「语音转写」后再拆解"
                .to_string(),
        );
    }

    // 解析 AI 配置（用户自配或官方）
    let settings = with_store(state, |store| Ok(crate::store::settings::settings_snapshot(&store)))?;
    let config = AiModelManager::resolve_chat_config(&settings, None)
        .ok_or_else(|| "未配置可用的 AI 模型，请在设置中配置后重试".to_string())?;

    // 拼输入：元信息 + 文案（截断）+ 口播大纲（有 SRT 用大纲，否则退全文）
    let cues = srt_cues(&srt);
    let outline = srt_outline(&cues, DECONSTRUCT_OUTLINE_STEP_SECS, DECONSTRUCT_OUTLINE_MAX);
    let engagement = {
        let mut parts: Vec<String> = Vec::new();
        if post.likes > 0 {
            parts.push(format!("赞 {}", post.likes));
        }
        if post.comments > 0 {
            parts.push(format!("评 {}", post.comments));
        }
        if post.collects > 0 {
            parts.push(format!("藏 {}", post.collects));
        }
        if post.likes > 0 && post.collects > 0 {
            parts.push(format!("赞藏比 1:{:.1}", post.collects as f64 / post.likes as f64));
        }
        parts.join(" · ")
    };

    let mut user_prompt = String::new();
    user_prompt.push_str(&format!(
        "【平台】{}\n【作者】{}\n",
        platform_label(&post.platform),
        post.author
    ));
    if let Some(duration) = duration_secs {
        user_prompt.push_str(&format!("【时长】{}\n", format_hms(duration as i64)));
    }
    if !engagement.is_empty() {
        user_prompt.push_str(&format!("【互动】{}\n", engagement));
    }
    if !post.title.trim().is_empty() {
        user_prompt.push_str(&format!("【标题】{}\n", post.title));
    }
    if !post.content.trim().is_empty() {
        user_prompt.push_str(&format!(
            "\n【文案（可能被截断）】\n{}\n",
            clip_chars(&post.content, DECONSTRUCT_BODY_CLIP)
        ));
    }
    if !outline.is_empty() {
        user_prompt.push_str(&format!(
            "\n【口播分段大纲（时间码取自真实字幕，只能引用这里出现的时间码）】\n{}\n",
            outline
        ));
    } else if !transcript.is_empty() {
        user_prompt.push_str(&format!(
            "\n【口播全文（无时间码，不要编造时间码）】\n{}\n",
            clip_chars(&transcript, DECONSTRUCT_TRANSCRIPT_CLIP)
        ));
    }

    let system_prompt = format!(
        "{DECONSTRUCT_SYSTEM_PROMPT}\n\n# 拆解方法论（必须遵守）\n{DECONSTRUCT_GUIDE}"
    );

    let raw = invoke_structured_chat_by_protocol(
        &config.protocol,
        &config.base_url,
        config.api_key.as_deref(),
        &config.model_name,
        &system_prompt,
        &user_prompt,
        true,
    )?;

    let parsed = extract_json_object(&raw)
        .ok_or_else(|| format!("模型未返回有效 JSON：{}", raw.chars().take(240).collect::<String>()))?;

    let topic_angle = str_field(&parsed, "topicAngle")
        .or_else(|| str_field(&parsed, "topic_angle"))
        .unwrap_or_default();
    let copy_structure = str_field(&parsed, "copyStructure")
        .or_else(|| str_field(&parsed, "copy_structure"))
        .unwrap_or_default();
    let opening = str_field(&parsed, "opening").unwrap_or_default();
    let title_patterns = string_array_field(&parsed, "titlePatterns")
        .or_else(|| string_array_field(&parsed, "title_patterns"))
        .unwrap_or_default();
    let hooks = string_array_field(&parsed, "hooks").unwrap_or_default();
    let reusable_takeaway = str_field(&parsed, "reusableTakeaway")
        .or_else(|| str_field(&parsed, "reusable_takeaway"))
        .unwrap_or_default();

    // 结构骨架：模型可给可不给（图文帖通常没有），给多少收多少，不因缺字段失败。
    let structure: Vec<DeconstructionSegment> = parsed
        .get("structure")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let role = str_field(item, "role").unwrap_or_default();
                    let gist = str_field(item, "gist").unwrap_or_default();
                    if role.trim().is_empty() && gist.trim().is_empty() {
                        return None;
                    }
                    Some(DeconstructionSegment {
                        role,
                        start: str_field(item, "start").unwrap_or_default(),
                        weight: item.get("weight").and_then(Value::as_f64),
                        gist,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    // 拆解依据：写清楚用了哪些输入，日后复盘才能区分「基于口播」与「只看了文案」。
    let basis = if !cues.is_empty() {
        format!(
            "含口播时间轴 {} 段 / 大纲 {} 行",
            cues.len(),
            outline.lines().count()
        )
    } else if !transcript.is_empty() {
        format!("含口播全文 {} 字（无时间码）", transcript.chars().count())
    } else {
        "仅文案，未含口播".to_string()
    };
    let source_evidence = format!(
        "对标监控 · {} · {}（{}）",
        platform_label(&post.platform),
        post.author,
        basis
    );

    let now = now_i64();
    let mut record = crate::PostDeconstructionRecord {
        id: make_id("decon"),
        post_id: post.id.clone(),
        platform: post.platform.clone(),
        author: post.author.clone(),
        post_title: post.title.clone(),
        post_url: post.post_url.clone(),
        topic_angle,
        copy_structure,
        opening,
        title_patterns,
        hooks,
        reusable_takeaway,
        structure,
        raw_content: post.content.clone(),
        source_evidence,
        created_at: now,
        updated_at: now,
    };
    let record_value = serde_json::to_value(&record).unwrap_or_else(|_| json!({}));

    with_store_mut(state, |store| {
        if let Some(pos) = store
            .post_deconstructions
            .iter()
            .position(|r| r.post_id == post.id)
        {
            // 已拆过：保留原 id / createdAt，覆盖其余字段
            record.id = store.post_deconstructions[pos].id.clone();
            record.created_at = store.post_deconstructions[pos].created_at;
            store.post_deconstructions[pos] = record.clone();
        } else {
            store.post_deconstructions.push(record.clone());
        }
        if let Some(pos) = store.monitored_posts.iter().position(|p| p.id == post.id) {
            store.monitored_posts[pos].deconstructed = true;
            store.monitored_posts[pos].updated_at = now.to_string();
        }
        Ok(json!({ "success": true }))
    })?;

    // 拆解完成后自动导出「帖子最终产物」到知识库目录树
    export_post_product(state, &post.id);

    Ok(json!({ "success": true, "deconstruction": record_value }))
}

/// 列出已沉淀的爆文拆解（方法论库），可按 author / platform 过滤。
pub(super) fn decon_list(state: &State<'_, AppState>, payload: &Value) -> Result<Value, String> {
    let author = str_field(payload, "author").unwrap_or_default();
    let platform = str_field(payload, "platform").unwrap_or_default();
    with_store(state, |store| {
        let mut items: Vec<Value> = store
            .post_deconstructions
            .iter()
            .filter(|item| crate::identity::is_admin() || store.monitored_posts.iter().any(|post| post.id == item.post_id && post_accessible(&store, post)))
            .filter(|r| author.is_empty() || r.author == author)
            .filter(|r| platform.is_empty() || r.platform == platform)
            .map(|r| serde_json::to_value(r).unwrap_or_else(|_| json!({})))
            .collect();
        // 新沉淀的排前面
        items.sort_by(|a, b| {
            let at = b.get("createdAt").and_then(Value::as_i64).unwrap_or(0);
            let bt = a.get("createdAt").and_then(Value::as_i64).unwrap_or(0);
            at.cmp(&bt)
        });
        let authors: Vec<String> = store
            .post_deconstructions
            .iter()
            .filter(|item| crate::identity::is_admin() || store.monitored_posts.iter().any(|post| post.id == item.post_id && post_accessible(&store, post)))
            .map(|r| r.author.clone())
            .filter(|v| !v.is_empty())
            .collect();
        let mut uniq: Vec<String> = Vec::new();
        for a in authors {
            if !uniq.contains(&a) {
                uniq.push(a);
            }
        }
        Ok(json!({ "success": true, "items": items, "authors": uniq }))
    })
}

// —— 二创产出：分镜脚本 / 多平台改写 ——

/// 目标平台的写作规范（改写用）。集中一处，避免 prompt 里散落平台细节。
pub(super) fn remix_platform_guide(target: &str) -> (&'static str, &'static str) {
    match target {
        "xiaohongshu" => (
            "小红书",
            "标题 ≤20 字、口语化、可带 emoji 但不超过 2 个；正文分 3-5 段，每段 1-3 行；\
             结尾一句互动引导；话题标签 5-8 个放最后一行。",
        ),
        "wechat" => (
            "公众号",
            "标题 ≤24 字、偏信息密度；开头 2 句必须给出结论；正文用小标题分段；\
             结尾引导在看/转发，不写外链。",
        ),
        "bilibili" => (
            "B站",
            "标题 ≤30 字、可带技能点前缀；简介前 2 行是钩子；正文适合口播稿节奏；\
             标签 5 个左右。",
        ),
        _ => (
            "抖音",
            "标题 ≤25 字、前 8 字必须抓人；正文口语化短句，一行一个意思；\
             话题标签 3-5 个；结尾一句引导评论。",
        ),
    }
}

/// 把模型给的镜头数组渲染成 Markdown 分镜表：时间码由后端按时长累加。
///
/// 为什么不让模型给时间码：它只能在「90 秒一行」的口播大纲里挑，会把同一条拆成
/// 多个镜头、写出重复或跳跃的时间码，还可能把「新片时间线」和「原片定位」混为一谈。
/// 后端累加后时间码天然自洽，原片定位另用抽帧/视频对照。
pub(super) fn render_storyboard_markdown(raw: &str, target_seconds: i64) -> String {
    let Some(parsed) = extract_json_object(raw) else {
        return raw.trim().to_string(); // 模型没给 JSON：原样返回，至少内容不丢
    };
    let shots = parsed
        .get("shots")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if shots.is_empty() {
        return raw.trim().to_string();
    }
    let title = str_field(&parsed, "title").unwrap_or_default();
    let mut total = 0i64;
    let mut table = String::from(
        "| 镜号 | 时间码 | 时长 | 画面设计 | 口播台词 | 屏幕字幕 | 备注 |\n| --- | --- | --- | --- | --- | --- | --- |\n",
    );
    for (index, shot) in shots.iter().enumerate() {
        let duration = shot
            .get("duration")
            .and_then(Value::as_i64)
            .unwrap_or(6)
            .clamp(1, 20);
        let start = total;
        total += duration;
        let cell = |key: &str| -> String {
            str_field(shot, key)
                .unwrap_or_default()
                .replace('|', "/")
                .replace('\n', " ")
        };
        table.push_str(&format!(
            "| {} | {} | {}s | {} | {} | {} | {} |\n",
            index + 1,
            format_hms(start),
            duration,
            cell("framing"),
            cell("line"),
            cell("caption"),
            cell("note")
        ));
    }
    let mut out = format!(
        "> 目标成片约 {target_seconds} 秒 · 实排 {total} 秒 · {n} 个镜头\n> 时间码是**新片时间线**（按镜头时长累加），不是原片定位；原片画面请对照同目录的 `抽帧_*.jpg` 与 `视频.mp4`。\n\n",
        target_seconds = target_seconds,
        total = total,
        n = shots.len()
    );
    if !title.trim().is_empty() {
        out.push_str(&format!("**{}**\n\n", title.trim()));
    }
    out.push_str(&table);
    out
}

/// 生成二创产出（`kind = storyboard | rewrite`）。
///
/// 输入复用拆解那套：元信息 + 文案 + 口播大纲；若已拆解过，额外带上结构骨架与金句，
/// 避免重复分析。产物落 `post_remixes`，导出时写进产物目录的 `二创/`。
pub(super) fn remix_generate(state: &State<'_, AppState>, payload: &Value) -> Result<Value, String> {
    let post_id = str_field(payload, "postId")
        .or_else(|| str_field(payload, "post_id"))
        .ok_or_else(|| "缺少 postId".to_string())?;
    let kind = str_field(payload, "kind").unwrap_or_else(|| "storyboard".to_string());
    let target = str_field(payload, "target").unwrap_or_else(|| "douyin".to_string());
    let target_seconds = payload
        .get("targetSeconds")
        .and_then(Value::as_i64)
        .filter(|seconds| *seconds >= 15 && *seconds <= 900)
        .unwrap_or(60);

    let post = with_store(state, |store| {
        Ok(store
            .monitored_posts
            .iter()
            .find(|item| item.id == post_id)
            .cloned())
    })?
    .ok_or_else(|| "未找到该动态，请先抓取".to_string())?;

    let analysis = with_store(state, |store| {
        Ok(store
            .post_video_analyses
            .iter()
            .find(|item| item.post_id == post_id)
            .cloned())
    })?;
    let transcript = analysis
        .as_ref()
        .map(|item| item.transcript.trim().to_string())
        .unwrap_or_default();
    let srt = analysis
        .as_ref()
        .map(|item| item.transcript_srt.clone())
        .unwrap_or_default();
    let decon = with_store(state, |store| {
        Ok(store
            .post_deconstructions
            .iter()
            .find(|item| item.post_id == post_id)
            .cloned())
    })?;

    if post.content.trim().is_empty() && transcript.is_empty() {
        return Err("这条既没有正文也没有转写，无法二创：请先抓正文或跑语音转写".to_string());
    }

    let settings = with_store(state, |store| Ok(crate::store::settings::settings_snapshot(&store)))?;
    let config = AiModelManager::resolve_chat_config(&settings, None)
        .ok_or_else(|| "未配置可用的 AI 模型，请在设置中配置后重试".to_string())?;

    let cues = srt_cues(&srt);
    let outline = srt_outline(&cues, DECONSTRUCT_OUTLINE_STEP_SECS, DECONSTRUCT_OUTLINE_MAX);

    let mut context = String::new();
    context.push_str(&format!(
        "【原素材】{} · {} · 《{}》\n",
        platform_label(&post.platform),
        post.author,
        post.title
    ));
    context.push_str(&format!("【原帖】{}\n", post.post_url));
    if !post.content.trim().is_empty() {
        context.push_str(&format!(
            "\n【原文案】\n{}\n",
            clip_chars(&post.content, DECONSTRUCT_BODY_CLIP)
        ));
    }
    if !outline.is_empty() {
        context.push_str(&format!("\n【口播大纲（时间码取自真实字幕）】\n{}\n", outline));
    } else if !transcript.is_empty() {
        context.push_str(&format!(
            "\n【口播全文（无时间码）】\n{}\n",
            clip_chars(&transcript, DECONSTRUCT_TRANSCRIPT_CLIP)
        ));
    }
    if let Some(decon) = &decon {
        if !decon.hooks.is_empty() {
            context.push_str(&format!(
                "\n【原帖金句（仅供理解，不得照搬）】\n{}\n",
                decon.hooks
                    .iter()
                    .take(6)
                    .map(|hook| format!("- {hook}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }
    }

    let (system_prompt, user_prompt) = if kind == "rewrite" {
        let (name, guide) = remix_platform_guide(&target);
        (
            format!(
                "你是资深自媒体内容编辑，负责把一条已验证的爆款内容改写成适合{}的版本。\
硬性要求：1) 禁止连续 20 字以上与原口播/原文重合，必须换视角、换结构与表达；\
2) 保留原观点的价值，但补充原文没有的增量（例子、数字、类比）；\
3) 结尾标注一句「观点参考：{{原作者}} · {{平台}} · {{原帖链接}}」；\
4) 只输出 Markdown 正文，不要解释你的思路。\
平台规范：{}\n\n# 改写方法论（必须遵守）\n{REWRITE_GUIDE}",
                name, guide
            ),
            format!(
                "{context}\n\n请输出 Markdown：\n## 标题候选\n（5 条，逐条一行）\n\n## 正文\n（按平台规范分段）\n\n## 话题标签\n（一行）\n"
            ),
        )
    } else {
        (
            format!(
                "你是短视频分镜导演。基于给定素材，写一份可直接执行的分镜脚本，目标时长约 {target_seconds} 秒。\
硬性要求：1) 你排的是**新片**的时间线，时间码由系统按镜头时长累加生成，你不要输出时间码，只给每镜时长（秒）；\
2) 画面列写**设计建议**（景别、拍什么、怎么剪），不是对原片的描述——你没有看到画面，禁止声称画面内容；\
3) 台词可改写，但不得整段照搬原口播；4) 只输出 JSON，不要解释。\n\n# 分镜方法论（必须遵守）\n{STORYBOARD_GUIDE}"
            ),
            format!(
                "{context}\n\n输出 JSON：{{\"title\": 一句话片名, \"shots\": [{{\"duration\": 7, \"framing\": \"画面设计（景别+拍什么）\", \"line\": \"口播台词\", \"caption\": \"屏幕字幕（可空）\", \"note\": \"拍摄/剪辑备注（可空）\"}}]}}。\n镜头总时长控制在 {target_seconds} 秒左右，单镜 3-8 秒。\n"
            ),
        )
    };

    let raw = invoke_structured_chat_by_protocol(
        &config.protocol,
        &config.base_url,
        config.api_key.as_deref(),
        &config.model_name,
        &system_prompt,
        &user_prompt,
        kind == "storyboard",
    )?;
    let content = if kind == "storyboard" {
        render_storyboard_markdown(&raw, target_seconds)
    } else {
        raw.trim().to_string()
    };
    if content.is_empty() {
        return Err("模型返回了空内容，请稍后重试".to_string());
    }

    let basis = if !cues.is_empty() {
        format!("含口播时间轴 {} 段", cues.len())
    } else if !transcript.is_empty() {
        format!("含口播全文 {} 字", transcript.chars().count())
    } else {
        "仅文案".to_string()
    };
    let source_evidence = format!(
        "对标监控 · {} · {}（{}）",
        platform_label(&post.platform),
        post.author,
        basis
    );

    let now = now_i64();
    let mut record = crate::PostRemixRecord {
        id: make_id("remix"),
        post_id: post.id.clone(),
        platform: post.platform.clone(),
        author: post.author.clone(),
        post_title: post.title.clone(),
        kind: kind.clone(),
        target: if kind == "rewrite" { target.clone() } else { String::new() },
        content,
        source_evidence,
        created_at: now,
        updated_at: now,
    };

    let saved = with_store_mut(state, |store| {
        // 同一帖同一类型（改写按平台区分）只留最新一份，避免越生成越多
        if let Some(pos) = store.post_remixes.iter().position(|item| {
            item.post_id == record.post_id && item.kind == record.kind && item.target == record.target
        }) {
            record.id = store.post_remixes[pos].id.clone();
            record.created_at = store.post_remixes[pos].created_at;
            store.post_remixes[pos] = record.clone();
        } else {
            store.post_remixes.push(record.clone());
        }
        Ok(serde_json::to_value(&record).unwrap_or_else(|_| json!({})))
    })?;

    let export_error = try_export_post_product(state, &post.id).err();

    Ok(json!({ "success": true, "remix": saved, "exported": export_error.is_none(), "exportError": export_error }))
}

/// 导出素材包：确保产物已落盘，返回文件清单（打包交给前端调 file:save-zip）。
///
/// 只负责打包，不管对方怎么用：清单里的 name 保留「平台/日期/作者/标题/文件」
/// 相对路径，解压后就是一份完整的素材目录。
pub(super) fn material_files(state: &State<'_, AppState>, payload: &Value) -> Result<Value, String> {
    let ids: Vec<String> = payload
        .get("ids")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    if ids.is_empty() {
        return Err("缺少 ids".to_string());
    }
    let root = resolve_export_target(state)?;
    let mut files: Vec<Value> = Vec::new();
    for id in &ids {
        try_export_post_product(state, &id)?; // 幂等：已有产物只是补齐
        let post = with_store(state, |store| {
            Ok(store.monitored_posts.iter().find(|p| p.id == *id).cloned())
        })?;
        let Some(post) = post else { continue };
        let l1 = root.join(platform_label(&post.platform));
        let Some(dir) = find_export_dir_for_post(&l1, &id) else { continue };
        collect_bundle_files(&dir, &root, &mut files);
    }
    // web 端没有原生保存对话框：直接打成 zip 落到导出根下（/api/file 只放行导出根内的文件），
    // 前端用 GET /api/file?path=... 让浏览器下载。
    let bundle = payload.get("bundle").and_then(Value::as_bool).unwrap_or(false);
    if bundle {
        if files.is_empty() {
            return Err("没有可打包的素材".to_string());
        }
        let name = payload
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("素材包")
            .to_string();
        // Each request gets a separate directory so users cannot overwrite each other's bundle.
        let dir = root.join("_素材包").join(make_id("bundle"));
        std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
        let name = sanitize_fs_name(&name).trim_matches('.').to_string();
        if name.is_empty() { return Err("素材包名称无效".to_string()); }
        let target = dir.join(format!("{name}.zip"));
        let written = crate::commands::file_ops::archive::write_zip_archive(state, &files, target)?;
        std::fs::write(dir.join(".beav-bundle.json"), serde_json::to_vec(&json!({
            "owner": crate::identity::owner_or_admin(), "postIds": ids,
        })).map_err(|error| error.to_string())?).map_err(|error| error.to_string())?;
        return Ok(json!({ "success": true, "count": files.len(), "bundle": written }));
    }
    Ok(json!({ "success": true, "count": files.len(), "files": files }))
}

/// 递归收集产物文件（产物目录只有「二创/」一层子目录，深度给 3 足够）。
pub(super) fn collect_bundle_files(dir: &Path, root: &Path, out: &mut Vec<Value>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if out.len() < 5000 {
                collect_bundle_files(&path, root, out);
            }
            continue;
        }
        let Ok(relative) = path.strip_prefix(root) else { continue };
        out.push(json!({
            "source": path.display().to_string(),
            "name": relative.to_string_lossy().replace('\\', "/"),
        }));
    }
}

/// 列出某帖（或全部）的二创产出。
pub(super) fn remix_list(state: &State<'_, AppState>, payload: &Value) -> Result<Value, String> {
    let post_id = str_field(payload, "postId")
        .or_else(|| str_field(payload, "post_id"))
        .unwrap_or_default();
    with_store(state, |store| {
        let mut items: Vec<Value> = store
            .post_remixes
            .iter()
            .filter(|item| crate::identity::is_admin() || store.monitored_posts.iter().any(|post| post.id == item.post_id && post_accessible(&store, post)))
            .filter(|item| post_id.is_empty() || item.post_id == post_id)
            .map(|item| serde_json::to_value(item).unwrap_or_else(|_| json!({})))
            .collect();
        items.sort_by(|a, b| {
            let at = b.get("updatedAt").and_then(Value::as_i64).unwrap_or(0);
            let bt = a.get("updatedAt").and_then(Value::as_i64).unwrap_or(0);
            at.cmp(&bt)
        });
        Ok(json!({ "success": true, "items": items }))
    })
}
