use super::*;

/// 本地查看单条动态详情。
///
/// 优先读缓存（`content` 非空直接返回，不联网）；`refresh=true` 或缓存为空时，用该平台
/// 已登录的 Chrome（CDP）打开帖子详情页抓正文/图片/互动数据，写回记录（excerpt 取正文前 80 字）。
/// 平台未登录 / 抓取失败都**不报错**，返回原记录 + `note` 说明，前端照常本地展示已有信息。
pub(super) fn post_detail(state: &State<'_, AppState>, payload: &Value) -> Result<Value, String> {
    let id = str_field(payload, "id").ok_or_else(|| "缺少 id".to_string())?;
    let refresh = payload
        .get("refresh")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let record = with_store(state, |store| {
        Ok(store
            .monitored_posts
            .iter()
            .find(|item| item.id == id)
            .cloned())
    })?
    .ok_or_else(|| "动态不存在".to_string())?;
    // 打开详情是只读操作：被分配的博主也要能打开，所以用放宽规则（owner 或 sharedTo）。
    let shared = with_store(state, |store| {
        Ok(record
            .watch_id
            .as_ref()
            .and_then(|watch_id| store.competitor_watchlist.iter().find(|item| &item.id == watch_id))
            .map(|watch| watch.shared_to.clone())
            .unwrap_or_default())
    })?;
    if !crate::identity::can_access(&record.owner, &shared) {
        return Err("动态不存在".to_string());
    }

    let can_cdp = Platform::parse(&record.platform).map(|p| p.is_cdp()).unwrap_or(false);
    if !can_cdp {
        return Ok(json!({
            "success": true, "post": record, "fetched": false,
            "note": "该平台暂不支持本地抓正文，请点「原文」查看",
        }));
    }
    if !refresh && (record.detail_fetched || !record.content.is_empty()) {
        // 存量记录自愈：老数据里的图片还是远程 URL（CDN 校验 Referer → WebView 裂图、导出也复制不到），
        // 打开详情时就地本地化一次，不必等用户点「重新抓正文」。
        // 含 `//host/...` 这种协议相对地址：老数据里常见，同样要本地化
        let has_remote = record
            .images
            .iter()
            .any(|u| u.starts_with("http") || u.starts_with("//"))
            || record
                .cover_url
                .as_deref()
                .map(|u| u.starts_with("http") || u.starts_with("//"))
                .unwrap_or(false);
        if has_remote {
            let (imgs, cover) = cache_post_images(
                state,
                &record.id,
                &record.platform,
                &record.images,
                record.cover_url.as_deref(),
            );
            let mut healed = record.clone();
            healed.images = imgs;
            if cover.is_some() {
                healed.cover_url = cover;
            }
            let _ = with_store_mut(state, |store| {
                if let Some(slot) = store.monitored_posts.iter_mut().find(|p| p.id == healed.id) {
                    slot.images = healed.images.clone();
                    slot.cover_url = healed.cover_url.clone();
                }
                Ok(())
            });
            return Ok(json!({
                "success": true, "post": healed,
                "fetched": !healed.content.is_empty(), "note": "",
            }));
        }
        return Ok(json!({ "success": true, "post": record, "fetched": !record.content.is_empty(), "note": "" }));
    }
    let port = headless_browser::active_chrome_port(&record.platform);
    if port == 0 {
        return Ok(json!({
            "success": true, "post": record, "fetched": false,
            "note": "该平台 Chrome 未连接（应用重启后会话失效）。请到「登录管理」点「打开 Chrome 登录」重新接管——cookie 保存在本机 profile，无需重新登录。",
        }));
    }

    let platform = record.platform.clone();
    let url = record.post_url.clone();
    let detail = tauri::async_runtime::block_on(async move {
        let ws_url = headless_browser::ws_url_for_port(port).await?;
        headless_browser::scrape_post_detail(&ws_url, &url, &platform).await
    });

    match detail {
        Ok(detail) => {
            let content = detail
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            let fetched = !content.is_empty();
            // 是否「成功解析到帖子」：抽取器 ok:true 且有标题或图片即算（纯图/无正文笔记 ok:true 也算）。
            // 与 fetched（内容非空）区分开——后者对无正文笔记永远为 false，不能拿来当缓存钥匙。
            let parsed = detail
                .get("ok")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                && (detail
                    .get("title")
                    .and_then(Value::as_str)
                    .map(|value| !value.trim().is_empty())
                    .unwrap_or(false)
                    || detail
                        .get("images")
                        .and_then(Value::as_array)
                        .map(|arr| !arr.is_empty())
                        .unwrap_or(false));
            // 远程图片本地化（CDN 校验 Referer，WebView 直连必 403 → 裂图）。
            let remote_images: Vec<String> = detail
                .get("images")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(str::trim))
                        .filter(|s| !s.is_empty())
                        .map(ToString::to_string)
                        .collect()
                })
                .unwrap_or_default();
            let remote_cover = detail
                .get("cover")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let (cached_images, cached_cover) =
                cache_post_images(state, &record.id, &record.platform, &remote_images, remote_cover);

            let updated = with_store_mut(state, |store| {
                let Some(item) = store
                    .monitored_posts
                    .iter_mut()
                    .find(|item| item.id == id)
                else {
                    return Err("动态不存在".to_string());
                };
                if fetched {
                    // **只在「新抓到的正文不比旧的短」时才覆盖**。
                    //
                    // 平台改版后提取器可能只拿得到标题级文本（抖音的
                    // `[data-e2e="video-desc"]` 已失效，正文会退化成标题，几十个字符）。
                    // 若无条件覆盖，任何一次「重新抓正文」/批量补字段都会把已经存好的
                    // 完整正文冲成标题 —— 而拆解、摘要都依赖全文，等于静默毁数据。
                    //
                    // 例外：旧内容是**脏数据**时必须允许用更短的新内容覆盖。
                    // 抖音那版坏提取抓出来的是「…3.6万举报发布时间：2025-09-19 17:30」，
                    // 比干净文案还长（71 vs 48），光比长度会把垃圾一直锁在库里。
                    // 旧的更脏就让位（哪怕更短），否则要求新内容不短于旧的。
                    // 判据见 should_replace_content 的注释。
                    if should_replace_content(&item.content, &content) {
                        item.content = content.clone();
                        item.excerpt = Some(content.chars().take(80).collect::<String>());
                    }
                }
                // 标题：列表页留下的脏标题（「播放量+文案+文案重复」）在这里纠正。
                //
                // 抖音列表页的 title 是拼接出来的，详情页抓到的文案是干净的，
                // 所以用详情文案做真值来修；小红书 / B 站的 title 本来就是标题，
                // 不满足「以数字开头」的条件，不会被改动。
                if let Some(fixed_title) = corrected_title_from_detail(&item.title, &content) {
                    item.title = fixed_title;
                }
                if let Some(mt) = detail
                    .get("mediaType")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    item.media_type = mt.to_string();
                }
                if let Some(cover) = cached_cover.as_ref() {
                    item.cover_url = Some(cover.clone());
                }
                // 作者：**有监控归属时一律不覆盖**。
                //
                // 列表抓取阶段 author 已经写成 watch.name（该 watch 的博主本人），这是权威值；
                // 详情页的 author 来自页面兜底链，会误抓正文里的 `@提及`（如「@抖音科技」）
                // 或推荐位的其他作者，于是同一作者裂成多个导出目录（实测踩坑）。
                // 只有「没有归属的临时帖（adhoc）」或「当前作者为空」才采用详情页的值。
                if detail_author_may_override(item.watch_id.as_deref(), &item.author) {
                    if let Some(author) = detail
                        .get("author")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                    {
                        item.author = author.to_string();
                    }
                }
                let incoming_date = detail
                    .get("date")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty());
                match incoming_date {
                    Some(date) => item.published_at = Some(date.to_string()),
                    None => {
                        // 抖音抽不到发布时间时留一条诊断（`dateDebug` 只在抖音分支里出现）：
                        // 空字符串 = DOM 选择器与两条 SSR 兜底全失效；有值 = 命中了但格式没解析出来。
                        // 不影响落库，纯粹为下次定位。
                        if detail.get("dateDebug").is_some() {
                            crate::logging::emit_legacy_line(
                                crate::logging::event::LogSource::Host,
                                crate::logging::event::LogLevel::Warn,
                                "monitor",
                                "monitor.detail.date_missing",
                                format!(
                                    "[monitor detail] 未解析到发布时间 platform={} dateDebug={:?} url={}",
                                    item.platform,
                                    detail.get("dateDebug").and_then(Value::as_str).unwrap_or(""),
                                    item.post_url
                                ),
                                json!({
                                    "postId": item.id,
                                    "platform": item.platform,
                                    "dateDebug": detail.get("dateDebug").cloned().unwrap_or(Value::Null),
                                }),
                                None,
                            );
                        }
                    }
                }
                let likes = detail.get("likes").and_then(Value::as_i64).unwrap_or(0);
                if likes > 0 {
                    item.likes = likes;
                }
                item.collects = detail
                    .get("collects")
                    .and_then(Value::as_i64)
                    .unwrap_or(item.collects);
                item.comments = detail
                    .get("comments")
                    .and_then(Value::as_i64)
                    .unwrap_or(item.comments);
                // 详情页图片列表落库：**已本地化**，二次打开才有缩略图可显示（也才能导出）
                if !cached_images.is_empty() {
                    item.images = cached_images.clone();
                }
                // 详情页成功解析即标记已抓取（图/标题抓到即可，纯图/无正文笔记不算失败），
                // 二次打开据此秒开，不再因 content 空而反复重抓。
                if parsed {
                    item.detail_fetched = true;
                }
                item.updated_at = now_i64().to_string();
                // 先把更新后的帖子克隆出来：item 是 store.monitored_posts 的可变借用，
                // clone 之后该借用即被 NLL 释放，才能把 store 整体再次可变借给 push_snapshot。
                let updated_item = item.clone();
                // 详情页抓到真实点赞/评论/收藏 → 落一条互动快照（复盘走势的数据源）
                push_snapshot(
                    store,
                    &updated_item.id,
                    &updated_item.platform,
                    &updated_item.author,
                    updated_item.likes,
                    updated_item.comments,
                    updated_item.collects,
                    now_i64(),
                );
                Ok(updated_item)
            })?;

            // 图文帖没有「分析视频」这个触发点 —— 详情抓取成功就直接导出产物，
            // 否则图文帖永远进不了导出库（实测：小红书图文抓下来没有产物目录）。
            // 视频帖仍由 analyze_video / transcribe 之后导出，避免这里多导一份。
            if updated.media_type != "video" && updated.detail_fetched {
                export_post_product(state, &updated.id);
            }
            Ok(json!({
                "success": true,
                "post": updated,
                "fetched": fetched,
                "images": detail.get("images").cloned().unwrap_or(Value::Null),
                "note": if fetched {
                    "".to_string()
                } else if parsed {
                    // 解析成功但无正文：纯图 / 无正文笔记属正常，不该弹「未解析」告警
                    "".to_string()
                } else {
                    // 真解析失败 → 把诊断信息带回给用户：落地到哪个页、页面里有什么，
                    // 这样能区分「未登录 / 被 xsec_token 拦 / 验证码 / 改版」。
                    let mut hint =
                        "页面未解析到正文，已展示缓存信息".to_string();
                    if Platform::parse(&record.platform) == Some(Platform::Xiaohongshu)
                        && !record.post_url.contains("xsec_token")
                    {
                        hint.push_str("；链接缺 xsec_token（可能已过期），请重新抓取该博主");
                    }
                    // 正文候选元素诊断（抽取器在「标题有、正文空」时回传），便于定位选择器失效
                    if let Some(cdbg) = detail
                        .get("contentDebug")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                    {
                        hint.push_str(&format!("；正文候选：{cdbg}"));
                    }
                    if let Some(page_title) = detail
                        .get("pageTitle")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                    {
                        hint.push_str(&format!("；落地页「{page_title}」"));
                    }
                    if let Some(snippet) = detail
                        .get("snippet")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                    {
                        hint.push_str(&format!("：{snippet}"));
                    }
                    hint
                },
            }))
        }
        Err(error) => {
            if headless_browser::is_connection_error(&error) {
                // 该平台 Chrome 大概率已被关闭/崩溃：清掉失效会话，UI 回到「未启动」，
                // 提示用户重新打开（cookie 在 profile 里，无需重新登录）。
                headless_browser::deactivate_session(&record.platform);
                Ok(json!({
                    "success": true, "post": record, "fetched": false,
                    "note": "该平台的 Chrome 已断开（可能被手动关闭或崩溃）。请到「登录管理」重新打开并登录后再试。",
                }))
            } else {
                Ok(json!({
                    "success": true, "post": record, "fetched": false,
                    "note": format!("抓正文失败: {error}"),
                }))
            }
        }
    }
}

pub(super) fn host_of(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(|host| host.to_string()))
}

/// 从 B 站视频链接里取 BV 号（`…/video/BV1xx411c7mD/?p=1` → `BV1xx411c7mD`）。
pub(super) fn bv_of_url(url: &str) -> Option<String> {
    let rest = url.split_once("/video/")?.1;
    let bv = rest
        .split(|c| c == '?' || c == '#' || c == '/')
        .next()
        .unwrap_or("");
    if bv.starts_with("BV") && bv.len() > 4 {
        Some(bv.to_string())
    } else {
        None
    }
}

/// 两个主机是否同属一个可注册域（`space.bilibili.com` ≈ `www.bilibili.com`）。
///
/// B 站主页在 `space.bilibili.com`、视频在 `www.bilibili.com`、部分活动页在 `m.bilibili.com`，
/// 用精确 host 相等会把合集/API 补全的结果全部过滤掉。
pub(super) fn same_registrable_domain(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let root = |h: &str| -> String {
        let parts: Vec<&str> = h.split('.').filter(|p| !p.is_empty()).collect();
        if parts.len() >= 2 {
            format!("{}.{}", parts[parts.len() - 2], parts[parts.len() - 1])
        } else {
            h.to_string()
        }
    };
    if a.is_empty() || b.is_empty() {
        return false;
    }
    root(a) == root(b)
}

pub(super) fn platform_of(url: &str) -> String {
    let host = host_of(url).unwrap_or_default();
    Platform::from_host(&host)
        .unwrap_or(Platform::Web)
        .as_str()
        .to_string()
}

/// 决定一条帖子记录归属哪个监控项（`watch_id`）。
///
/// 返回 `Some(watch_id)` 表示这条帖子归该监控项所有，`None` 表示保持原样（不覆盖）。
///
/// 背景：落库时如果无条件写 `watch_id = 当前 watch`，那么「同一个博主被建了
/// 两个监控项」（或两个用户监控同一博主）时，后抓的一方会把先抓的记录整个抢走 ——
/// 先抓的那个用户的卡片上「已收录 N 篇」会凭空归零，看起来就像「抓到了却没存进去」。
///
/// 规则：
/// - 记录已有归属，且那个监控项**还在** → **保留原归属**，不抢（不同监控项各管各的）。
/// - 记录已有归属，但那个监控项**已被删除** → 归给当前监控项。
///   否则这些帖子会永远挂在不存在的 watch 上，卡片数恒为 0、也永远不会被接管。
/// - 记录还没有归属（临时粘贴单帖抓的孤儿记录）→ 归给当前监控项（顺手收编）。
/// - 新记录 → 归给当前监控项。
pub(super) fn resolve_watch_id(
    existing_watch_id: Option<&str>,
    current_watch_id: Option<&str>,
    original_watch_exists: bool,
) -> Option<String> {
    match existing_watch_id {
        Some(owner) if original_watch_exists => Some(owner.to_string()),
        _ => current_watch_id.map(str::to_string),
    }
}

/// 单次抓取条数：**固定值，不再对用户开放配置**。
///
/// 之前由前端输入框 + `store.monitor_scan_limit` 配置，但实际只有一个合理量级（每天增量窗口），
/// 让用户填反而制造「抓太少漏帖 / 抓太多滑动超时」两类问题。这里收口成常量：
/// `resolve_scan_limit` 忽略 payload 与持久化值，`get_scan_limit` 也只回这个数（前端据此显示）。
/// 补历史的深翻窗口另由 `BACKFILL_MAX` 控制，不受影响。
pub(super) const SCAN_LIMIT: usize = 30;

/// 解析「单次抓取条数」：恒定返回 `SCAN_LIMIT`，忽略请求 payload 与历史持久化配置。
///
/// 保留函数签名是为了让所有调用点（fetch_monitor / adhoc 抓取 / 每日预取）无需改动，
/// 也避免旧前端传 `limit` 时报错。
/// 单次抓取条数。默认 `SCAN_LIMIT`（日常增量只要最近 30 条）；
/// 补历史深翻会在 payload 里显式传 `limit = BACKFILL_MAX(500)`，必须尊重它——
/// 否则滚动循环里的 `captured >= limit` 会在第一屏就把 500 的目标截成 30。
pub(super) fn resolve_scan_limit(_state: &State<'_, AppState>, payload: &Value) -> usize {
    let requested = payload
        .get("limit")
        .or_else(|| payload.get("scanLimit"))
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(SCAN_LIMIT);
    requested.clamp(1, 2000)
}

/// 异常值检测：把某帖当前点赞与该作者近窗口的点赞中位数比较，超过阈值倍数即判为爆文。
///
/// 为什么用「作者中位数」而非「该帖自身历史」：
/// - 竞品「互动≥平时3倍」的「平时」= 该作者常态水平，作者级基线最贴合语义；
/// - 单帖自身历史在刚跟踪时只有 1 个点，无法构成基线；
/// - 中位数对历史中的偶发爆文鲁棒，不会把基线抬高导致漏判。
///
/// 数据口径：`post_snapshots` 仅在有真实互动时落点（push_snapshot 护栏跳过全零），且 CDP
/// 列表抓取只回点赞数（评论/收藏需打开详情才补），故基线与实际比较统一用 `likes`。
/// 因此「倍数」在列表态实为「点赞倍数」，开了详情的帖仍是同一口径，倍数语义一致。
///
/// 守卫：作者为空 / 当前点赞 ≤0 / 阈值 ≤0 / 历史有效样本 <2 / 基线中位数 ≤0 一律不判爆。
pub(super) fn compute_spike(
    store: &AppStore,
    author: &str,
    platform: &str,
    current_likes: i64,
    window_days: i64,
    threshold: f64,
) -> (bool, f64) {
    if author.is_empty() || current_likes <= 0 || threshold <= 0.0 {
        return (false, 0.0);
    }
    let now = now_i64();
    let cutoff = now - window_days * 86_400_000;
    // 取该作者近窗口内、每帖最新一条快照的 likes（同帖多次抓取只留最新，避免重复计数）
    use std::collections::HashMap;
    let mut latest: HashMap<String, (i64, i64)> = HashMap::new(); // post_id -> (captured_at, likes)
    for s in &store.post_snapshots {
        if s.captured_at < cutoff || s.likes <= 0 {
            continue;
        }
        if s.author != author {
            continue;
        }
        if !platform.is_empty() && s.platform != platform {
            continue;
        }
        match latest.get(&s.post_id) {
            Some((prev_ts, _)) if s.captured_at <= *prev_ts => {}
            _ => {
                latest.insert(s.post_id.clone(), (s.captured_at, s.likes));
            }
        }
    }
    let mut likes_vec: Vec<i64> = latest.values().map(|&(_, l)| l).collect();
    if likes_vec.len() < 2 {
        return (false, 0.0);
    }
    likes_vec.sort_unstable();
    let n = likes_vec.len();
    let median = if n % 2 == 1 {
        likes_vec[n / 2] as f64
    } else {
        (likes_vec[n / 2 - 1] + likes_vec[n / 2]) as f64 / 2.0
    };
    if median <= 0.0 {
        return (false, 0.0);
    }
    let ratio = current_likes as f64 / median;
    (ratio >= threshold, ratio)
}

/// 读取当前异常倍数阈值（0 / 缺失回落默认 3.0，便于前端初始化与计算兜底）。
pub(super) fn resolve_spike_threshold(state: &State<'_, AppState>) -> f64 {
    const DEFAULT_THRESHOLD: f64 = 3.0;
    with_store(state, |store| Ok(store.monitor_spike_threshold))
        .ok()
        .filter(|value| *value > 0.0)
        .unwrap_or(DEFAULT_THRESHOLD)
}

/// 全局抓取互斥：同一时刻只允许一轮抓取（手动补历史 / 定时增量 / 多 watch 深翻）。
/// 否则会同时开多个 CDP 标签抢同一个 Chrome，互相拖慢甚至触发平台限流。
pub(super) static MONITOR_FETCH_INFLIGHT: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

pub(super) struct MonitorFetchGuard;

impl MonitorFetchGuard {
    pub(super) fn acquire() -> Option<Self> {
        use std::sync::atomic::Ordering;
        if MONITOR_FETCH_INFLIGHT
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            Some(Self)
        } else {
            None
        }
    }
}

impl Drop for MonitorFetchGuard {
    fn drop(&mut self) {
        MONITOR_FETCH_INFLIGHT.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

pub(super) fn fetch_monitor(
    state: &State<'_, AppState>,
    payload: &Value,
    app: Option<AppHandle>,
    emit_progress: bool,
) -> Result<Value, String> {
    let _fetch_guard = match MonitorFetchGuard::acquire() {
        Some(guard) => guard,
        None => return Err("monitor busy: 另一轮抓取正在进行，请稍候再试".to_string()),
    };
    let watch: Option<crate::CompetitorWatchRecord> =
        if let Some(id) = str_field(payload, "watchId").or_else(|| str_field(payload, "watch_id")) {
            with_store(state, |store| {
                guard_store_target(&store, "monitor:fetch", &json!({ "watchId": id }))?;
                Ok(store
                    .competitor_watchlist
                    .iter()
                    .find(|item| item.id == id)
                    .cloned())
            })?
        } else {
            None
        };

    let url = str_field(payload, "url")
        .or_else(|| watch.as_ref().and_then(|item| item.profile_url.clone()))
        .ok_or_else(|| "缺少 url 或 watchId".to_string())?;

    // 小红书 / 抖音是 JS 渲染 + 登录态站点，纯 HTTP 抽不到帖子。
    // 若已用 Chrome 登录则走 CDP 抓取，否则给出明确的登录引导（不报错，避免噪音）。
    let platform_hint = watch
        .as_ref()
        .map(|item| item.platform.clone())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| platform_of(&url));
    if Platform::parse(&platform_hint).map(|p| p.is_cdp()).unwrap_or(false) {
        if headless_browser::active_chrome_port(&platform_hint) == 0 {
            return Ok(json!({
                "success": true,
                "profileUrl": url,
                "scanned": 0,
                "created": [],
                "updated": [],
                "capturedVia": "chrome-cdp",
                "note": "小红书 / 抖音 / B 站需要登录态抓取。请先在左侧「登录管理」点「打开 Chrome 登录」完成登录，再回来抓取。",
            }));
        }
        return fetch_monitor_via_cdp(state, watch.as_ref(), &url, &platform_hint, payload, app, emit_progress);
    }

    // 抓取博主主页，抽取同域帖子链接。注意：抓取在 store 锁之外进行，避免网络阻塞时持锁。
    let page = web_access::fetch(&json!({
        "url": url,
        "maxChars": 30_000,
        "includeLinks": true,
    }))?;
    let links = page
        .get("links")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let base_host = host_of(&url).unwrap_or_default();

    let candidates: Vec<String> = links
        .iter()
        .filter_map(|link| link.get("url").and_then(Value::as_str).map(String::from))
        .filter(|candidate| {
            host_of(candidate).map(|host| host == base_host).unwrap_or(false)
                && candidate != &url
        })
        .take(resolve_scan_limit(state, payload))
        .collect();

    let (author_name, platform, watch_id) = match &watch {
        Some(item) => (item.name.clone(), item.platform.clone(), Some(item.id.clone())),
        None => (
            host_of(&url).unwrap_or_else(|| "web".to_string()),
            platform_of(&url),
            None,
        ),
    };

    let now = now_i64();
    let mut created: Vec<crate::MonitoredPostRecord> = Vec::new();
    let mut updated: Vec<crate::MonitoredPostRecord> = Vec::new();

    for candidate in &candidates {
        let post = match web_access::fetch(&json!({ "url": candidate, "maxChars": 12_000 })) {
            Ok(fetched) => fetched,
            Err(_) => continue,
        };
        let title = post
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("未命名")
            .to_string();
        let text = post
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let excerpt = if text.chars().count() > 200 {
            text.chars().take(200).collect()
        } else {
            text.clone()
        };

        // 与已抓取的同 post_url 对比，判断是否新动态。
        // 同样按「归属用户 + URL」限定：否则两个用户监控同一博主时，
        // 后抓的会把先抓的记录抢走（watch_id 被覆盖），先抓的用户卡片上的条数归零。
        let scope_owner = watch
            .as_ref()
            .map(|item| item.owner.clone())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(crate::identity::owner_or_admin);
        let (existing, original_watch_exists): (Option<crate::MonitoredPostRecord>, bool) =
            with_store(state, |store| {
                let found = store
                    .monitored_posts
                    .iter()
                    .find(|item| {
                        &item.post_url == candidate
                            && crate::identity::owner_equals(&item.owner, &scope_owner)
                    })
                    .cloned();
                let alive = found
                    .as_ref()
                    .and_then(|item| item.watch_id.as_deref())
                    .map(|watch_id| {
                        store
                            .competitor_watchlist
                            .iter()
                            .any(|entry| entry.id == watch_id)
                    })
                    .unwrap_or(false);
                Ok((found, alive))
            })?;

        let (is_new, prev_likes, prev_comments) = match &existing {
            Some(previous) => (false, previous.likes, previous.comments),
            None => (true, 0, 0),
        };
        // web/详情路径无互动数（likes=0）→ 不重算异常，沿用旧值（新帖默认 false，避免误标爆文）
        let (spike, spike_ratio) = match &existing {
            Some(previous) => (previous.spike, previous.spike_ratio),
            None => (false, 0.0),
        };

        // 同 watch 抓取：网页路径也拿不到互动数，已存在记录一律沿用旧值，
        // 避免重新抓取把 media_type / 正文 / 收藏评论清零（详见另一处同名注释）。
        let looks_like_video = candidate.contains("/video/");
        let record = crate::MonitoredPostRecord {
            id: existing
                .as_ref()
                .map(|previous| previous.id.clone())
                .unwrap_or_else(|| make_id("mpost")),
            owner: existing
                .as_ref()
                .map(|previous| previous.owner.clone())
                .filter(|value| !value.is_empty())
                .or_else(|| watch.as_ref().map(|item| item.owner.clone()).filter(|value| !value.is_empty()))
                .unwrap_or_else(crate::identity::owner_or_admin),
            watch_id: resolve_watch_id(
                existing.as_ref().and_then(|previous| previous.watch_id.as_deref()),
                watch_id.as_deref(),
                original_watch_exists,
            ),
            platform: platform.clone(),
            author: author_name.clone(),
            author_url: watch.as_ref().and_then(|item| item.profile_url.clone()),
            post_url: candidate.clone(),
            title,
            media_type: existing
                .as_ref()
                .map(|previous| previous.media_type.clone())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| {
                    if looks_like_video {
                        "video".to_string()
                    } else {
                        "image".to_string()
                    }
                }),
            cover_url: existing.as_ref().and_then(|previous| previous.cover_url.clone()),
            images: existing
                .as_ref()
                .map(|previous| previous.images.clone())
                .unwrap_or_default(),
            // 已有正文一律保留，绝不被本次页面文本覆盖。
            // 网页路径拿到的 `text` 只是页面可见文字（小红书常是那串 hashtag），
            // 直接覆盖会把详情抓到的完整正文冲掉——重启后 Chrome 会话失效、定时抓取
            // 改走网页路径时必现（CDP 路径早已修过同样的坑）。
            content: existing
                .as_ref()
                .map(|previous| previous.content.clone())
                .filter(|value| !value.trim().is_empty())
                .unwrap_or(text),
            detail_fetched: existing
                .as_ref()
                .map(|previous| previous.detail_fetched)
                .unwrap_or(false),
            excerpt: existing
                .as_ref()
                .and_then(|previous| previous.excerpt.clone())
                .or(Some(excerpt)),
            published_at: existing
                .as_ref()
                .and_then(|previous| previous.published_at.clone())
                // 合集/收藏夹 API 直接给发布日期（yyyy-mm-dd），新帖落库时用它，
                // 省掉「打开详情才补时间」的一次往返。
                .or_else(|| {
                    post.get("publishedAt")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(ToString::to_string)
                }),
            likes: existing.as_ref().map(|previous| previous.likes).unwrap_or(0),
            comments: existing
                .as_ref()
                .map(|previous| previous.comments)
                .unwrap_or(0),
            collects: existing
                .as_ref()
                .map(|previous| previous.collects)
                .unwrap_or(0),
            prev_likes,
            prev_comments,
            is_new,
            spike,
            spike_ratio,
            captured_via: "monitor".to_string(),
            topic_injected: existing
                .as_ref()
                .map(|previous| previous.topic_injected)
                .unwrap_or(false),
            deconstructed: existing
                .as_ref()
                .map(|previous| previous.deconstructed)
                .unwrap_or(false),
            // 视频下载态沿用旧记录，重新抓取不覆盖已下好的文件
            video_local_path: existing
                .as_ref()
                .and_then(|previous| previous.video_local_path.clone()),
            video_preview_path: existing
                .as_ref()
                .and_then(|previous| previous.video_preview_path.clone()),
            video_size: existing.as_ref().and_then(|previous| previous.video_size),
            video_status: existing
                .as_ref()
                .map(|previous| previous.video_status.clone())
                .filter(|status| !status.is_empty())
                .unwrap_or_else(|| "none".to_string()),
            video_downloaded_at: existing
                .as_ref()
                .and_then(|previous| previous.video_downloaded_at),
            video_error: existing.as_ref().and_then(|previous| previous.video_error.clone()),
            video_has_audio: existing.as_ref().and_then(|previous| previous.video_has_audio),
            created_at: existing
                .as_ref()
                .map(|previous| previous.created_at.clone())
                .unwrap_or_else(|| now.to_string()),
            updated_at: now.to_string(),
        };

        with_store_mut(state, |store| {
            if let Some(position) =
                store.monitored_posts.iter().position(|item| item.id == record.id)
            {
                store.monitored_posts[position] = record.clone();
            } else {
                store.monitored_posts.push(record.clone());
            }
            // 互动快照（web 路径为全零，push_snapshot 内部会跳过）
            push_snapshot(
                store,
                &record.id,
                &record.platform,
                &record.author,
                record.likes,
                record.comments,
                record.collects,
                now,
            );
            Ok::<(), String>(())
        })?;

        if is_new {
            created.push(record);
        } else {
            updated.push(record);
        }
    }

    if let Some(item) = &watch {
        with_store_mut(state, |store| {
            if let Some(position) = store
                .competitor_watchlist
                .iter()
                .position(|entry| entry.id == item.id)
            {
                store.competitor_watchlist[position].last_fetch_at = Some(now.to_string());
                store.competitor_watchlist[position].updated_at = now.to_string();
            }
            Ok::<(), String>(())
        })?;
    }

    Ok(json!({
        "success": true,
        "profileUrl": url,
        "scanned": candidates.len(),
        "created": created,
        "updated": updated,
        "note": if candidates.is_empty() {
            "未从主页抽取到同域帖子链接（多见于 JS 动态渲染站点）。可改用单篇链接抓取，或手动粘贴帖子 URL 到 monitor:fetch 的 url 字段。"
        } else {
            ""
        },
    }))
}

/// 临时抓取（无 watch）时的作者解析。
///
/// 优先抓一次帖子详情拿真实昵称（`scrape_post_detail` 的 `author`），失败回退平台中文名。
/// 目标 URL：输入本身是帖子链接就用它，否则从抓到的 posts 里挑第一条帖子链接。
pub(super) fn resolve_adhoc_author(port: u16, url: &str, posts: &[Value], platform: &str) -> String {
    let looks_like_post = |u: &str| {
        u.contains("/video/") || u.contains("/note/") || u.contains("/explore/") || u.contains("/item/")
    };
    let target = if looks_like_post(url) {
        Some(url.to_string())
    } else {
        posts
            .iter()
            .filter_map(|p| p.get("url").and_then(Value::as_str))
            .map(str::to_string)
            .find(|u| looks_like_post(u))
    };
    if let (Some(target), true) = (target, port > 0) {
        let detail = tauri::async_runtime::block_on(async {
            let ws_url = headless_browser::ws_url_for_port(port).await?;
            headless_browser::scrape_post_detail(&ws_url, &target, platform).await
        });
        if let Ok(detail) = detail {
            if let Some(author) = detail
                .get("author")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                return author.to_string();
            }
        }
    }
    Platform::parse(platform)
        .map(|p| p.label().to_string())
        .unwrap_or_else(|| "未知作者".to_string())
}

/// 通过已登录的 Chrome（CDP）抓取小红书 / 抖音博主主页，落库到 monitored_posts。
/// 在 `spawn_blocking` 上下文（见 ipc_invoke）内，使用 `block_on` 驱动异步 CDP 调用是安全的。
pub(super) fn fetch_monitor_via_cdp(
    state: &State<'_, AppState>,
    watch: Option<&crate::CompetitorWatchRecord>,
    url: &str,
    platform: &str,
    payload: &Value,
    app: Option<AppHandle>,
    emit_progress: bool,
) -> Result<Value, String> {
    let port = headless_browser::active_chrome_port(platform);
    let wait_selector = crate::scraper::adapter_for_str(platform)
        .map(|adapter| adapter.list_wait_selector())
        .unwrap_or("a[href*=\"/explore/\"]");
    let watch_id = watch.map(|item| item.id.clone()).unwrap_or_default();
    // limit 必须在抓取前解析：抽取器按它决定滚动到第几屏 / 收多少条。
    let limit = resolve_scan_limit(state, payload);
    let scraped = tauri::async_runtime::block_on(async move {
        let ws_url = headless_browser::ws_url_for_port(port).await?;
        // 补历史（emit_progress=true）走 Rust 侧循环驱动的「深翻」抓取：
        // 能实时上报进度、尽量翻到底；日常增量仍用原 scroll 抓取（小窗口，不需要进度）。
        if emit_progress {
            headless_browser::scrape_profile_posts_paged(
                &ws_url,
                url,
                wait_selector,
                limit,
                app,
                &watch_id,
                emit_progress,
            )
            .await
        } else {
            headless_browser::scrape_profile_posts(&ws_url, url, wait_selector, limit).await
        }
    })?;

    // 抽取器返回 { posts: [...], exhausted: 是否翻到主页底部, diag: 滚动诊断 }。
    // exhausted 是「历史已抓完」的判据：翻到底了才说明这个博主的历史补齐了。
    // diag 用来定位「为什么没翻到更深处」（首屏就停了 / 滚错容器 / 页面就是不给更多）。
    let exhausted = scraped
        .get("exhausted")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let scroll_diag = scraped.get("diag").cloned().unwrap_or(Value::Null);
    if !scroll_diag.is_null() {
        println!("[monitor] profile scroll diag: {scroll_diag}");
    }
    // 主页声明的作品总数（进度条分母；补历史深翻时由 paged 抓取返回）
    let profile_total = scraped
        .get("profileTotal")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let mut posts = scraped
        .get("posts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    // B 站：DOM 只覆盖当前页签（投稿列表）；合集 / 收藏夹 / 空间下全部合集走公开 API 补全，
    // 否则「合集里的视频」永远抓不到（诉求：博主数据要拿全）。失败只记日志，不影响 DOM 结果。
    let is_bili = Platform::parse(platform) == Some(Platform::Bilibili);
    let mut extras_added = 0usize;
    // 合集/收藏夹补全只属于「补历史」（emit_progress=true）：
    // 普通「抓取」是日增量，只该拿滚动窗口里的 30 条；否则点一次抓取就灌进几百条老视频。
    if is_bili && emit_progress {
        let extra = tauri::async_runtime::block_on(async {
            let ws = headless_browser::ws_url_for_port(port).await?;
            headless_browser::scrape_bilibili_extras(&ws, url).await
        })
        .unwrap_or_else(|e| {
            println!("[monitor] bilibili extras failed: {e}");
            json!({ "posts": [] })
        });
        if let Some(arr) = extra.get("posts").and_then(Value::as_array) {
            let mut seen: std::collections::HashSet<String> = posts
                .iter()
                .filter_map(|p| p.get("url").and_then(Value::as_str))
                .map(|s| s.split('?').next().unwrap_or(s).to_string())
                .collect();
            let before = posts.len();
            for item in arr {
                if let Some(u) = item.get("url").and_then(Value::as_str) {
                    let key = u.split('?').next().unwrap_or(u).to_string();
                    if seen.insert(key) {
                        posts.push(item.clone());
                    }
                }
            }
            extras_added = posts.len() - before;
            println!(
                "[monitor] bilibili extras: +{extras_added} → {} 条（{}）",
                posts.len(),
                extra.get("debug").cloned().unwrap_or(Value::Null)
            );
        }
    }
    // B 站列表卡片只有播放量、没有点赞 —— 统一补一次互动数（点赞/评论/收藏）。
    // 只补 likes 仍为 0 的条目并限量 60，避免补历史时把几百条全查一遍触发风控（-412）。
    if is_bili {
        let bvs: Vec<String> = {
            let mut seen = std::collections::HashSet::new();
            posts.iter()
                .filter(|p| p.get("likes").and_then(Value::as_i64).unwrap_or(0) <= 0)
                .filter_map(|p| p.get("url").and_then(Value::as_str))
                .filter_map(bv_of_url)
                .filter(|bv| seen.insert(bv.clone()))
                .take(60)
                .collect()
        };
        if !bvs.is_empty() {
            let stats = tauri::async_runtime::block_on(async {
                let ws = headless_browser::ws_url_for_port(port).await?;
                headless_browser::fetch_bilibili_stats(&ws, url, &bvs).await
            })
            .unwrap_or_else(|e| {
                println!("[monitor] bilibili stats failed: {e}");
                json!({})
            });
            let mut filled = 0usize;
            for post in posts.iter_mut() {
                let Some(link) = post.get("url").and_then(Value::as_str).map(str::to_string) else {
                    continue;
                };
                let Some(bv) = bv_of_url(&link) else { continue };
                let Some(st) = stats.get(&bv) else { continue };
                if let Some(obj) = post.as_object_mut() {
                    obj.insert("likes".to_string(), st.get("like").cloned().unwrap_or(json!(0)));
                    obj.insert("comments".to_string(), st.get("reply").cloned().unwrap_or(json!(0)));
                    obj.insert("collects".to_string(), st.get("favorite").cloned().unwrap_or(json!(0)));
                    filled += 1;
                }
            }
            println!("[monitor] bilibili stats filled: {filled}/{}", bvs.len());
        }
    }

    let base_host = host_of(url).unwrap_or_default();
    // 作者解析：绑定了对标博主就用博主名；**临时粘贴单帖（无 watch）不能拿域名当作者**
    // （会显示成 `www.douyin.com`），改为抓一次帖子详情取真实昵称，失败再回退平台中文名。
    let author_name = match watch {
        Some(item) => item.name.clone(),
        None => resolve_adhoc_author(port, url, &posts, platform),
    };
    let now = now_i64();
    let mut created: Vec<crate::MonitoredPostRecord> = Vec::new();
    let mut updated: Vec<crate::MonitoredPostRecord> = Vec::new();

    // 补历史（深翻 + 合集补全）才放宽上限；普通抓取维持 30 条日增量窗口。
    let effective_limit = if is_bili && emit_progress {
        limit.max(600)
    } else {
        limit
    };
    let scraped_posts = posts.len() as i64;
    let mut dropped_domain = 0i64;
    let mut updated_existing = 0i64;
    for post in posts.iter().take(effective_limit) {
        let post_url = post
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        // 同站但不同子域要放行（B 站主页 space.* / 视频 www.*），否则合集结果会被整体丢掉
        if post_url.is_empty()
            || host_of(&post_url)
                .map(|host| !same_registrable_domain(&host, &base_host))
                .unwrap_or(true)
        {
            dropped_domain += 1;
            continue;
        }
        let title = post
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("未命名")
            .to_string();
        let likes = post.get("likes").and_then(Value::as_i64).unwrap_or(0);
        // xhs 的 URL 带 xsec_token 且可能轮换 —— 身份判定用去掉 query 的裸 URL，
        // 完整 URL（含 token）留在 post_url 里供「原文」打开。
        let post_key = post_url.split('?').next().unwrap_or(&post_url).to_string();

        // 找已有记录：**按 (归属用户, 裸 URL) 匹配**，同一用户下的同一条帖子只留一份。
        //
        // 原来只有全局匹配（连 owner 都不看）。两个用户监控同一个博主时，
        // 后抓的那个会把先抓的记录整个抢走 —— watch_id 被覆盖后，先抓的用户
        // 卡片上的「已收录 N 篇」凭空归零（跨用户串数据）。
        //
        // 归属用户必须取**监控项的 owner**，不能取当前请求的 owner：
        // 定时任务线程里没有请求身份（= admin），但它抓的是各用户自己的监控项，
        // 用 admin 去比会把所有用户的帖子混成一堆。
        let scope_owner = watch
            .map(|item| item.owner.clone())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(crate::identity::owner_or_admin);
        // 同时查出「原归属的监控项是否还存在」——已被删除的 watch 其帖子要能被接管。
        let (existing, original_watch_exists): (Option<crate::MonitoredPostRecord>, bool) =
            with_store(state, |store| {
                let found = store
                    .monitored_posts
                    .iter()
                    .find(|item| {
                        item.post_url.split('?').next().unwrap_or(&item.post_url) == post_key
                            && crate::identity::owner_equals(&item.owner, &scope_owner)
                    })
                    .cloned();
                let alive = found
                    .as_ref()
                    .and_then(|item| item.watch_id.as_deref())
                    .map(|watch_id| {
                        store
                            .competitor_watchlist
                            .iter()
                            .any(|entry| entry.id == watch_id)
                    })
                    .unwrap_or(false);
                Ok((found, alive))
            })?;
        // 已存在：点赞数变化视为「动态信号」；新链接：视为新动态。
        let (is_new, prev_likes, prev_comments) = match &existing {
            Some(previous) => (false, previous.likes, previous.comments),
            None => (true, 0, 0),
        };
        // CDP 列表带真实点赞 → 真正计算异常值（作者近窗口点赞中位数 × 阈值倍数）
        let threshold = resolve_spike_threshold(state);
        let (spike, spike_ratio) = with_store(state, |store| {
            Ok(compute_spike(
                &*store,
                &author_name,
                platform,
                likes,
                30,
                threshold,
            ))
        })?;

        // 列表页只有 url/标题/点赞，拿不到正文、媒体类型、收藏/评论。
        // 对已存在的帖子一律**沿用旧值**，否则每次重新抓取都会把已抓好的
        // media_type(→image)、content(→空)、collects/comments(→0) 清零
        // （实测踩坑：补历史一次，73 条视频全变 image、正文 36→7、赞藏评归零）。
        // 抖音的 URL 就能判定媒体类型：/video/ 是视频，/note/ 是图文。
        let looks_like_video = post_url.contains("/video/");
        let current_watch_id = watch.map(|item| item.id.as_str());
        let record = crate::MonitoredPostRecord {
            id: existing
                .as_ref()
                .map(|previous| previous.id.clone())
                .unwrap_or_else(|| make_id("mpost")),
            owner: existing
                .as_ref()
                .map(|previous| previous.owner.clone())
                .filter(|value| !value.is_empty())
                .or_else(|| watch.map(|item| item.owner.clone()).filter(|value| !value.is_empty()))
                .unwrap_or_else(crate::identity::owner_or_admin),
            // 归属不被抢：已有归属就保留（见 resolve_watch_id 的注释）。
            watch_id: resolve_watch_id(
                existing.as_ref().and_then(|previous| previous.watch_id.as_deref()),
                current_watch_id,
                original_watch_exists,
            ),
            platform: platform.to_string(),
            author: author_name.clone(),
            author_url: watch.and_then(|item| item.profile_url.clone()),
            post_url,
            title: title.clone(),
            media_type: existing
                .as_ref()
                .map(|previous| previous.media_type.clone())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| {
                    if looks_like_video {
                        "video".to_string()
                    } else {
                        "image".to_string()
                    }
                }),
            cover_url: existing.as_ref().and_then(|previous| previous.cover_url.clone()),
            images: existing
                .as_ref()
                .map(|previous| previous.images.clone())
                .unwrap_or_default(),
            content: existing
                .as_ref()
                .map(|previous| previous.content.clone())
                .unwrap_or_default(),
            detail_fetched: existing
                .as_ref()
                .map(|previous| previous.detail_fetched)
                .unwrap_or(false),
            excerpt: match existing.as_ref() {
                // 旧记录里 excerpt 若只是「旧标题的副本」，跟着新标题刷新；
                // 否则 B 站标题曾被抓成「播放量+弹幕+时长」的脏数据会一直留在列表副标题里。
                Some(previous) => match previous.excerpt.clone() {
                    Some(old) if old.trim() != previous.title.trim() => Some(old),
                    _ => Some(title.clone()),
                },
                None => Some(title.clone()),
            },
            published_at: existing
                .as_ref()
                .and_then(|previous| previous.published_at.clone())
                // 合集/收藏夹 API 直接给发布日期（yyyy-mm-dd），新帖落库时用它，
                // 省掉「打开详情才补时间」的一次往返。
                .or_else(|| {
                    post.get("publishedAt")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(ToString::to_string)
                }),
            // 列表点赞可能为 0（抖音列表不带点赞）→ 不要覆盖已有的真实点赞
            likes: if likes > 0 {
                likes
            } else {
                existing.as_ref().map(|previous| previous.likes).unwrap_or(0)
            },
            // 新帖才有互动数（B 站由 archive/stat 补全，其它平台通常为 0）；老帖一律沿用旧值。
            comments: existing
                .as_ref()
                .map(|previous| previous.comments)
                .or_else(|| post.get("comments").and_then(Value::as_i64))
                .unwrap_or(0),
            collects: existing
                .as_ref()
                .map(|previous| previous.collects)
                .or_else(|| post.get("collects").and_then(Value::as_i64))
                .unwrap_or(0),
            prev_likes,
            prev_comments,
            is_new,
            spike,
            spike_ratio,
            captured_via: "chrome-cdp".to_string(),
            topic_injected: existing
                .as_ref()
                .map(|previous| previous.topic_injected)
                .unwrap_or(false),
            deconstructed: existing
                .as_ref()
                .map(|previous| previous.deconstructed)
                .unwrap_or(false),
            // 视频下载态沿用旧记录，重新抓取不覆盖已下好的文件
            video_local_path: existing
                .as_ref()
                .and_then(|previous| previous.video_local_path.clone()),
            video_preview_path: existing
                .as_ref()
                .and_then(|previous| previous.video_preview_path.clone()),
            video_size: existing.as_ref().and_then(|previous| previous.video_size),
            video_status: existing
                .as_ref()
                .map(|previous| previous.video_status.clone())
                .filter(|status| !status.is_empty())
                .unwrap_or_else(|| "none".to_string()),
            video_downloaded_at: existing
                .as_ref()
                .and_then(|previous| previous.video_downloaded_at),
            video_error: existing.as_ref().and_then(|previous| previous.video_error.clone()),
            video_has_audio: existing.as_ref().and_then(|previous| previous.video_has_audio),
            created_at: existing
                .as_ref()
                .map(|previous| previous.created_at.clone())
                .unwrap_or_else(|| now.to_string()),
            updated_at: now.to_string(),
        };

        with_store_mut(state, |store| {
            if let Some(position) = store.monitored_posts.iter().position(|item| item.id == record.id)
            {
                store.monitored_posts[position] = record.clone();
            } else {
                store.monitored_posts.push(record.clone());
            }
            // 互动快照（CDP 列表路径带真实点赞数）
            push_snapshot(
                store,
                &record.id,
                &record.platform,
                &record.author,
                record.likes,
                record.comments,
                record.collects,
                now,
            );
            Ok::<(), String>(())
        })?;

        if is_new {
            created.push(record);
        } else {
            updated_existing += 1;
            updated.push(record);
        }
    }

    if let Some(item) = watch {
        with_store_mut(state, |store| {
            if let Some(position) = store
                .competitor_watchlist
                .iter()
                .position(|entry| entry.id == item.id)
            {
                store.competitor_watchlist[position].last_fetch_at = Some(now.to_string());
                store.competitor_watchlist[position].updated_at = now.to_string();
            }
            Ok::<(), String>(())
        })?;
    }

    let scanned = posts.iter().take(effective_limit).count();
    Ok(json!({
        "success": true,
        "profileUrl": url,
        // 用 effective_limit 而不是原始 limit：B 站会合并合集/收藏夹补全的条目，
        // 否则会出现「扫描 30 条，新增 190 条」这种自相矛盾的提示。
        "scanned": scanned,
        "created": created,
        "updated": updated,
        // 落库诊断：抓到的总条数 / 域名不符丢弃 / 已存在（更新）条数
        "scrapedPosts": scraped_posts,
        "droppedDomain": dropped_domain,
        "updatedExisting": updated_existing,
        "capturedVia": "chrome-cdp",
        // 是否已翻到主页底部（历史抓完的判据）
        "historyExhausted": exhausted,
        // 滚动诊断（首屏就停 / 滚错容器 / 页面不给更多）
        "scrollDiag": scroll_diag,
        // 主页声明的作品总数（进度条分母；未知为 0）
        "profileTotal": profile_total,
        "note": if extras_added > 0 {
            format!("已扫描 {scanned} 条（含合集/收藏夹补全 {extras_added} 条）。")
        } else {
            String::new()
        },
    }))
}
