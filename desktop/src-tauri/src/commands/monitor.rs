use crate::persistence::{with_store, with_store_mut};
use crate::store::topic_center as topic_center_store;
use crate::tools::headless_browser;
use crate::tools::web_access;
use crate::ai_model_manager::AiModelManager;
use crate::official_support::invoke_structured_chat_by_protocol;
use crate::{app_brand_display_name, make_id, now_i64, AppState, AppStore};
use crate::ffmpeg_program;
use crate::ffprobe_program;
use crate::workspace_root;
use crate::platform::Platform;
use crate::store::types::{MonitoredPostRecord, PostVideoAnalysisRecord};
use crate::commands::media_transcribe::execute_media_transcribe;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use chrono::{Local, TimeZone, Utc};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::time::interval;

/// 对标博主监控通道分发。
///
/// 支持的通道：
/// - monitor:watch-add    添加一个被监控博主
/// - monitor:watch-list   列出监控列表
/// - monitor:watch-remove 删除一个监控项
/// - monitor:fetch        抓取博主主页或单篇链接，落库到 monitored_posts
/// - monitor:post-list    列出已抓取到的帖子（可按 watchId / spikeOnly 过滤）
/// - monitor:inject-topic 把监控帖子注入选题库（topic_center，method=competitor_monitor）
/// - monitor:chrome-status   查询本机 Chrome + 调试端口状态
/// - monitor:chrome-launch   为指定平台拉起 Chrome 登录页（独立 profile）
/// - monitor:chrome-test     通过 CDP 测试抓取一段 JS 评估结果（验证通道）
/// - monitor:clear           清空全部「对标博主」与「全部动态」数据（危险操作，UI 需二次确认）
/// - monitor:post-detail     本地查看单条动态详情（CDP 抓正文并缓存，尽量不跳外部平台）
pub fn handle_monitor_channel(
    app: &AppHandle,
    state: &State<'_, AppState>,
    channel: &str,
    payload: &Value,
) -> Option<Result<Value, String>> {
    if !channel.starts_with("monitor:") {
        return None;
    }
    Some(
        (|| match channel {
            "monitor:watch-add" => watch_add(state, payload),
            "monitor:watch-list" => watch_list(state, payload),
            "monitor:watch-remove" => watch_remove(state, payload),
            "monitor:fetch" => fetch_monitor(state, payload, None, false),
            "monitor:post-list" => post_list(state, payload),
            "monitor:inject-topic" => inject_topic(state, payload),
            "monitor:run-due" => run_due_fetches(state, now_i64()),
            "monitor:chrome-status" => chrome_status(app, payload),
            "monitor:chrome-launch" => chrome_launch(app, payload),
            "monitor:chrome-test" => chrome_test(app, payload),
            "monitor:chrome-shutdown" => chrome_shutdown(),
            "monitor:clear" => clear_data(state),
            "monitor:post-remove" => post_remove(state, payload),
            "monitor:clear-posts" => clear_posts(state),
            "monitor:post-detail" => post_detail(state, payload),
            "monitor:retro" => retro(state, payload),
            "monitor:set-scan-limit" => set_scan_limit(state, payload),
            "monitor:get-scan-limit" => get_scan_limit(state),
            "monitor:set-spike-threshold" => set_spike_threshold(state, payload),
            "monitor:get-spike-threshold" => get_spike_threshold(state),
            "monitor:deconstruct" => deconstruct(state, payload),
            "monitor:decon-list" => decon_list(state, payload),
            "monitor:export-post" => export_post_command(state, payload),
            "monitor:get-export-root" => get_export_root(state),
            "monitor:set-export-root" => set_export_root(state, payload),
            "monitor:pick-export-root" => pick_export_root(),
            "monitor:get-obsidian-vault" => get_obsidian_vault(state),
            "monitor:set-obsidian-vault" => set_obsidian_vault(state, payload),
            "monitor:pick-obsidian-vault" => pick_obsidian_vault(),
            "monitor:download-video" => download_video(app, state, payload),
            "monitor:analyze-video" => analyze_video(app, state, payload),
            "monitor:transcribe-video" => transcribe_video_only(app, state, payload),
            "monitor:patch-audio-batch" => patch_audio_batch(app.clone(), payload),
            "monitor:video-analysis" => video_analysis(state, payload),
            // 手动触发「每日 00:00 批量预取」：列表(URL 去重)→补全正文→视频下载+转写。
            // 主要用于调试（不必等到半夜），正式触发由 start_monitor_scheduler 的 00:00 循环驱动。
            "monitor:run-daily" => run_daily_prefetch(app.clone()),
            // 补历史：深翻主页把更早的历史帖一次性落库（手动触发）
            "monitor:backfill-history" => run_history_backfill(app.clone(), payload),
            _ => Err(format!(
                "{} host does not recognize channel `{channel}`.",
                app_brand_display_name()
            )),
        })(),
    )
}

fn chrome_status(_app: &AppHandle, payload: &Value) -> Result<Value, String> {
    let platform = payload
        .get("platform")
        .and_then(Value::as_str)
        .unwrap_or("default");
    let path = headless_browser::detect_chrome_path();
    let port = headless_browser::active_chrome_port(platform);
    let ready_platforms = headless_browser::ready_platforms();
    Ok(json!({
        "chromeInstalled": path.is_some(),
        "chromePath": path.as_ref().map(|p| p.display().to_string()),
        "platform": platform,
        "debugPort": port,
        "debugReady": port != 0,
        "readyPlatforms": ready_platforms,
    }))
}

fn chrome_launch(app: &AppHandle, payload: &Value) -> Result<Value, String> {
    let platform = payload
        .get("platform")
        .and_then(Value::as_str)
        .unwrap_or("default");
    let login_url = payload
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or("https://www.xiaohongshu.com");
    let chrome_path = headless_browser::detect_chrome_path()
        .ok_or_else(|| "未检测到 Chrome / Edge / Brave，请先安装 Google Chrome".to_string())?;
    let profile = headless_browser::profile_dir_for(app, platform)?;
    let port = headless_browser::launch_chrome_for_login(&chrome_path, &profile, platform, login_url)?;
    Ok(json!({
        "success": true,
        "platform": platform,
        "debugPort": port,
        "profileDir": profile.display().to_string(),
        "note": "已为你打开登录页，请在弹出的 Chrome 中完成登录。每个平台有独立 profile（cookie 隔离）。Beav 不会保存你的密码，只复用这次登录的 cookie。",
    }))
}

fn chrome_test(_app: &AppHandle, payload: &Value) -> Result<Value, String> {
    let platform = payload
        .get("platform")
        .and_then(Value::as_str)
        .unwrap_or("default");
    // 同步壳：进入 tokio runtime 执行 CDP 测试（ipc_invoke 已在 spawn_blocking 内，安全）。
    let port = headless_browser::active_chrome_port(platform);
    if port == 0 {
        return Err(format!(
            "「{platform}」的 Chrome 调试端口未启动，请先在该平台点「打开 Chrome 登录」完成登录"
        ));
    }
    let target_url = payload
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or("https://www.xiaohongshu.com");
    // 默认测试脚本：取页面标题，并用页面文本粗判是否仍处于「未登录」态（登录页通常有「登录/扫码」字样）。
    let expression = payload
        .get("expression")
        .and_then(Value::as_str)
        .unwrap_or(
            r#"
            (() => {
                const t = document.title;
                const txt = (document.body && document.body.innerText) || '';
                const loggedOut = /登录|扫码|sign[\s_-]?in|未登录|扫码登录/i.test(txt)
                    && !/(个人主页|消息|创作中心|creator|我的主页)/i.test(txt);
                return { title: t, loggedOut: loggedOut };
            })()
            "#,
        );
    let port_arg = port as u16;
    let result = tauri::async_runtime::block_on(async move {
        let ws_url = headless_browser::ws_url_for_port(port_arg).await?;
        headless_browser::navigate_and_eval(&ws_url, target_url, expression).await
    });
    result.map(|value| json!({ "success": true, "platform": platform, "result": value }))
}

fn chrome_shutdown() -> Result<Value, String> {
    headless_browser::shutdown_chrome();
    Ok(json!({ "success": true, "note": "已关闭登录态 Chrome 进程，下次抓取需重新登录" }))
}

fn str_field(payload: &Value, key: &str) -> Option<String> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// 落一条互动快照（供「数据复盘」画走势）。
///
/// 关键护栏：三项互动数加起来必须为正数才落。web 抓取路径（fetch_monitor）拿不到点赞数，
/// 写的是全零；若不拦，这些 0 值会混进按天聚合，把走势线「砸」出假下跌。只有 CDP 列表 /
/// 详情页产出的真实互动才值得入历史。
fn push_snapshot(
    store: &mut crate::AppStore,
    post_id: &str,
    platform: &str,
    author: &str,
    likes: i64,
    comments: i64,
    collects: i64,
    captured_at: i64,
) {
    if likes + comments + collects <= 0 {
        return;
    }
    store.post_snapshots.push(crate::PostSnapshotRecord {
        id: make_id("snap"),
        post_id: post_id.to_string(),
        platform: platform.to_string(),
        author: author.to_string(),
        likes,
        comments,
        collects,
        captured_at,
    });
}

fn watch_add(state: &State<'_, AppState>, payload: &Value) -> Result<Value, String> {
    let name = str_field(payload, "name").ok_or_else(|| "缺少博主名称 name".to_string())?;
    let now = now_i64();
    let record = crate::CompetitorWatchRecord {
        id: make_id("watch"),
        name,
        platform: str_field(payload, "platform").unwrap_or_default(),
        profile_url: str_field(payload, "profileUrl")
            .or_else(|| str_field(payload, "profile_url"))
            .map(|url| crate::tools::headless_browser::normalize_profile_url(&url)),
        handle: str_field(payload, "handle"),
        note: str_field(payload, "note"),
        enabled: payload
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        interval_hours: payload
            .get("intervalHours")
            .or_else(|| payload.get("interval_hours"))
            .and_then(Value::as_i64)
            .unwrap_or(24),
        last_fetch_at: None,
        history_backfilled_at: None,
        created_at: now.to_string(),
        updated_at: now.to_string(),
    };
    with_store_mut(state, |store| {
        store.competitor_watchlist.push(record.clone());
        Ok(json!({ "success": true, "item": record }))
    })
}

fn watch_list(state: &State<'_, AppState>, payload: &Value) -> Result<Value, String> {
    let only_enabled = payload
        .get("enabledOnly")
        .or_else(|| payload.get("enabled_only"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    with_store_mut(state, |store| {
        // 自愈迁移：把 v1（有 bug 的版本）打上的「已补历史」标记作废 ——
        // 那次标记是在深翻一条都没抓到时打的（如 Chrome 未连接），属于误标。
        // 清掉后前端会重新显示「历史未补齐」，每晚的自动补历史也会重新尝试。
        let mut cleaned = 0i64;
        for watch in store.competitor_watchlist.iter_mut() {
            if watch.history_backfilled_at.is_some()
                && !history_backfill_valid(watch.history_backfilled_at.as_deref())
            {
                watch.history_backfilled_at = None;
                cleaned += 1;
            }
        }
        if cleaned > 0 {
            println!("[monitor] cleaned {cleaned} legacy history-backfill flags");
        }
        let mut items: Vec<_> = store.competitor_watchlist.iter().cloned().collect();
        if only_enabled {
            items.retain(|item| item.enabled);
        }
        items.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        // 每个监控的「已收录 N 篇」由后端派发：前端 posts 数组是按选中 watchId 拉取的，
        // 未选中的监控用它过滤恒为 0（聚焦才显示正确数量）。过滤语义与 post_list 一致。
        let items: Vec<Value> = items
            .iter()
            .map(|watch| {
                let count = store
                    .monitored_posts
                    .iter()
                    .filter(|p| p.watch_id.as_deref() == Some(watch.id.as_str()))
                    .count();
                let mut v = serde_json::to_value(watch).unwrap_or(Value::Null);
                if let Some(obj) = v.as_object_mut() {
                    obj.insert("postCount".to_string(), json!(count));
                }
                v
            })
            .collect();
        Ok(json!(items))
    })
}

fn watch_remove(state: &State<'_, AppState>, payload: &Value) -> Result<Value, String> {
    let id = str_field(payload, "id").ok_or_else(|| "缺少 id".to_string())?;
    with_store_mut(state, |store| {
        let before = store.competitor_watchlist.len();
        store.competitor_watchlist.retain(|item| item.id != id);
        if store.competitor_watchlist.len() == before {
            return Err("监控项不存在".to_string());
        }
        Ok(json!({ "success": true }))
    })
}

/// 清空全部「对标博主」与「全部动态」数据。
///
/// 危险操作，仅由 UI 二次确认后调用。`with_store_mut` 会触发 `schedule_store_persist`，
/// 所以清空结果会落盘（beav-state.json 里两个数组变空），重启后依然为空。
fn clear_data(state: &State<'_, AppState>) -> Result<Value, String> {
    with_store_mut(state, |store| {
        let removed_watchlist = store.competitor_watchlist.len();
        let removed_posts = store.monitored_posts.len();
        store.competitor_watchlist.clear();
        store.monitored_posts.clear();
        Ok(json!({
            "success": true,
            "removedWatchlist": removed_watchlist,
            "removedPosts": removed_posts,
        }))
    })
}

/// 删除单条动态（帖子）。按 id 从 `monitored_posts` 中移除并落盘。
fn post_remove(state: &State<'_, AppState>, payload: &Value) -> Result<Value, String> {
    let id = str_field(payload, "id").ok_or_else(|| "缺少 id".to_string())?;
    with_store_mut(state, |store| {
        let before = store.monitored_posts.len();
        store.monitored_posts.retain(|item| item.id != id);
        if store.monitored_posts.len() == before {
            return Err("动态不存在".to_string());
        }
        Ok(json!({ "success": true }))
    })
}

/// 清空全部动态（仅 `monitored_posts`，不动对标博主列表）。
fn clear_posts(state: &State<'_, AppState>) -> Result<Value, String> {
    with_store_mut(state, |store| {
        let removed = store.monitored_posts.len();
        store.monitored_posts.clear();
        Ok(json!({ "success": true, "removedPosts": removed }))
    })
}

/// 本地查看单条动态详情。
///
/// 优先读缓存（`content` 非空直接返回，不联网）；`refresh=true` 或缓存为空时，用该平台
/// 已登录的 Chrome（CDP）打开帖子详情页抓正文/图片/互动数据，写回记录（excerpt 取正文前 80 字）。
/// 平台未登录 / 抓取失败都**不报错**，返回原记录 + `note` 说明，前端照常本地展示已有信息。
fn post_detail(state: &State<'_, AppState>, payload: &Value) -> Result<Value, String> {
    let id = str_field(payload, "id").ok_or_else(|| "缺少 id".to_string())?;
    let refresh = payload
        .get("refresh")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let record = with_store_mut(state, |store| {
        Ok(store
            .monitored_posts
            .iter()
            .find(|item| item.id == id)
            .cloned())
    })?
    .ok_or_else(|| "动态不存在".to_string())?;

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
                    item.content = content.clone();
                    item.excerpt = Some(content.chars().take(80).collect::<String>());
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
                if let Some(author) = detail
                    .get("author")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    item.author = author.to_string();
                }
                if let Some(date) = detail
                    .get("date")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    item.published_at = Some(date.to_string());
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

fn host_of(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(|host| host.to_string()))
}

/// 从 B 站视频链接里取 BV 号（`…/video/BV1xx411c7mD/?p=1` → `BV1xx411c7mD`）。
fn bv_of_url(url: &str) -> Option<String> {
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
fn same_registrable_domain(a: &str, b: &str) -> bool {
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

fn platform_of(url: &str) -> String {
    let host = host_of(url).unwrap_or_default();
    Platform::from_host(&host)
        .unwrap_or(Platform::Web)
        .as_str()
        .to_string()
}

/// 单次抓取条数：**固定值，不再对用户开放配置**。
///
/// 之前由前端输入框 + `store.monitor_scan_limit` 配置，但实际只有一个合理量级（每天增量窗口），
/// 让用户填反而制造「抓太少漏帖 / 抓太多滑动超时」两类问题。这里收口成常量：
/// `resolve_scan_limit` 忽略 payload 与持久化值，`get_scan_limit` 也只回这个数（前端据此显示）。
/// 补历史的深翻窗口另由 `BACKFILL_MAX` 控制，不受影响。
const SCAN_LIMIT: usize = 30;

/// 解析「单次抓取条数」：恒定返回 `SCAN_LIMIT`，忽略请求 payload 与历史持久化配置。
///
/// 保留函数签名是为了让所有调用点（fetch_monitor / adhoc 抓取 / 每日预取）无需改动，
/// 也避免旧前端传 `limit` 时报错。
fn resolve_scan_limit(_state: &State<'_, AppState>, _payload: &Value) -> usize {
    SCAN_LIMIT
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
fn compute_spike(
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
fn resolve_spike_threshold(state: &State<'_, AppState>) -> f64 {
    const DEFAULT_THRESHOLD: f64 = 3.0;
    with_store(state, |store| Ok(store.monitor_spike_threshold))
        .ok()
        .filter(|value| *value > 0.0)
        .unwrap_or(DEFAULT_THRESHOLD)
}

fn fetch_monitor(
    state: &State<'_, AppState>,
    payload: &Value,
    app: Option<AppHandle>,
    emit_progress: bool,
) -> Result<Value, String> {
    let watch: Option<crate::CompetitorWatchRecord> =
        if let Some(id) = str_field(payload, "watchId").or_else(|| str_field(payload, "watch_id")) {
            with_store_mut(state, |store| {
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
        let existing: Option<crate::MonitoredPostRecord> =
            with_store_mut(state, |store| {
                Ok(store
                    .monitored_posts
                    .iter()
                    .find(|item| &item.post_url == candidate)
                    .cloned())
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
            watch_id: watch_id.clone(),
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
            content: if text.trim().is_empty() {
                existing
                    .as_ref()
                    .map(|previous| previous.content.clone())
                    .unwrap_or_default()
            } else {
                text
            },
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
fn resolve_adhoc_author(port: u16, url: &str, posts: &[Value], platform: &str) -> String {
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
fn fetch_monitor_via_cdp(
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

        let existing: Option<crate::MonitoredPostRecord> = with_store_mut(state, |store| {
            Ok(store
                .monitored_posts
                .iter()
                .find(|item| item.post_url.split('?').next().unwrap_or(&item.post_url) == post_key)
                .cloned())
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
        let record = crate::MonitoredPostRecord {
            id: existing
                .as_ref()
                .map(|previous| previous.id.clone())
                .unwrap_or_else(|| make_id("mpost")),
            watch_id: watch.map(|item| item.id.clone()),
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

/// 设置单次抓取条数（持久化到 store，重启后保留）。范围 [1, 200]，越界回落默认 12。
/// 兼容旧调用：抓取条数已固定为 `SCAN_LIMIT`，这里不再落库（忽略传入值）。
fn set_scan_limit(_state: &State<'_, AppState>, _payload: &Value) -> Result<Value, String> {
    Ok(json!({
        "success": true,
        "limit": SCAN_LIMIT,
        "note": "单次抓取条数已固定为 30，不再支持自定义",
    }))
}

/// 读取当前抓取条数：恒定返回 `SCAN_LIMIT`，供前端显示静态提示。
fn get_scan_limit(_state: &State<'_, AppState>) -> Result<Value, String> {
    Ok(json!({ "success": true, "limit": SCAN_LIMIT, "fixed": true }))
}

/// 配置异常倍数阈值（持久化到 store）。合理区间 1.0~20.0，越界/缺失回落 3.0。
fn set_spike_threshold(state: &State<'_, AppState>, payload: &Value) -> Result<Value, String> {
    let raw = payload
        .get("threshold")
        .and_then(Value::as_f64)
        .ok_or_else(|| "缺少 threshold 参数".to_string())?;
    let threshold = if raw < 1.0 || raw > 20.0 { 3.0 } else { raw };
    with_store_mut(state, |store| {
        store.monitor_spike_threshold = threshold;
        Ok(json!({ "success": true, "threshold": threshold }))
    })
}

/// 读取当前异常倍数阈值（0 / 缺失回落默认 3.0，便于前端初始化）。
fn get_spike_threshold(state: &State<'_, AppState>) -> Result<Value, String> {
    with_store_mut(state, |store| {
        let threshold = if store.monitor_spike_threshold <= 0.0 {
            3.0
        } else {
            store.monitor_spike_threshold
        };
        Ok(json!({ "success": true, "threshold": threshold }))
    })
}

fn post_list(state: &State<'_, AppState>, payload: &Value) -> Result<Value, String> {
    let watch_id = str_field(payload, "watchId").or_else(|| str_field(payload, "watch_id"));
    let spike_only = payload
        .get("spikeOnly")
        .or_else(|| payload.get("spike_only"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let limit = payload
        .get("limit")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok());
    with_store_mut(state, |store| {
        let mut items: Vec<_> = store.monitored_posts.iter().cloned().collect();
        if let Some(id) = &watch_id {
            items.retain(|item| item.watch_id.as_deref() == Some(id));
        }
        if spike_only {
            items.retain(|item| item.spike);
        }
        items.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| right.created_at.cmp(&left.created_at))
        });
        if let Some(max) = limit {
            items.truncate(max.max(1));
        }
        Ok(json!(items))
    })
}

/// 数据复盘聚合通道（monitor:retro）。
///
/// 输入：`author`（可选，按博主）、`platform`（可选）、`days`（默认 30）。
/// 输出：
/// - `authors`：库内全部去重作者，供前端下拉选择；
/// - `series`：按天分桶的互动走势（同一 post 同日取最新快照，再跨 post 求和，避免重复累加），
///   每项 `{ ts(当天零点毫秒), likes, comments, collects, total, posts }`；
/// - `topPosts`：当前库内（按筛选）互动总量 Top 10；
/// - `summary`：跟踪帖数 / 快照数 / 活跃天数 / 总互动 / 篇均 / 最高赞。
///
/// 走势依赖 `post_snapshots` 历史表（抓取时按护栏写入真实互动）。库里在改版前已存在的旧帖
/// 没有快照，故首次使用需重新抓取博主 / 打开详情来积累数据点。
fn retro(state: &State<'_, AppState>, payload: &Value) -> Result<Value, String> {
    let author = str_field(payload, "author").unwrap_or_default();
    let platform = str_field(payload, "platform").unwrap_or_default();
    let days = payload
        .get("days")
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .unwrap_or(30) as i64;
    let now = now_i64();
    let cutoff = now - days * 86_400_000;
    let author_filter = !author.is_empty();
    let platform_filter = !platform.is_empty();

    use std::collections::{HashMap, HashSet};

    with_store_mut(state, |store| {
        // 作者清单（取全部库内作者，不受当前筛选影响，给前端下拉用）
        let mut authors: Vec<String> = store
            .monitored_posts
            .iter()
            .map(|item| item.author.clone())
            .filter(|value| !value.is_empty())
            .collect();
        authors.sort();
        authors.dedup();

        // 过滤快照（时间窗 + 作者/平台）
        let snaps: Vec<&crate::PostSnapshotRecord> = store
            .post_snapshots
            .iter()
            .filter(|s| {
                s.captured_at >= cutoff
                    && (!author_filter || s.author == author)
                    && (!platform_filter || s.platform == platform)
            })
            .collect();

        // 按 (天, post_id) 取每组最新快照，避免同一天多次抓取重复累加
        let mut latest: HashMap<(i64, String), &crate::PostSnapshotRecord> = HashMap::new();
        for s in &snaps {
            let day = s.captured_at / 86_400_000;
            let key = (day, s.post_id.clone());
            let keep = match latest.get(&key) {
                Some(prev) => s.captured_at > prev.captured_at,
                None => true,
            };
            if keep {
                latest.insert(key, s);
            }
        }

        // 按天聚合（跨 post 求和）
        let mut by_day: HashMap<i64, (i64, i64, i64, i64, HashSet<String>)> = HashMap::new();
        for (_key, s) in &latest {
            let day = s.captured_at / 86_400_000;
            let entry = by_day.entry(day).or_insert((0, 0, 0, 0, HashSet::new()));
            entry.0 += s.likes;
            entry.1 += s.comments;
            entry.2 += s.collects;
            entry.3 += s.likes + s.comments + s.collects;
            entry.4.insert(s.post_id.clone());
        }

        let mut series: Vec<Value> = by_day
            .iter()
            .map(|(day, (likes, comments, collects, total, posts))| {
                json!({
                    "ts": day * 86_400_000,
                    "likes": likes,
                    "comments": comments,
                    "collects": collects,
                    "total": total,
                    "posts": posts.len(),
                })
            })
            .collect();
        series.sort_by(|left, right| {
            left.get("ts")
                .and_then(Value::as_i64)
                .cmp(&right.get("ts").and_then(Value::as_i64))
        });

        // Top 帖子：当前库内（按筛选）按互动总量排序取前 10
        let mut posts: Vec<_> = store
            .monitored_posts
            .iter()
            .filter(|item| {
                (!author_filter || item.author == author)
                    && (!platform_filter || item.platform == platform)
            })
            .cloned()
            .collect();
        posts.sort_by(|left, right| {
            let left_total = left.likes + left.comments + left.collects;
            let right_total = right.likes + right.comments + right.collects;
            right_total.cmp(&left_total)
        });
        let top_posts: Vec<Value> = posts
            .iter()
            .take(10)
            .map(|item| {
                json!({
                    "id": item.id,
                    "title": item.title,
                    "platform": item.platform,
                    "author": item.author,
                    "likes": item.likes,
                    "comments": item.comments,
                    "collects": item.collects,
                    "total": item.likes + item.comments + item.collects,
                    "url": item.post_url,
                    "mediaType": item.media_type,
                    "coverUrl": item.cover_url,
                    "videoLocalPath": item.video_local_path,
                    "videoPreviewPath": item.video_preview_path,
                    "videoStatus": item.video_status,
                    "videoSize": item.video_size,
                    "videoDownloadedAt": item.video_downloaded_at,
                    "videoError": item.video_error,
                    "deconstructed": item.deconstructed,
                    "updatedAt": item.updated_at,
                })
            })
            .collect();

        let tracked = posts.len() as i64;
        let snapshots = snaps.len() as i64;
        let active_days = by_day.len() as i64;
        let total_engagement: i64 = posts.iter().map(|p| p.likes + p.comments + p.collects).sum();
        let avg_per_post = if tracked > 0 { total_engagement / tracked } else { 0 };
        let top_likes = posts.iter().map(|p| p.likes).max().unwrap_or(0);

        Ok(json!({
            "success": true,
            "authors": authors,
            "days": days,
            "author": author,
            "platform": platform,
            "series": series,
            "topPosts": top_posts,
            "summary": {
                "trackedPosts": tracked,
                "snapshots": snapshots,
                "activeDays": active_days,
                "totalEngagement": total_engagement,
                "avgPerPost": avg_per_post,
                "topLikes": top_likes,
            },
        }))
    })
}

fn inject_topic(state: &State<'_, AppState>, payload: &Value) -> Result<Value, String> {
    let ids: Vec<String> = match payload.get("ids").and_then(Value::as_array) {
        Some(array) => array
            .iter()
            .filter_map(|value| value.as_str().map(String::from))
            .collect(),
        None => vec![str_field(payload, "id").ok_or_else(|| "缺少 id 或 ids".to_string())?],
    };
    let now = now_i64();
    let mut injected: Vec<Value> = Vec::new();
    with_store_mut(state, |store| {
        for id in &ids {
            let Some(post) = store.monitored_posts.iter().find(|item| &item.id == id).cloned()
            else {
                continue;
            };
            let topic_id = make_id("topic");
            let candidate = json!({
                "topic_name": post.title.clone(),
                "content_direction": post.excerpt.clone().unwrap_or_default(),
                "source_evidence": format!("对标监控 · {} · {}", post.platform.clone(), post.author.clone()),
                "method": "competitor_monitor",
                "createdBy": "monitor",
                "sourceRefs": [{
                    "url": post.post_url,
                    "platform": post.platform,
                    "author": post.author,
                    "title": post.title,
                    "spike": post.spike,
                }],
            });
            let record = topic_center_store::topic_record_from_payload(
                &candidate,
                topic_id,
                "monitor",
                now,
            );
            if let Some(position) = store.topic_center.iter().position(|item| item.id == record.id) {
                store.topic_center[position] = record.clone();
            } else {
                store.topic_center.push(record.clone());
            }
            topic_center_store::upsert_wander_history_compat(store, &record);
            if let Some(position) = store.monitored_posts.iter().position(|item| &item.id == id) {
                store.monitored_posts[position].topic_injected = true;
                store.monitored_posts[position].updated_at = now.to_string();
            }
            injected.push(json!(record));
        }
        Ok(json!({ "success": true, "count": injected.len(), "items": injected }))
    })
}

/// 从模型输出里稳健抽取 JSON 对象（兼容 ```json 围栏与前后多余文本）。
fn extract_json_object(text: &str) -> Option<Value> {
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
fn strip_code_fence(s: &str) -> &str {
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
fn string_array_field(value: &Value, key: &str) -> Option<Vec<String>> {
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
fn deconstruct(state: &State<'_, AppState>, payload: &Value) -> Result<Value, String> {
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
    if post.content.trim().is_empty() {
        return Err("该动态暂无正文，请先点开详情抓取正文后再拆解".to_string());
    }

    // 解析 AI 配置（用户自配或官方）
    let settings = with_store(state, |store| Ok(store.settings.clone()))?;
    let config = AiModelManager::resolve_chat_config(&settings, None)
        .ok_or_else(|| "未配置可用的 AI 模型，请在设置中配置后重试".to_string())?;

    // 构建 prompt：系统负责角色与 schema，用户携带原文
    let system_prompt = "你是资深内容拆解专家，擅长把爆款文案拆成可复用的方法论要素。\
        只输出 JSON，不要任何解释性文字。JSON 字段：\
        topicAngle（选题角度：为什么值得写，1-2 句）、\
        copyStructure（文案结构：开头如何钩人、中间如何递进、结尾如何转化）、\
        opening（开场白 / 前 3 秒钩子，可引用原文或改写）、\
        titlePatterns（标题句式数组，可多条）、\
        hooks（金句 / 钩子数组，可多条）、\
        reusableTakeaway（可复用的方法论点，1-2 句）。";
    let user_prompt = format!(
        "请拆解以下爆款文案：\n标题：{}\n作者：{}\n正文：\n{}\n",
        post.title, post.author, post.content
    );

    let raw = invoke_structured_chat_by_protocol(
        &config.protocol,
        &config.base_url,
        config.api_key.as_deref(),
        &config.model_name,
        system_prompt,
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
        raw_content: post.content.clone(),
        source_evidence: format!("对标监控 · {} · {}", post.platform, post.author),
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
fn decon_list(state: &State<'_, AppState>, payload: &Value) -> Result<Value, String> {
    let author = str_field(payload, "author").unwrap_or_default();
    let platform = str_field(payload, "platform").unwrap_or_default();
    with_store(state, |store| {
        let mut items: Vec<Value> = store
            .post_deconstructions
            .iter()
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

// —— 竞品视频抓取闭环（下载 + 内容分析）——
// 下载目录：应用数据根下的 monitor-videos/（视频文件与抽帧都落这里）。
fn monitor_videos_dir(state: &State<'_, AppState>) -> Result<PathBuf, String> {
    let base = workspace_root(state)?;
    let dir = base.join("monitor-videos");
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建视频目录失败：{e}"))?;
    Ok(dir)
}

/// 竞品图片本地缓存目录：`<workspace>/monitor-images/<post_id>/`。
fn monitor_images_dir(state: &State<'_, AppState>, post_id: &str) -> Result<PathBuf, String> {
    let dir = workspace_root(state)?.join("monitor-images").join(post_id);
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建图片缓存目录失败：{e}"))?;
    Ok(dir)
}

/// 单张远程图片落盘（带 Referer/UA）。成功返回本地路径，失败返回 `None`。
///
/// 为什么必须本地化：B 站 `i0.hdslb.com`、小红书 `sns-webpic*.xhscdn.com` 都校验 Referer，
/// WebView2 里 `<img src="https://...">` 不带 Referer → 403 → 详情页/导出全是裂图。
fn cache_remote_image(
    client: &reqwest::blocking::Client,
    dir: &Path,
    url: &str,
    name: &str,
    referer: &str,
    ua: &str,
) -> Option<String> {
    let ext = url
        .split('?')
        .next()
        .unwrap_or("")
        .rsplit('.')
        .next()
        .map(|e| e.to_ascii_lowercase())
        .filter(|e| matches!(e.as_str(), "jpg" | "jpeg" | "png" | "webp" | "gif"))
        .unwrap_or_else(|| "jpg".to_string());
    let target = dir.join(format!("{name}.{ext}"));
    if target.is_file() && std::fs::metadata(&target).map(|m| m.len() > 0).unwrap_or(false) {
        return Some(target.display().to_string());
    }
    let resp = client
        .get(url)
        .header("Referer", referer)
        .header("User-Agent", ua)
        .send()
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let bytes = resp.bytes().ok()?;
    if bytes.is_empty() || std::fs::write(&target, &bytes).is_err() {
        return None;
    }
    Some(target.display().to_string())
}

/// 批量本地化帖子图片；下载失败的条目保留原 URL（至少不丢数据）。
fn cache_post_images(
    state: &State<'_, AppState>,
    post_id: &str,
    platform: &str,
    urls: &[String],
    cover: Option<&str>,
) -> (Vec<String>, Option<String>) {
    let dir = match monitor_images_dir(state, post_id) {
        Ok(dir) => dir,
        Err(_) => return (urls.to_vec(), cover.map(ToString::to_string)),
    };
    let referer = Platform::parse(platform)
        .map(|p| p.home_url())
        .unwrap_or("https://www.bilibili.com");
    let ua = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
    let client = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
    {
        Ok(client) => client,
        Err(_) => return (urls.to_vec(), cover.map(ToString::to_string)),
    };

    let mut local: Vec<String> = Vec::with_capacity(urls.len());
    for (idx, url) in urls.iter().take(12).enumerate() {
        // 协议相对 URL（`//i0.hdslb.com/...`）必须先补 https，否则会被误判成「已是本地路径」，
        // 原样存进记录 → WebView 当成相对路径 → 裂图。
        let normalized = if url.starts_with("//") {
            format!("https:{url}")
        } else {
            url.trim().to_string()
        };
        if !normalized.starts_with("http") {
            // 已经是本地文件才保留；否则（空串/非法）直接丢
            if Path::new(&normalized).is_file() {
                local.push(normalized);
            }
            continue;
        }
        let name = format!("img_{:02}", idx + 1);
        match cache_remote_image(&client, &dir, &normalized, &name, referer, ua) {
            Some(path) => local.push(path),
            // 下载失败就**丢弃**：远程直连对这两家 CDN 必然 403，留着只会显示裂图。
            None => println!("[monitor] cache image failed, dropped: {normalized}"),
        }
    }

    let cover_local = cover.and_then(|c| {
        let normalized = if c.starts_with("//") {
            format!("https:{c}")
        } else {
            c.trim().to_string()
        };
        if !normalized.starts_with("http") {
            return Path::new(&normalized).is_file().then_some(normalized);
        }
        cache_remote_image(&client, &dir, &normalized, "cover", referer, ua)
    });
    (local, cover_local)
}

/// 在帖子页抽取视频直链的 JS 模板。
///
/// `__XHS__` / `__DY__` 由 Rust 替换为 `true`/`false` 后注入平台分支；其余为 JS 原样执行。
///
/// # 设计要点（2026-10-21 真机联调后重写）
///
/// 首版只赌 `window.__INITIAL_STATE__` / `_ROUTER_DATA` 的固定字段，实机全部落空
/// （`vtags:2, found:0`）：抖音已改用 MSE 流式播放，`<video>.src` 是 `blob:`，页面全局对象里
/// 也读不到裸直链。因此改为**不依赖站点内部结构**的广撒网：
/// 1. `performance.getEntriesByType('resource')` —— 浏览器记录的真实网络请求，平台再怎么改版都还在，
///    且带完整签名参数。这是主力手段。
/// 2. 深层遍历 `window` 里的已知状态容器（只作为补充）。
/// 3. `<video>` / `<source>` 的 `currentSrc`、`src`（含 blob: 之外的直链）。
///
/// 最后按「视频特征打分」排序：命中 `.mp4` / `.m3u8` / `video/tos` / `aweme` / `playwm` 等特征越强的排前面，
/// 优先选无水印（`wm=`）的高清地址。
const VIDEO_EXTRACT_EXPRESSION: &str = r#"
(() => {
    const XHS = __XHS__;
    const DY = __DY__;
    const candidates = [];
    // 抖音网页版播放器走 DASH：视频轨与音频轨是**两个独立直链**。`video.play_addr` 系列
    // 只含视频（下载下来 ffprobe 查不到 audio stream），音频在 `music.play_url` / `.../audio/tos/...`。
    // 不识别音频候选 → 下载到的 mp4 永远没声音 → 语音转写必然失败（历史 bug）。
    const looksAudio = (u) => {
        if (/\.(?:mp3|m4a|aac|wav|ogg)(?:[?#]|$)/i.test(u)) return true;
        if (/mime_type=[^&]*audio/i.test(u)) return true;
        if (/\/audio\/|\/music\/|media-audio|[.-]audio[.-]/i.test(u)) return true;
        return false;
    };
    const push = (u, src) => {
        if (typeof u !== 'string') return;
        const t = u.trim();
        if (!t) return;
        if (!/^https?:\/\//i.test(t)) return;
        // 去掉 blob: / data: / 明显的图片、静态资源
        if (/^blob:|^data:/i.test(t)) return;
        if (/\.(?:jpg|jpeg|png|gif|webp|avif|svg|css|js|woff2?)(?:[?#]|$)/i.test(t)) return;
        candidates.push({ url: t, src: src || 'unknown', audio: looksAudio(t) });
    };

    // —— 1) 主力：performance resource timing（真实网络请求，含完整签名）——
    let perfTotal = 0;
    try {
        const entries = performance.getEntriesByType('resource') || [];
        perfTotal = entries.length;
        for (const e of entries) push(e.name, 'perf');
    } catch (e) {}

    // —— 2) 补充：平台状态容器深遍历 ——
    const walk = (o, depth) => {
        if (!o || typeof o !== 'object' || depth > 6) return;
        for (const key in o) {
            let v;
            try { v = o[key]; } catch (e) { continue; }
            if (typeof v === 'string') {
                const k = key.toLowerCase();
                // 只看名字里带"播放/流"语义的键，避免把头像、封面等图片地址当视频
                if (/play|stream|video|src|url_list|download_addr|bit_rate/.test(k)) push(v, 'state:' + key);
            } else if (v && typeof v === 'object') {
                walk(v, depth + 1);
            }
        }
    };
    try {
        if (XHS) walk(window.__INITIAL_STATE__, 0);
        if (DY) {
            walk(window._ROUTER_DATA, 0);
            walk(window.__INITIAL_STATE__, 0);
            walk(window.__pace_f, 0);
        }
    } catch (e) {}

    // —— 3) 补充：DOM 上的 video / source ——
    let vtags = 0;
    try {
        const els = document.querySelectorAll('video, video source, source');
        vtags = document.querySelectorAll('video').length;
        for (const el of els) push(el.currentSrc, 'dom');
        for (const el of els) push(el.getAttribute && el.getAttribute('src'), 'dom-attr');
    } catch (e) {}

    // —— 打分排序：视频特征越强越靠前 ——
    const score = (c) => {
        const u = c.url;
        let s = 0;
        if (/\.m3u8(?:[?#]|$)/i.test(u)) s += 100;              // HLS：ffmpeg 可直接合并
        else if (/\.mp4(?:[?#]|$)/i.test(u)) s += 90;           // 直链 mp4：最理想
        if (/\/video\/tos\//i.test(u)) s += 40;                 // 抖音 TOS 路径
        if (/(aweme|douyinvod|xhscdn|snssdk|byteimg)/i.test(u)) s += 30;
        if (/video|play|stream/i.test(u)) s += 15;
        if (/[\?&]wm=\d/i.test(u)) s -= 20;                     // 水印地址降权
        if (/[\?&]ratio=(\d{3,4})p/i.test(u)) s += 5;           // 明确分辨率
        if (/\.(?:jpg|jpeg|png|webp)(?:[?#]|$)/i.test(u)) s -= 200; // 兜底再杀一次图片
        return s;
    };

    const uniq = [];
    const seen = new Set();
    for (const c of candidates) {
        if (seen.has(c.url)) continue;
        seen.add(c.url);
        uniq.push(c);
    }
    uniq.sort((a, b) => score(b) - score(a));

    // 音频候选单列（不受视频打分/截断影响）：抖音 DASH 分离流要靠它补音轨。
    const audios = uniq
        .filter((c) => c.audio)
        .slice(0, 6)
        .map((c) => c.url);

    // 优先非水印、非图片
    const best = uniq.find((c) => score(c) > 20 && !c.audio) || uniq.find((c) => !c.audio) || uniq[0] || null;
    return {
        url: best ? best.url : '',
        urls: uniq.filter((c) => !c.audio).slice(0, 12).map((c) => c.url),
        audios: audios,
        srcOf: best ? best.src : '',
        debug: JSON.stringify({
            xhs: !!window.__INITIAL_STATE__,
            dy: !!window._ROUTER_DATA,
            vtags,
            perfTotal,
            found: uniq.length,
            audios: audios.length,
            top: uniq.slice(0, 3).map((c) => ({ src: c.src, u: c.url.slice(0, 120) })),
        }),
    };
})()
"#;

/// 页面抽取结果：视频直链候选 + 音频直链候选。
///
/// 分开返回是必须的 —— 抖音 DASH 把音视频拆成两条流，只下视频轨会得到**无声 mp4**，
/// 后续语音转写必然失败（且失败原因极难定位）。音频候选用于下载后补音轨。
struct VideoStreamCandidates {
    videos: Vec<String>,
    audios: Vec<String>,
}

/// B 站取流：页面内调 `view`（拿 cid）+ `playurl`（拿 DASH 音视频直链）。
///
/// 不复用通用「扫 performance 资源」那套：B 站播放器用 **m4s + Range 分段**，
/// performance 里全是带 `&range=` 的分片地址，ffmpeg 只下一段就完事；且 CDN 校验 Referer。
/// 走官方 playurl 拿整段 `baseUrl` 才是完整文件。
const BILI_VIDEO_EXTRACT_EXPRESSION: &str = r#"
(async () => {
    const out = { url: '', urls: [], audios: [], debug: '' };
    const m = location.pathname.match(/\/video\/(BV[0-9A-Za-z]+)/);
    const bv = m ? m[1] : '';
    if (!bv) { out.debug = 'no-bv'; return out; }
    const getJson = async (u) => {
        try {
            const r = await fetch(u, { credentials: 'include' });
            return await r.json();
        } catch (e) { return null; }
    };
    const view = await getJson('https://api.bilibili.com/x/web-interface/view?bvid=' + bv);
    if (!view || view.code !== 0 || !view.data) {
        out.debug = 'view-fail:' + (view && view.code);
        return out;
    }
    const cid = view.data.cid || (view.data.pages && view.data.pages[0] && view.data.pages[0].cid) || 0;
    if (!cid) { out.debug = 'no-cid'; return out; }
    const pu = await getJson('https://api.bilibili.com/x/player/playurl?bvid=' + bv + '&cid=' + cid + '&qn=80&fnval=4048&fourk=1');
    if (!pu || pu.code !== 0 || !pu.data) {
        out.debug = 'playurl-fail:' + (pu && pu.code);
        return out;
    }
    const pick = (x) => x && (x.baseUrl || x.base_url || '');
    const dash = pu.data.dash || null;
    if (dash) {
        const vids = (dash.video || []).filter((x) => pick(x));
        // 优先 avc(h264, codecid=7)：WebView 直接能播；再按清晰度 id 从高到低
        vids.sort((a, b) => {
            const ca = a.codecid === 7 ? 1 : 0;
            const cb = b.codecid === 7 ? 1 : 0;
            if (ca !== cb) return cb - ca;
            return (b.id || 0) - (a.id || 0);
        });
        out.urls = vids.slice(0, 4).map(pick);
        const au = (dash.audio || []).filter((x) => pick(x));
        au.sort((a, b) => (b.bandwidth || 0) - (a.bandwidth || 0));
        out.audios = au.slice(0, 2).map(pick);
    } else if (pu.data.durl && pu.data.durl[0]) {
        // 老格式（番剧/部分版权视频）：整段 mp4
        out.urls = (pu.data.durl || []).slice(0, 4).map((d) => d.url).filter(Boolean);
    }
    out.url = out.urls[0] || '';
    out.debug = JSON.stringify({ bv: bv, cid: cid, videos: out.urls.length, audios: out.audios.length });
    return out;
})()
"#;

async fn extract_video_stream_url_async(
    ws_url: &str,
    post_url: &str,
    platform: &str,
) -> Result<VideoStreamCandidates, String> {
    let is_bili = Platform::parse(platform) == Some(Platform::Bilibili);
    let (expression, await_promise) = if is_bili {
        (BILI_VIDEO_EXTRACT_EXPRESSION.to_string(), true)
    } else {
        let mut expr = VIDEO_EXTRACT_EXPRESSION.to_string();
        expr = expr
            .replace("__XHS__", if Platform::parse(platform) == Some(Platform::Xiaohongshu) { "true" } else { "false" })
            .replace("__DY__", if Platform::parse(platform) == Some(Platform::Douyin) { "true" } else { "false" });
        (expr, false)
    };

    let result = headless_browser::navigate_and_eval_settle(
        ws_url,
        post_url,
        &expression,
        8_000,
        await_promise,
    )
    .await?;
    // navigate_and_eval 返回 JS 求值结果；要么是对象，要么是字符串化的对象，做兼容解析。
    let parsed = if result.is_string() {
        serde_json::from_str::<Value>(result.as_str().unwrap_or("{}"))
            .unwrap_or_else(|_| json!({}))
    } else {
        result.clone()
    };
    let debug = parsed
        .get("debug")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    // 候选直链（已按视频特征打分排序，首选在前）；交给调用方逐个尝试 —— 签名可能个别失效。
    let mut urls: Vec<String> = Vec::new();
    if let Some(list) = parsed.get("urls").and_then(Value::as_array) {
        for item in list {
            if let Some(s) = item.as_str() {
                if !s.is_empty() {
                    urls.push(s.to_string());
                }
            }
        }
    }
    if urls.is_empty() {
        if let Some(one) = parsed.get("url").and_then(Value::as_str) {
            if !one.is_empty() {
                urls.push(one.to_string());
            }
        }
    }
    if urls.is_empty() {
        return Err(format!(
            "未能从页面抽取到视频直链（debug：{debug}）。取流逻辑可能已变，请按 debug 回填抽取字段。"
        ));
    }
    let audios: Vec<String> = parsed
        .get("audios")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default();
    println!(
        "[monitor] extract_video_stream_url debug: {debug}（video={} audio={}）",
        urls.len(),
        audios.len()
    );
    Ok(VideoStreamCandidates {
        videos: urls,
        audios,
    })
}

/// 探测本地视频是否含音轨。
///
/// 抖音 DASH 的 `play_addr` 只给**视频轨**，下下来是个无声 mp4。不探测的话，后续语音转写会在
/// ffmpeg 抽音频阶段报 `Output file does not contain any stream`，而错误又被上层吞掉，
/// 表现成「转写永远为空」且原因不明。探测失败（ffprobe 缺）时保守返回 `true`，
/// 避免误伤本来正常的文件。
fn video_has_audio(app: &AppHandle, path: &str) -> bool {
    let Ok(probe) = ffprobe_program(Some(app)) else {
        return true;
    };
    let Ok(out) = Command::new(&probe)
        .arg("-v")
        .arg("error")
        .arg("-select_streams")
        .arg("a")
        .arg("-show_entries")
        .arg("stream=codec_name")
        .arg("-of")
        .arg("csv=p=0")
        .arg(path)
        .output()
    else {
        return true;
    };
    !String::from_utf8_lossy(&out.stdout).trim().is_empty()
}

/// 用 ffprobe 读容器时长（秒）。探测不到返回 `None`，不编造默认值。
fn probe_duration_secs(app: &AppHandle, path: &str) -> Option<f64> {
    let probe = ffprobe_program(Some(app)).ok()?;
    let out = Command::new(&probe)
        .arg("-v")
        .arg("error")
        .arg("-show_entries")
        .arg("format=duration")
        .arg("-of")
        .arg("csv=p=0")
        .arg(path)
        .output()
        .ok()?;
    let raw = String::from_utf8_lossy(&out.stdout);
    let value = raw.trim().trim_end_matches(',').parse::<f64>().ok()?;
    if value.is_finite() && value > 0.0 {
        Some(value)
    } else {
        None
    }
}

/// 为无声视频补音轨（抖音 DASH 音视频分离的补偿）。
///
/// 视频轨已从 `play_addr` 下好，这里再抓一条音频流（`music.play_url` / `.../audio/tos/...`），
/// 用 `-c copy` 合并成带声音的 mp4 并原地替换。合并走「临时文件 + rename」，
/// 中途失败不会破坏已下好的视频轨 —— 没声音也比没画面好。
fn merge_audio_into_video(
    app: &AppHandle,
    video_path: &std::path::Path,
    audio_url: &str,
    referer: &str,
    ua: &str,
) -> Result<(), String> {
    let ff = ffmpeg_program(Some(app))?;
    let headers = format!("Referer: {referer}\r\nUser-Agent: {ua}");
    let tmp_audio = video_path.with_extension("audio.m4a");
    let _ = std::fs::remove_file(&tmp_audio);
    let out = Command::new(&ff)
        .arg("-y")
        .arg("-headers")
        .arg(&headers)
        .arg("-i")
        .arg(audio_url)
        .arg("-rw_timeout")
        .arg("20000000")
        .arg("-vn")
        .arg(&tmp_audio)
        .output()
        .map_err(|e| format!("启动 ffmpeg 下载音频失败：{e}"))?;
    if !out.status.success() || std::fs::metadata(&tmp_audio).map(|m| m.len()).unwrap_or(0) == 0 {
        let _ = std::fs::remove_file(&tmp_audio);
        return Err(format!(
            "音频轨下载失败：{}",
            String::from_utf8_lossy(&out.stderr)
                .lines()
                .rev()
                .take(2)
                .collect::<Vec<_>>()
                .join(" | ")
        ));
    }

    let merged = video_path.with_extension("merged.mp4");
    let _ = std::fs::remove_file(&merged);
    let out = Command::new(&ff)
        .arg("-y")
        .arg("-i")
        .arg(video_path)
        .arg("-i")
        .arg(&tmp_audio)
        .arg("-map")
        .arg("0:v:0")
        .arg("-map")
        .arg("1:a:0")
        .arg("-c")
        .arg("copy")
        .arg("-movflags")
        .arg("+faststart")
        .arg(&merged)
        .output()
        .map_err(|e| format!("启动 ffmpeg 合并失败：{e}"))?;
    let _ = std::fs::remove_file(&tmp_audio);
    if !out.status.success() || std::fs::metadata(&merged).map(|m| m.len()).unwrap_or(0) == 0 {
        let _ = std::fs::remove_file(&merged);
        return Err(format!(
            "音视频合并失败：{}",
            String::from_utf8_lossy(&out.stderr)
                .lines()
                .rev()
                .take(2)
                .collect::<Vec<_>>()
                .join(" | ")
        ));
    }
    std::fs::rename(&merged, video_path).map_err(|e| format!("替换原视频失败：{e}"))?;
    Ok(())
}

/// 判断原片是否需要转码成 H.264 代理视频才能内联播放。
///
/// WebView2（Windows）多数环境**没有 HEVC/VP9/AV1 软硬解码器**，且 Tauri asset 协议不支持 Range 分段，
/// 超过 1080p 的原片即使编码可解，整文件拉取也很慢。所以：
/// - 视频流编码不是 `h264` → 必须转（HEVC/VP9/AV1 播不了）
/// - 高度 > 1080 → 转成 720p 轻量版（内联播放更稳）
/// - 其余（H.264 ≤1080p）→ 用 faststart 后的原片直接播，省下数分钟软转码。
///
/// 探测失败（ffprobe 缺/异常）时**保守转码**，宁慢勿放不出来。
fn needs_playback_transcode(app: &AppHandle, path: &str) -> bool {
    let probe = match ffprobe_program(Some(app)) {
        Ok(p) => p,
        Err(_) => return true,
    };
    let out = Command::new(&probe)
        .arg("-v")
        .arg("error")
        .arg("-select_streams")
        .arg("v:0")
        .arg("-show_entries")
        .arg("stream=codec_name,height")
        .arg("-of")
        .arg("json")
        .arg(path)
        .output();
    let parsed = match out {
        Ok(o) if o.status.success() => match serde_json::from_slice::<Value>(&o.stdout) {
            Ok(v) => v,
            Err(_) => return true,
        },
        _ => return true,
    };
    let streams = parsed.get("streams").and_then(Value::as_array);
    match streams.and_then(|s| s.first()) {
        Some(s) => {
            let codec = s.get("codec_name").and_then(Value::as_str).unwrap_or("").to_lowercase();
            let height = s.get("height").and_then(Value::as_i64).unwrap_or(0);
            let not_h264 = codec != "h264" && !codec.is_empty();
            let too_tall = height > 1080;
            not_h264 || too_tall
        }
        None => true,
    }
}

/// 下载竞品视频到本地（仅小红书 / 抖音，需登录态 Chrome）。
///
/// 流程：CDP 现抓现下视频直链（签名短链过期快）→ 用 ffmpeg 落盘（同时支持直链 mp4 与 HLS m3u8 合并）。
/// 下载成功写回 `video_local_path` / `video_status=done`；失败写 `video_status=failed` + `video_error`。
fn download_video(app: &AppHandle, state: &State<'_, AppState>, payload: &Value) -> Result<Value, String> {
    let id = str_field(payload, "id").ok_or_else(|| "缺少 id".to_string())?;
    let record = with_store_mut(state, |store| {
        Ok(store.monitored_posts.iter().find(|p| p.id == id).cloned())
    })?
    .ok_or_else(|| "动态不存在".to_string())?;
    if !Platform::parse(&record.platform).map(|p| p.supports_video()).unwrap_or(false) {
        return Ok(json!({
            "success": false,
            "note": "目前仅支持小红书 / 抖音 / B 站的视频下载（需登录态 Chrome）。",
        }));
    }
    let port = headless_browser::active_chrome_port(&record.platform);
    if port == 0 {
        return Ok(json!({
            "success": false,
            "note": "该平台 Chrome 未连接（应用重启后会话失效）。请到「登录管理」点「打开 Chrome 登录」重新接管后再下载。",
        }));
    }

    let dir = monitor_videos_dir(state)?;
    let dest = dir.join(format!("{id}.mp4"));
    let dest_str = dest.display().to_string();
    let platform = record.platform.clone();
    let post_url = record.post_url.clone();
    let referer = Platform::parse(&platform)
        .map(|p| p.home_url())
        .unwrap_or(Platform::Douyin.home_url())
        .to_string();
    let ua = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36".to_string();

    // 标记下载中，便于前端展示状态
    with_store_mut(state, |store| {
        if let Some(p) = store.monitored_posts.iter_mut().find(|x| x.id == id) {
            p.video_status = "downloading".to_string();
        }
        Ok(())
    })?;

    // 1) CDP 现抓现下视频直链（签名短链，必须同会话内完成）
    let candidates = tauri::async_runtime::block_on(async {
        let ws_url = headless_browser::ws_url_for_port(port).await?;
        extract_video_stream_url_async(&ws_url, &post_url, &platform).await
    })?;

    // 2) ffmpeg 落盘（直链 mp4 与 HLS m3u8 都支持；带 Referer 过防盗链）。
    //    签名链偶发失效 / 返回 403，按候选顺序逐个重试，取第一个成功的。
    //
    // `patchAudio` 模式：视频轨已在本地且完好时**跳过视频重下**，只走后面的补音轨逻辑。
    // 存量 54 条抖音视频都是「有画面无声音」，重下要再拉 2GB+，而补音轨只下几十 KB。
    let patch_only = payload
        .get("patchAudio")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let existing_video_ok =
        dest.is_file() && std::fs::metadata(&dest).map(|m| m.len() > 0).unwrap_or(false);
    let skip_video_download = patch_only && existing_video_ok;

    let ff = ffmpeg_program(Some(app))?;
    let headers = format!("Referer: {referer}\r\nUser-Agent: {ua}");
    let mut last_stderr = String::new();
    let mut ok = skip_video_download;
    if skip_video_download {
        println!("[monitor] patchAudio: 跳过视频重下，直接补音轨 {id}");
    }
    for stream_url in candidates.videos.iter().take(4).filter(|_| !skip_video_download) {
        let output = Command::new(&ff)
            .arg("-y")
            .arg("-headers")
            .arg(&headers)
            .arg("-i")
            .arg(stream_url)
            .arg("-c")
            .arg("copy")
            // faststart：把 moov 索引原子搬到文件头。不加的话 moov 在 500MB+ 的文件末尾，
            // 而 Tauri asset 协议不支持 Range 请求，WebView 只能整文件拉完才能解码 → 视频卡住播不出。
            .arg("-movflags")
            .arg("+faststart")
            .arg("-rw_timeout")
            .arg("20000000")
            .arg(&dest_str)
            .output()
            .map_err(|e| format!("启动 ffmpeg 失败：{e}"))?;
        if output.status.success() {
            ok = true;
            break;
        }
        last_stderr = String::from_utf8_lossy(&output.stderr).to_string();
        println!("[monitor] ffmpeg candidate failed: {}", &last_stderr.lines().rev().take(3).collect::<Vec<_>>().join(" | "));
    }
    if !ok {
        // 失败时删掉可能残留的半截文件，避免下次「已下载」判定出错
        let _ = std::fs::remove_file(&dest);
        let brief: String = last_stderr
            .lines()
            .rev()
            .filter(|l| !l.trim().is_empty())
            .take(3)
            .collect::<Vec<_>>()
            .join(" | ");
        let msg = if brief.is_empty() {
            "ffmpeg 下载失败（无输出，可能是链接已过期，重试即可）".to_string()
        } else {
            format!("ffmpeg 下载失败：{brief}")
        };
        with_store_mut(state, |store| {
            if let Some(p) = store.monitored_posts.iter_mut().find(|x| x.id == id) {
                p.video_status = "failed".to_string();
                p.video_error = Some(msg.clone());
            }
            Ok(())
        })?;
        return Ok(json!({ "success": false, "error": msg }));
    }

    // 2.5) 补音轨：抖音走 DASH，音视频是**两条独立流**，`play_addr` 只含视频轨。
    //      不补的话拿到的 mp4 全程无声 → 语音转写必然失败（这正是此前 56 次转写
    //      全部抽音频失败、转写恒空的根因）。补失败不阻断下载：无声音仍可抽帧、可播放。
    let has_audio_before = video_has_audio(app, &dest_str);
    let mut audio_patched = false;
    let mut audio_note: Option<String> = None;
    if !has_audio_before {
        if candidates.audios.is_empty() {
            audio_note = Some("源视频无音轨，且页面未抽取到音频候选（抖音可能已改版）".to_string());
        } else {
            let mut last_err = String::new();
            for audio_url in candidates.audios.iter().take(3) {
                match merge_audio_into_video(app, &dest, audio_url, &referer, &ua) {
                    Ok(()) => {
                        audio_patched = true;
                        println!("[monitor] audio track merged for {id}");
                        break;
                    }
                    Err(e) => {
                        println!("[monitor] audio merge failed for {id}: {e}");
                        last_err = e;
                    }
                }
            }
            if !audio_patched {
                audio_note = Some(format!("源视频无音轨，补音轨失败：{last_err}"));
            }
        }
    }
    let has_audio = has_audio_before || audio_patched;

    let size = std::fs::metadata(&dest).map(|m| m.len() as i64).unwrap_or(0);

    // 3) 按需生成**轻量代理视频**用于前端播放。
    //
    // 原片若是 H.264 且 ≤1080p，WebView 配 faststart 后直接内联播放即可，**不必转码**（省下数分钟）。
    // 仅当原片是 HEVC/VP9/AV1（WebView2 多数环境无解码器）或超过 1080p 时才转成 720p/H.264/CRF30——
    // 实测 4K·HEVC·14 分钟 533MB 全软转码要 4-5 分钟，这是 CPU 软编码的固有成本，不能更快，
    // 故前端超时放宽到 600s 覆盖它；原片始终保留供抽帧与转写。
    let preview_path = dest.with_file_name(format!("{id}.preview.mp4"));
    let _ = std::fs::remove_file(&preview_path); // 上次失败的半截产物先清掉
    let need_transcode = needs_playback_transcode(app, &dest_str);
    // 默认不转码：复用已 faststart 的原片作播放源（H.264 ≤1080p 直接可播）。
    let mut preview_str: Option<String> = None;
    if need_transcode {
        let transcode = Command::new(&ff)
            .arg("-y")
            .arg("-i")
            .arg(&dest_str)
            .arg("-vf")
            // 最长边不超过 1280，保持偶数宽高（h264 yuv420p 要求）
            .arg("scale='min(1280,iw)':-2")
            .arg("-c:v")
            .arg("libx264")
            .arg("-preset")
            .arg("veryfast")
            .arg("-crf")
            .arg("30")
            .arg("-pix_fmt")
            .arg("yuv420p")
            .arg("-c:a")
            .arg("aac")
            .arg("-b:a")
            .arg("96k")
            .arg("-movflags")
            .arg("+faststart")
            .arg(&preview_path)
            .output()
            .map_err(|e| format!("启动 ffmpeg 转码失败：{e}"))?;
        if transcode.status.success()
            && std::fs::metadata(&preview_path).map(|m| m.len()).unwrap_or(0) > 0
        {
            preview_str = Some(preview_path.display().to_string());
        } else {
            println!(
                "[monitor] preview transcode failed: {}",
                String::from_utf8_lossy(&transcode.stderr)
                    .lines()
                    .rev()
                    .take(2)
                    .collect::<Vec<_>>()
                    .join(" | ")
            );
        }
    }
    // 转码失败不阻断下载：前端退回原片，原片若是 H.264 仍可播


    with_store_mut(state, |store| {
        if let Some(p) = store.monitored_posts.iter_mut().find(|x| x.id == id) {
            p.video_local_path = Some(dest_str.clone());
            p.video_status = "done".to_string();
            p.video_size = Some(size);
            p.video_preview_path = preview_str.clone();
            p.video_downloaded_at = Some(now_i64());
            p.video_error = None;
            p.video_has_audio = Some(has_audio);
        }
        Ok(())
    })?;

    Ok(json!({
        "success": true,
        "localPath": dest_str,
        "previewPath": preview_str,
        "size": size,
        "hasAudio": has_audio,
        "audioWarning": audio_note,
    }))
}

/// 跑一次语音转写，返回 `(全文, 状态, 错误说明)`。
///
/// 状态取值：`ok` / `no-audio`（源视频无音轨）/ `not-configured`（设置里没配 ASR）/ `failed`。
///
/// 返回体契约（务必对齐 `execute_media_transcribe`）：`transcript`（全文）、`transcriptPath`
/// （字幕文件路径）、`contentPreview`（**已被截断到 4000 字**）。早期实现按 `text`/`subtitles`/
/// `content` 取值 —— 这些键根本不存在，于是转写成功也恒为空串。改动此处前先确认契约。
fn run_transcription(
    app: &AppHandle,
    state: &State<'_, AppState>,
    local_path: &str,
) -> (String, String, Option<String>, String) {
    if !video_has_audio(app, local_path) {
        return (
            String::new(),
            "no-audio".to_string(),
            Some(
                "源视频没有音轨（抖音 DASH 音视频分离，只下到了视频轨），无法转写。\
                 重新下载该视频会自动补音轨后再分析。"
                    .to_string(),
            ),
            String::new(),
        );
    }
    match execute_media_transcribe(app, state, None, &json!({ "sourcePath": local_path })) {
        Ok(v) => {
                let text = v
                .get("transcript")
                .and_then(Value::as_str)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .or_else(|| {
                    v.get("contentPreview")
                        .and_then(Value::as_str)
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                })
                .unwrap_or_default();
            // 时间轴：单独取转写服务产出的 SRT 文件（带每句起止时间），供前端做「时间轴」展示。
            // 与 transcript 分离，绝不把 `00:00:00,800 -->` 塞进 clean 全文（会污染 LLM / 详情页）。
            let srt = v
                .get("transcriptPath")
                .and_then(Value::as_str)
                .and_then(|p| std::fs::read_to_string(p).ok())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_default();
            if text.is_empty() {
                // 没有干净全文时，从 SRT 抽掉序号 / 时间轴行作为降级全文，保证 transcript 不为空且无时间戳噪音。
                let plain = if srt.is_empty() {
                    String::new()
                } else {
                    strip_srt_timestamps(&srt)
                };
                if plain.is_empty() {
                    (
                        String::new(),
                        "failed".to_string(),
                        Some("转写服务返回了空内容（整段无口播，或音频被判成纯音乐）。".to_string()),
                        srt,
                    )
                } else {
                    (plain, "ok".to_string(), None, srt)
                }
            } else {
                (text, "ok".to_string(), None, srt)
            }
        }
        Err(e) => {
            println!("[monitor] video transcript failed: {e}");
            let status = if e.contains("未配置音频转写服务") {
                "not-configured"
            } else {
                "failed"
            };
            (
                String::new(),
                status.to_string(),
                Some(e.chars().take(800).collect()),
                String::new(),
            )
        }
    }
}

/// 把 SRT 文本去掉序号行与时间轴行（`00:00:01,200 --> 00:00:04,500`），仅保留字幕正文。
/// 用于转写服务只回传 SRT、没有干净 `transcript` 全文时的降级处理。
fn strip_srt_timestamps(srt: &str) -> String {
    srt.lines()
        .map(|l| l.trim())
        .filter(|l| {
            if l.is_empty() {
                return false;
            }
            // 跳过纯序号行
            if l.chars().all(|c| c.is_ascii_digit()) {
                return false;
            }
            // 跳过时间轴行
            if l.contains("-->") {
                return false;
            }
            true
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 分析已下载的竞品视频：抽帧（ffmpeg）+ 语音转写（复用 media_transcribe）+ 落库。
///
/// 视频本体需先经 `monitor:download-video` 落盘；转写需在设置里配置 transcription 服务，
/// 未配置时 transcript 留空（不报错，仅跳过）。LLM 摘要（`summary`）暂未接入，可后续复用 deconstruct 链路。
fn analyze_video(app: &AppHandle, state: &State<'_, AppState>, payload: &Value) -> Result<Value, String> {
    let id = str_field(payload, "id").ok_or_else(|| "缺少 id".to_string())?;
    let record = with_store_mut(state, |store| {
        Ok(store.monitored_posts.iter().find(|p| p.id == id).cloned())
    })?
    .ok_or_else(|| "动态不存在".to_string())?;
    let local_path = record
        .video_local_path
        .clone()
        .ok_or_else(|| "该视频尚未下载，请先「下载视频」".to_string())?;

    let dir = monitor_videos_dir(state)?;
    let frames_dir = dir.join(&id).join("frames");
    std::fs::create_dir_all(&frames_dir).map_err(|e| format!("创建帧目录失败：{e}"))?;

    // 抽帧：每 5s 一帧（客户需求：5 秒间隔、不限制张数，长视频按真实时长全量抽）
    let ff = ffmpeg_program(Some(app))?;
    let frame_pattern = frames_dir.join("frame_%03d.jpg").display().to_string();
    let _ = Command::new(&ff)
        .arg("-y")
        .arg("-i")
        .arg(&local_path)
        .arg("-vf")
        .arg("fps=1/5")
        .arg(&frame_pattern)
        .output();

    let frames: Vec<String> = std::fs::read_dir(&frames_dir)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|e| {
                    let p = e.path();
                    if p.extension().map(|x| x == "jpg").unwrap_or(false) {
                        Some(p.display().to_string())
                    } else {
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    // 语音转写：默认在抽帧后同步跑；`skipTranscript` 时为「只抽帧」模式（下载 worker 用），
    // 转写交给 `spawn_transcribe_workers` 的后置队列，避免 ASR 等待阻塞下载并行度。
    let local_path_str = local_path.clone();
    let skip_transcript = payload
        .get("skipTranscript")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let force_transcript = payload
        .get("force")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    // 已有可用转写时直接复用：重新分析通常只想刷新抽帧/摘要，
    // 不能因为一次网络抖动或服务商故障，把之前成功的结果覆盖成空。
    let existing_analysis = with_store(state, |store| {
        Ok(store
            .post_video_analyses
            .iter()
            .find(|a| a.post_id == id)
            .cloned())
    })?;
    let existing_opt = existing_analysis
        .as_ref()
        .filter(|a| a.transcript_status == "ok" && !a.transcript.trim().is_empty());

    let (mut transcript, mut transcript_status, mut transcript_error, mut transcript_srt) = if skip_transcript {
        (String::new(), "pending".to_string(), None, String::new())
    } else if !force_transcript {
        if let Some(a) = existing_opt {
            (a.transcript.clone(), a.transcript_status.clone(), a.transcript_error.clone(), a.transcript_srt.clone())
        } else {
            run_transcription(app, state, &local_path_str)
        }
    } else {
        run_transcription(app, state, &local_path_str)
    };

    // 本次转写失败/为空、但之前有成功结果 → 保留旧结果（含时间轴）
    if !skip_transcript && transcript.trim().is_empty() && transcript_status != "ok" {
        if let Some(a) = existing_opt {
            transcript = a.transcript.clone();
            transcript_status = a.transcript_status.clone();
            transcript_error = a.transcript_error.clone();
            transcript_srt = a.transcript_srt.clone();
        }
    }
    let duration_secs = probe_duration_secs(app, &local_path_str);

    // 按 post_id upsert 到视频分析库
    let now = now_i64();
    let analysis = with_store_mut(state, |store| {
        let existing = store.post_video_analyses.iter().find(|a| a.post_id == id).cloned();
        let mut rec = existing.unwrap_or_else(PostVideoAnalysisRecord::default);
        if rec.id.is_empty() {
            rec.id = make_id("video-analysis");
        }
        rec.post_id = id.clone();
        rec.platform = record.platform.clone();
        rec.author = record.author.clone();
        rec.post_title = record.title.clone();
        rec.post_url = record.post_url.clone();
        rec.local_path = local_path.clone();
        rec.transcript = transcript.clone();
        rec.transcript_status = transcript_status.clone();
        rec.transcript_error = transcript_error.clone();
        rec.transcript_srt = transcript_srt.clone();
        rec.duration_secs = duration_secs;
        rec.frames = frames.clone();
        rec.created_at = if rec.created_at == 0 { now } else { rec.created_at };
        rec.updated_at = now;
        if let Some(slot) = store.post_video_analyses.iter_mut().find(|a| a.post_id == id) {
            *slot = rec.clone();
        } else {
            store.post_video_analyses.push(rec.clone());
        }
        Ok(serde_json::to_value(&rec).unwrap_or_else(|_| json!({})))
    })?;

    // 分析完成后自动导出「帖子最终产物」到知识库目录树
    export_post_product(state, &id);
    // 清理原片，仅保留预览（存在独立预览时才删）
    delete_original_after_analysis(state, &id);

    Ok(json!({ "success": true, "analysis": analysis }))
}

/// 只补语音转写：不重新抽帧、不动已有帧，仅跑 ASR 并回填记录。
///
/// 用于「下载阶段只抽帧，转写排队后置」的流程（见 `spawn_transcribe_workers`），
/// 也供前端「只重新转写」按钮使用。记录不存在时退化成完整分析（先建记录再说）。
fn transcribe_video_only(
    app: &AppHandle,
    state: &State<'_, AppState>,
    payload: &Value,
) -> Result<Value, String> {
    let id = str_field(payload, "id").ok_or_else(|| "缺少 id".to_string())?;
    let has_record = with_store(state, |store| {
        Ok(store
            .post_video_analyses
            .iter()
            .any(|a| a.post_id == id))
    })?;
    if !has_record {
        return analyze_video(app, state, &json!({ "id": id }));
    }
    let record = with_store(state, |store| {
        Ok(store.monitored_posts.iter().find(|p| p.id == id).cloned())
    })?
    .ok_or_else(|| "动态不存在".to_string())?;
    let local_path = record
        .video_local_path
        .clone()
        .ok_or_else(|| "该视频尚未下载，请先「下载视频」".to_string())?;

    let (transcript, status, error, srt) = run_transcription(app, state, &local_path);
    let now = now_i64();
    let analysis = with_store_mut(state, |store| {
        let Some(rec) = store.post_video_analyses.iter_mut().find(|a| a.post_id == id) else {
            return Ok(json!({}));
        };
        rec.transcript = transcript.clone();
        rec.transcript_status = status.clone();
        rec.transcript_error = error.clone();
        rec.transcript_srt = srt.clone();
        rec.updated_at = now;
        Ok(serde_json::to_value(&*rec).unwrap_or_else(|_| json!({})))
    })?;

    // 转写补齐后自动导出「帖子最终产物」到知识库目录树
    export_post_product(state, &id);
    delete_original_after_analysis(state, &id);

    Ok(json!({ "success": true, "analysis": analysis, "status": status }))
}

/// —— 帖子最终产物导出（5 级目录分类）——
///
/// 把一篇帖子的「最终产物」按知识库根目录落盘为物理文件夹树：
///   知识库根 / 平台(抖音) / 日期(YYYYMMDD) / 作者 / 标题前10字[+帖子ID] / 具体内容文件
/// 触发：视频帖 `analyze_video` 完成后、转写补齐(`transcribe_video_only`)后、以及
///       图文/视频帖 `deconstruct` 完成后自动导出；也可经 `monitor:export-post` 手动触发
///       （便于导出未拆解的图文帖）。
/// 内容为：原图(`images`)、抽帧图(`frames`)、正文、转写全文、转写时间轴(SRT)、
///         视频摘要、爆文拆解(六要素 JSON+MD)、帖子信息(JSON 溯源)。
/// 设计为幂等：重复导出只覆盖/补齐，不会重复造目录；标题前 10 字撞名时追加帖子 ID
/// （通过 `.beav-post-id` 标记文件识别「自己的」目录，避免重导出时产生孤儿目录）。

/// 中文平台名（知识库一级目录用）。
fn platform_label(platform: &str) -> String {
    match Platform::parse(platform) {
        Some(p) => p.label().to_string(),
        None => {
            let s = platform.trim();
            if s.is_empty() {
                "未知平台".to_string()
            } else {
                sanitize_fs_name(s)
            }
        }
    }
}

/// 把文件名非法字符（Windows：`\ / : * ? " < > |` 及控制字符）替换为 `_` 并压缩连续下划线。
fn sanitize_fs_name(s: &str) -> String {
    const ILLEGAL: &[char] = &['\\', '/', ':', '*', '?', '"', '<', '>', '|'];
    let mut out: String = s
        .chars()
        .map(|c| {
            if ILLEGAL.contains(&c) || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    while out.contains("__") {
        out = out.replace("__", "_");
    }
    out.trim().trim_matches('_').to_string()
}

/// 标题前 `n` 个字符（按 Unicode 字符计）作为四级目录名；空标题回退「未命名」。
fn title_export_prefix(title: &str, n: usize) -> String {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return "未命名".to_string();
    }
    let prefix: String = trimmed.chars().take(n).collect();
    let sanitized = sanitize_fs_name(&prefix);
    if sanitized.is_empty() {
        "未命名".to_string()
    } else {
        sanitized
    }
}

/// 从文本里抠出第一个形如 `YYYY-MM-DD` / `YYYY/MM/DD` / `YYYYMMDD` 的日期，规范为 `YYYYMMDD`。
fn first_date_in(text: &str) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut i = 0;
    while i < n {
        if chars[i].is_ascii_digit() {
            // YYYYMMDD
            if i + 8 <= n && chars[i..i + 8].iter().all(|c| c.is_ascii_digit()) {
                let s: String = chars[i..i + 8].iter().copied().collect();
                if let (Ok(y), Ok(m), Ok(d)) = (
                    s[0..4].parse::<u32>(),
                    s[4..6].parse::<u32>(),
                    s[6..8].parse::<u32>(),
                ) {
                    if (1..=12).contains(&m) && (1..=31).contains(&d) {
                        return Some(s);
                    }
                }
            }
            // YYYY-MM-DD / YYYY/MM/DD
            if i + 10 <= n {
                let b: Vec<char> = chars[i..i + 10].iter().copied().collect();
                if b[4] == '-' || b[4] == '/' {
                    let y: String = b[0..4].iter().copied().collect();
                    let m: String = b[5..7].iter().copied().collect();
                    let d: String = b[8..10].iter().copied().collect();
                    if let (Ok(yy), Ok(mm), Ok(dd)) =
                        (y.parse::<u32>(), m.parse::<u32>(), d.parse::<u32>())
                    {
                        if (1..=12).contains(&mm) && (1..=31).contains(&dd) {
                            return Some(format!("{yy:04}{mm:02}{dd:02}"));
                        }
                    }
                }
            }
        }
        i += 1;
    }
    None
}

/// 毫秒时间戳 → 本地时区 `YYYYMMDD`。
fn millis_to_yyyymmdd(ms: i64) -> Option<String> {
    let secs = ms / 1000;
    let dt = match Utc.timestamp_opt(secs, 0) {
        chrono::LocalResult::Single(dt) => dt,
        _ => return None,
    };
    let local = dt.with_timezone(&Local);
    Some(local.format("%Y%m%d").to_string())
}

fn today_yyyymmdd() -> String {
    Local::now().format("%Y%m%d").to_string()
}

/// 帖子产物二级目录（日期）：优先用可解析的 `published_at`，否则回退 `created_at`（抓取时间），再否则今天。
fn export_date_yyyymmdd(post: &MonitoredPostRecord) -> String {
    if let Some(pa) = post.published_at.as_deref() {
        if let Some(d) = first_date_in(pa) {
            return d;
        }
        if let Ok(ms) = pa.parse::<i64>() {
            if let Some(d) = millis_to_yyyymmdd(ms) {
                return d;
            }
        }
    }
    if let Ok(ms) = post.created_at.parse::<i64>() {
        if let Some(d) = millis_to_yyyymmdd(ms) {
            return d;
        }
    }
    today_yyyymmdd()
}

/// 在三级目录（作者）下找「自己的」产物目录（依据 `.beav-post-id` 标记）。
fn find_own_export_dir(l3: &Path, post_id: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(l3).ok()?;
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            if let Ok(content) = std::fs::read_to_string(p.join(".beav-post-id")) {
                if content.trim() == post_id {
                    return Some(p);
                }
            }
        }
    }
    None
}

/// 仅在目标缺失或大小变化时才拷贝，避免重复导出时反复复制大文件。返回是否真的复制了。
fn copy_if_changed(src: &Path, dst: &Path) -> bool {
    if let (Ok(sm), Ok(dm)) = (std::fs::metadata(src), std::fs::metadata(dst)) {
        if sm.len() == dm.len() {
            return false;
        }
    }
    match std::fs::copy(src, dst) {
        Ok(_) => true,
        Err(e) => {
            println!(
                "[monitor] export copy failed {} -> {}: {e}",
                src.display(),
                dst.display()
            );
            false
        }
    }
}

/// 真实落盘一篇帖子的「最终产物」到 5 级目录树。best-effort：任何失败都只记日志不阻断主流程。
fn export_post_product(state: &State<'_, AppState>, post_id: &str) {
    if let Err(e) = try_export_post_product(state, post_id) {
        println!("[monitor] export_post_product failed for {post_id}: {e}");
    }
}

/// 解析「帖子最终产物」的导出根目录（**始终跟随导出根目录**，不再按平台选知识库）。
///
/// - `monitor_export_root` 非空 → 用它（**不要求目录已存在**，导出时会 `create_dir_all`）；
/// - 否则回退 `workspace_root`（保留平台级一级目录）。
///
/// 注意：早期实现用 `is_dir()` 判断，用户清理过导出目录后（目录被删）会**静默回退到工作区**，
/// 表现为「设置了导出根目录但产物仍落在工作区」。这里改为只认配置值，目录由导出流程按需创建。
fn resolve_export_target(state: &State<'_, AppState>) -> Result<PathBuf, String> {
    let export_root = with_store(state, |store| Ok(store.monitor_export_root.clone()))?;
    let export_root = export_root.trim().to_string();
    if !export_root.is_empty() {
        return Ok(PathBuf::from(&export_root));
    }
    workspace_root(state)
}

/// 拆出标题尾部的 hashtag（抖音标题常把话题直接贴在末尾，如 `…决策者#财务 #cfo`）。
///
/// 返回（清洗后标题, 标签列表）。只有末尾形如 `#token`（token 内无空白）才会被剥离，
/// 因此不会误伤正文中出现的 `#`。
fn split_title_tags(raw: &str) -> (String, Vec<String>) {
    let mut tags: Vec<String> = Vec::new();
    let mut head = raw.trim().to_string();
    while let Some(pos) = head.rfind('#') {
        let tail = &head[pos + 1..];
        if tail.is_empty() || tail.contains(char::is_whitespace) || tail.contains('#') {
            break;
        }
        let tag = tail.trim_matches(|c: char| !(c.is_alphanumeric() || c == '_' || c == '-'));
        if tag.is_empty() {
            break;
        }
        tags.push(tag.to_string());
        head.truncate(pos);
        head = head.trim_end().to_string();
    }
    tags.reverse();
    (head.trim().to_string(), tags)
}

/// 生成 YAML 双引号标量（转义 `"` 与 `\`，换行压成空格）。
fn yaml_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' | '\r' => out.push(' '),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// 生成 Obsidian 知识卡 `knowledge.md`（旧名 `Beav知识卡.md`，写新版时会顺手删掉旧的）。
///
/// 设计要点（2026-10-27 重写）：
/// - **正文 / 语音转写直接内联**：Obsidian 不渲染 `.txt` / `.srt` / `.json`，
///   早期用 `![[转写全文]]` 会变成死链，因此改为把文本写进笔记本身。
/// - **关键帧用 `![](文件)` 图片引用而非 `![[...]]`**：只显示、不生成图谱边，
///   避免一篇帖子往图谱里连出上百条边。
/// - 元数据进 YAML frontmatter（作者/平台/赞藏评/来源/标签），hashtag 从标题剥离后转成 `tags`。
///
/// - 始终在产物目录 `l4` 写一份（prefix 为空，同目录相对引用，整包可移植）。
/// - 若配置了 `monitor_obsidian_vault` 且产物目录在其内，另在
///   `<vault>/Beav导出/<l4 相对导出根的路径>/` 写一份（prefix 为 vault 相对路径）。
///
/// best-effort：任何失败只记日志，不阻断导出主流程。
fn write_obsidian_card(
    state: &State<'_, AppState>,
    root: &Path,
    l4: &Path,
    post: &MonitoredPostRecord,
    analysis: &Option<PostVideoAnalysisRecord>,
    decon: &Option<crate::PostDeconstructionRecord>,
) {
    let (title, tags) = split_title_tags(post.title.trim());
    let title = if title.is_empty() { "未命名".to_string() } else { title };

    let ext_of = |p: &str| -> String {
        Path::new(p)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("jpg")
            .to_lowercase()
    };

    // 只列实际存在的产物（导出时对缺失源会 continue），避免断链
    let mut images: Vec<String> = Vec::new();
    for (idx, img) in post.images.iter().enumerate() {
        if Path::new(img).exists() {
            images.push(format!("图片_{}.{}", idx + 1, ext_of(img)));
        }
    }
    let mut frames: Vec<String> = Vec::new();
    if let Some(a) = analysis {
        for (idx, fr) in a.frames.iter().enumerate() {
            if Path::new(fr).exists() {
                frames.push(format!("抽帧_{:03}.{}", idx + 1, ext_of(fr)));
            }
        }
    }
    let video_src = post
        .video_preview_path
        .as_ref()
        .filter(|p| Path::new(p).exists())
        .or_else(|| post.video_local_path.as_ref().filter(|p| Path::new(p).exists()));
    let video_name = video_src.map(|p| format!("视频.{}", ext_of(p)));

    let body = post.content.trim();
    let transcript = analysis
        .as_ref()
        .map(|a| a.transcript.trim().to_string())
        .unwrap_or_default();
    let summary = analysis
        .as_ref()
        .map(|a| a.summary.trim().to_string())
        .unwrap_or_default();
    let srt = analysis
        .as_ref()
        .map(|a| a.transcript_srt.trim().to_string())
        .unwrap_or_default();

    if body.is_empty()
        && transcript.is_empty()
        && frames.is_empty()
        && images.is_empty()
        && decon.is_none()
        && video_name.is_none()
    {
        return;
    }

    let platform = platform_label(&post.platform);
    let published = post.published_at.as_deref().unwrap_or("").trim().to_string();
    let url = post.post_url.trim().to_string();

    // `rel`：相对当前笔记所在目录的路径前缀（HTML `<img>` / Markdown 链接用）
    // `vault`：vault 根相对路径前缀（`![[wikilink]]` 用，Obsidian 的 wikilink 始终从库根解析）
    // 产物目录那份两者都是空串；库内 hub 那份 `rel` 是 `../../…/`、`vault` 是库内路径。
    let render = |rel: &str, vault: &str| -> String {
        let path_of = |name: &str| format!("{rel}{name}");
        // 配图 / 关键帧都走原生 `![|200](path)`：Live Preview 下点击只打开预览、不露源码
        // （旧的内联 HTML `<img>` 会露源码）；附件默认不进图谱，不产生图谱边。
        // 关键帧用「默认折叠的 callout」承载，替代 `<details>`（HTML 块内 markdown 不解析）。
        let md_img = |name: &str| format!("![|200]({})", path_of(name));

        let mut md = String::new();

        // ---- YAML frontmatter ----
        md.push_str("---\n");
        md.push_str(&format!("title: {}\n", yaml_str(&title)));
        md.push_str(&format!(
            "author: {}\n",
            yaml_str(&sanitize_fs_name(&post.author))
        ));
        md.push_str(&format!("platform: {}\n", yaml_str(&platform)));
        md.push_str(&format!("post_id: {}\n", yaml_str(&post.id)));
        if !url.is_empty() {
            md.push_str(&format!("url: {}\n", yaml_str(&url)));
        }
        if !published.is_empty() {
            md.push_str(&format!("published: {}\n", yaml_str(&published)));
        }
        md.push_str(&format!("likes: {}\n", post.likes));
        md.push_str(&format!("collects: {}\n", post.collects));
        md.push_str(&format!("comments: {}\n", post.comments));
        if !tags.is_empty() {
            let list: Vec<String> = tags.iter().map(|t| yaml_str(t)).collect();
            md.push_str(&format!("tags: [{}]\n", list.join(", ")));
        }
        md.push_str("source: beav-monitor\n");
        md.push_str("---\n\n");

        // ---- 摘要行 ----
        md.push_str(&format!("# {title}\n\n"));
        md.push_str(&format!(
            "> **{}** · {} · 赞 {} / 藏 {} / 评 {}\n",
            sanitize_fs_name(&post.author),
            platform,
            post.likes,
            post.collects,
            post.comments
        ));
        if !url.is_empty() {
            md.push_str(&format!("> [查看原帖]({url})\n"));
        }
        md.push('\n');

        // ---- 正文（内联） ----
        if !body.is_empty() {
            md.push_str("## 正文\n\n");
            md.push_str(body);
            md.push_str("\n\n");
        }

        // ---- 语音转写（内联，折叠） ----
        if !transcript.is_empty() {
            md.push_str("## 语音转写\n\n");
            md.push_str(&format!(
                "<details>\n<summary>展开全文（{} 字）</summary>\n\n",
                transcript.chars().count()
            ));
            md.push_str(&transcript);
            md.push_str("\n\n</details>\n\n");
        }
        // ---- 逐句时间轴：直接展开显示 ----
        // `.srt` 不是 Obsidian 支持的嵌入格式，原来的链接点开只会甩给外部播放器；
        // 这里把文本内联进代码块（折叠），一眼能看，也不产生图谱边。
        if !srt.is_empty() {
            let cues = srt.lines().filter(|line| line.contains("-->")).count();
            md.push_str("### 逐句时间轴\n\n");
            md.push_str(&format!(
                "<details>\n<summary>展开全文（{cues} 条）</summary>\n\n```\n"
            ));
            md.push_str(&srt);
            md.push_str("\n```\n\n</details>\n\n");
        }
        if !summary.is_empty() {
            md.push_str("## 视频摘要\n\n");
            md.push_str(&summary);
            md.push_str("\n\n");
        }

        // ---- 关键帧（折叠 + 图片引用，不产生图谱边） ----
        // 用「默认折叠的 callout」而非 `<details>`：callout 内容仍是 markdown，图片走原生
        // `![|200]()`（Live Preview 点击只开预览、不露源码）；`<details>` 是 HTML 块，块内
        // markdown 不解析，只能用 HTML `<img>`，点开即露源码。折叠 = 默认不展开，保持轻量。
        if !frames.is_empty() {
            md.push_str(&format!("## 关键帧（{} 张）\n\n", frames.len()));
            md.push_str(&format!(
                "> [!abstract]- 展开全部关键帧（{} 张）\n",
                frames.len()
            ));
            for chunk in frames.chunks(4) {
                let row: Vec<String> = chunk.iter().map(|f| md_img(f)).collect();
                md.push_str(&format!("> {}\n", row.join(" ")));
            }
            md.push('\n');
        }

        // ---- 配图 ----
        if !images.is_empty() {
            md.push_str("## 配图\n\n");
            for chunk in images.chunks(4) {
                let row: Vec<String> = chunk.iter().map(|f| md_img(f)).collect();
                md.push_str(&row.join(" "));
                md.push_str("\n\n");
            }
        }

        // ---- 爆文拆解（.md，wikilink 可正常嵌入） ----
        if decon.is_some() {
            md.push_str("## 爆文拆解\n\n");
            md.push_str(&format!("![[{vault}爆文拆解]]\n\n"));
        }

        // ---- 视频（折叠，避免一开笔记就加载大文件） ----
        if let Some(v) = &video_name {
            // 视频嵌入**不能**放进 `<details>`：Obsidian 不解析 HTML 块里的 wikilink，
            // 会把 `![[视频.mp4]]` 当纯文本原样显示（实测）。图片用 HTML `<img>` 没事，
            // 但 `![[...]]` 是 Obsidian 自己的语法，必须裸放在正文里。
            md.push_str("## 视频\n\n");
            md.push_str(&format!("![[{vault}{v}]]\n\n"));
        }

        md.push_str("---\n\n");
        md.push_str(&format!(
            "- 完整字段：[帖子信息.json]({})\n",
            path_of("帖子信息.json")
        ));
        md
    };

    // 1) 始终写在产物目录（portable，同目录相对引用）
    let note_path = l4.join("knowledge.md");
    // 旧版本叫 `Beav知识卡.md`：改名后清掉旧文件，避免同一目录出现两张卡（都是生成物，可安全删）
    let legacy_note = l4.join("Beav知识卡.md");
    if legacy_note.is_file() {
        let _ = std::fs::remove_file(&legacy_note);
    }
    if let Err(e) = std::fs::write(&note_path, render("", "")) {
        println!("[monitor] write obsidian card failed: {e}");
    }

    // 2) 配置了资料库：产物在库内 → 额外在库内 hub 再写一份（vault 相对引用）
    let vault = with_store(state, |s| Ok(s.monitor_obsidian_vault.clone()))
        .unwrap_or_default();
    let vault = vault.trim().to_string();
    if vault.is_empty() {
        return;
    }
    let vault_path = PathBuf::from(&vault);
    if !vault_path.is_dir() {
        println!("[monitor] monitor_obsidian_vault 不是有效目录：{vault}");
        return;
    }
    let (l4_vault_rel, l4_root_rel) =
        match (l4.strip_prefix(&vault_path), l4.strip_prefix(root)) {
            (Ok(v), Ok(r)) => (v, r),
            _ => {
                println!(
                    "[monitor] 导出目录不在 Obsidian 资料库内（vault={vault}），无法在库内生成可解析的笔记；\
                     建议把「监控导出目标」设为该资料库内的目录。"
                );
                return;
            }
        };
    let hub_dir = vault_path.join("Beav导出").join(l4_root_rel);
    if let Err(e) = std::fs::create_dir_all(&hub_dir) {
        println!("[monitor] create obsidian hub dir failed: {e}");
        return;
    }
    // hub 笔记在 <vault>/Beav导出/<相对路径>/，产物在 <vault>/<导出根相对路径>/<相对路径>/
    // wikilink 用库根相对路径；HTML/Markdown 相对引用要回退到库根再下去
    let vault_prefix = format!("{}/", l4_vault_rel.to_string_lossy().replace('\\', "/"));
    let rel_prefix = {
        let depth = 1 + l4_root_rel.components().count();
        let mut p = "../".repeat(depth);
        p.push_str(&l4_vault_rel.to_string_lossy().replace('\\', "/"));
        p.push('/');
        p
    };
    let legacy_hub = hub_dir.join("Beav知识卡.md");
    if legacy_hub.is_file() {
        let _ = std::fs::remove_file(&legacy_hub);
    }
    let hub_note = hub_dir.join("knowledge.md");
    match std::fs::write(&hub_note, render(&rel_prefix, &vault_prefix)) {
        Ok(_) => println!("[monitor] obsidian hub card -> {}", hub_note.display()),
        Err(e) => println!("[monitor] write obsidian hub card failed: {e}"),
    }
}

fn try_export_post_product(
    state: &State<'_, AppState>,
    post_id: &str,
) -> Result<(), String> {
    let post = with_store(state, |store| {
        Ok(store
            .monitored_posts
            .iter()
            .find(|p| p.id == post_id)
            .cloned())
    })?;
    let post = match post {
        Some(p) => p,
        None => return Err("动态不存在".to_string()),
    };
    let analysis = with_store(state, |store| {
        Ok(store
            .post_video_analyses
            .iter()
            .find(|a| a.post_id == post_id)
            .cloned())
    })?;
    let decon = with_store(state, |store| {
        Ok(store
            .post_deconstructions
            .iter()
            .find(|d| d.post_id == post_id)
            .cloned())
    })?;

    // 导出根 → 平台 → 日期 → 作者 → 标题前10字
    let root = resolve_export_target(state)?;
    let date = export_date_yyyymmdd(&post);
    let author = {
        let a = sanitize_fs_name(&post.author);
        if a.is_empty() {
            "未知作者".to_string()
        } else {
            a.chars().take(32).collect()
        }
    };
    let l1 = root.join(platform_label(&post.platform));
    let l2 = l1.join(&date);
    let l3 = l2.join(&author);

    let title_prefix = title_export_prefix(&post.title, 10);
    let l4 = match find_own_export_dir(&l3, post_id) {
        Some(d) => d,
        None => {
            let candidate = l3.join(&title_prefix);
            if candidate.exists() {
                l3.join(format!("{title_prefix}_{post_id}"))
            } else {
                candidate
            }
        }
    };
    std::fs::create_dir_all(&l4).map_err(|e| format!("创建产物目录失败：{e}"))?;
    let _ = std::fs::write(l4.join(".beav-post-id"), post_id);

    // 原图
    for (idx, img) in post.images.iter().enumerate() {
        let src = Path::new(img);
        if !src.exists() {
            continue;
        }
        let ext = src
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("jpg")
            .to_lowercase();
        let dst = l4.join(format!("图片_{}.{}", idx + 1, ext));
        copy_if_changed(src, &dst);
    }

    // 抽帧图 + 视频分析文本
    if let Some(a) = &analysis {
        for (idx, fr) in a.frames.iter().enumerate() {
            let src = Path::new(fr);
            if !src.exists() {
                continue;
            }
            let ext = src
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("jpg")
                .to_lowercase();
            let dst = l4.join(format!("抽帧_{:03}.{}", idx + 1, ext));
            copy_if_changed(src, &dst);
        }
        if !a.transcript.trim().is_empty() {
            let _ = std::fs::write(l4.join("转写全文.txt"), &a.transcript);
        }
        if !a.transcript_srt.trim().is_empty() {
            let _ = std::fs::write(l4.join("转写时间轴.srt"), &a.transcript_srt);
        }
        if !a.summary.trim().is_empty() {
            let _ = std::fs::write(l4.join("视频摘要.txt"), &a.summary);
        }
    }

    // 正文
    if !post.content.trim().is_empty() {
        let _ = std::fs::write(l4.join("正文.txt"), &post.content);
    }

    // 视频本体：只导出一份 `视频.mp4`。
    // 优先轻量预览（720p/H.264/faststart）；但**预览只在需要转码时才生成**——
    // H.264 ≤1080p 的原片直接可播，`video_preview_path` 为空，此时回退原片，
    // 否则这类帖子会导出成「没有视频」。
    let video_src = post
        .video_preview_path
        .as_ref()
        .filter(|p| Path::new(p).exists())
        .or(post.video_local_path.as_ref());
    if let Some(vp) = video_src {
        let src = Path::new(vp);
        if src.exists() {
            let ext = src
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("mp4")
                .to_lowercase();
            copy_if_changed(src, &l4.join(format!("视频.{ext}")));
        }
    }

    // 爆文拆解（六要素）
    if let Some(d) = &decon {
        let json = serde_json::to_string_pretty(d).unwrap_or_default();
        let _ = std::fs::write(l4.join("爆文拆解.json"), json);
        let md = format!(
            "# 爆文拆解 · {}\n\n作者：{}\n\n## 选题角度\n{}\n\n## 文案结构\n{}\n\n## 开场白 / 前3秒钩子\n{}\n\n## 标题句式\n{}\n\n## 金句 / 钩子\n{}\n\n## 可复用方法论\n{}\n\n---\n\n### 拆解所用原文\n{}\n",
            d.post_title,
            d.author,
            d.topic_angle,
            d.copy_structure,
            d.opening,
            d.title_patterns
                .iter()
                .map(|s| format!("- {s}"))
                .collect::<Vec<_>>()
                .join("\n"),
            d.hooks
                .iter()
                .map(|s| format!("- {s}"))
                .collect::<Vec<_>>()
                .join("\n"),
            d.reusable_takeaway,
            d.raw_content
        );
        let _ = std::fs::write(l4.join("爆文拆解.md"), md);
    }

    // 帖子信息（溯源）
    let meta = json!({
        "postId": post.id,
        "platform": post.platform,
        "author": post.author,
        "title": post.title,
        "postUrl": post.post_url,
        "mediaType": post.media_type,
        "publishedAt": post.published_at,
        "createdAt": post.created_at,
        "likes": post.likes,
        "comments": post.comments,
        "collects": post.collects,
        "deconstructed": post.deconstructed,
        "videoLocalPath": post.video_local_path,
        "videoPreviewPath": post.video_preview_path,
        "hasVideoAnalysis": analysis.is_some(),
        "hasDeconstruction": decon.is_some(),
        "exportedAt": now_i64(),
    });
    let _ = std::fs::write(
        l4.join("帖子信息.json"),
        serde_json::to_string_pretty(&meta).unwrap_or_default(),
    );

    // 生成 Obsidian 聚合笔记（「知识卡」），用 ![[嵌入]] 关联所有产物
    write_obsidian_card(state, &root, &l4, &post, &analysis, &decon);

    println!("[monitor] exported post product -> {}", l4.display());
    Ok(())
}

/// 读取全局「导出根目录」（`monitor:get-export-root`）。
///
/// 返回 `{ success, root }`；`root` 为空表示跟随工作区根。
fn get_export_root(state: &State<'_, AppState>) -> Result<Value, String> {
    with_store(state, |store| {
        Ok(json!({ "success": true, "root": store.monitor_export_root.clone() }))
    })
}

/// 设置全局「导出根目录」（`monitor:set-export-root`）。
///
/// 入参：`root`（绝对目录；空串 / null 表示清空、回退工作区根）。
fn set_export_root(state: &State<'_, AppState>, payload: &Value) -> Result<Value, String> {
    let root = str_field(payload, "root")
        .or_else(|| str_field(payload, "dir"))
        .unwrap_or_default();
    let root = root.trim().to_string();
    // 目录不存在时直接创建（用户可能选了一个稍后被删掉/尚未创建的目录），
    // 创建失败才报错，避免「存了设置但导出静默回退工作区」。
    if !root.is_empty() {
        std::fs::create_dir_all(&root).map_err(|e| format!("创建导出根目录失败：{root}（{e}）"))?;
    }
    with_store_mut(state, |store| {
        store.monitor_export_root = root.clone();
        Ok(json!({ "success": true, "root": root }))
    })
}

/// 弹出系统目录选择器，返回所选目录（`monitor:pick-export-root`）。
fn pick_export_root() -> Result<Value, String> {
    let selected = crate::pick_files_native("选择监控导出根目录", true, false)?;
    let path = selected.first().map(|item| item.display().to_string());
    Ok(json!({
        "success": path.is_some(),
        "canceled": path.is_none(),
        "path": path,
    }))
}

/// 读取全局「Obsidian 资料库路径」（`monitor:get-obsidian-vault`）。
///
/// 返回 `{ success, vault }`；`vault` 为空表示未配置（导出只生成可移植的聚合笔记，
/// 需用户自行把导出目录拖进 vault 才会产生图谱边）。
fn get_obsidian_vault(state: &State<'_, AppState>) -> Result<Value, String> {
    with_store(state, |store| {
        Ok(json!({ "success": true, "vault": store.monitor_obsidian_vault.clone() }))
    })
}

/// 设置全局「Obsidian 资料库路径」（`monitor:set-obsidian-vault`）。
///
/// 入参：`vault`（绝对目录；空串 / null 表示清空）。非空时必须存在且为目录。
fn set_obsidian_vault(state: &State<'_, AppState>, payload: &Value) -> Result<Value, String> {
    let vault = str_field(payload, "vault")
        .or_else(|| str_field(payload, "dir"))
        .unwrap_or_default();
    let vault = vault.trim().to_string();
    if !vault.is_empty() && !Path::new(&vault).is_dir() {
        return Err(format!("Obsidian 资料库路径不存在：{vault}"));
    }
    with_store_mut(state, |store| {
        store.monitor_obsidian_vault = vault.clone();
        Ok(json!({ "success": true, "vault": vault }))
    })
}

/// 弹出系统目录选择器，返回所选 Obsidian 资料库目录（`monitor:pick-obsidian-vault`）。
fn pick_obsidian_vault() -> Result<Value, String> {
    let selected = crate::pick_files_native("选择 Obsidian 资料库（.obsidian 所在目录）", true, false)?;
    let path = selected.first().map(|item| item.display().to_string());
    Ok(json!({
        "success": path.is_some(),
        "canceled": path.is_none(),
        "path": path,
    }))
}

/// 「分析/转写」完成后删除 monitor-videos 里的原片，仅保留预览。**默认行为，不再提供开关**。
///
/// 唯一守卫：**必须存在独立预览文件**且与原片不是同一路径 —— 否则删掉就没视频了
/// （H.264 ≤1080p 的帖子不生成预览，直接复用原片播放）。
///
/// 删后把 `video_local_path` 与分析的 `local_path` 都改指预览，保证播放与「重新分析」仍可用。
fn delete_original_after_analysis(state: &State<'_, AppState>, post_id: &str) {
    let post = match with_store(state, |store| {
        Ok(store
            .monitored_posts
            .iter()
            .find(|p| p.id == post_id)
            .cloned())
    }) {
        Ok(Some(p)) => p,
        _ => return,
    };
    let (Some(local), Some(preview)) = (
        post.video_local_path.clone(),
        post.video_preview_path.clone(),
    ) else {
        return;
    };
    if local == preview {
        return;
    }
    let (lp, pp) = (Path::new(&local), Path::new(&preview));
    if !lp.exists() || !pp.exists() {
        return;
    }
    let preview_size = std::fs::metadata(pp).map(|m| m.len() as i64).unwrap_or(0);
    match std::fs::remove_file(lp) {
        Ok(()) => {
            let _ = with_store_mut(state, |store| {
                if let Some(slot) = store.monitored_posts.iter_mut().find(|p| p.id == post_id) {
                    slot.video_local_path = Some(preview.clone());
                    if preview_size > 0 {
                        slot.video_size = Some(preview_size);
                    }
                    slot.updated_at = now_i64().to_string();
                }
                if let Some(rec) = store
                    .post_video_analyses
                    .iter_mut()
                    .find(|a| a.post_id == post_id)
                {
                    rec.local_path = preview.clone();
                    rec.updated_at = now_i64();
                }
                Ok(())
            });
            println!("[monitor] deleted original video for {post_id}, kept preview: {preview}");
        }
        Err(e) => println!("[monitor] delete original video failed for {post_id}: {e}"),
    }
}

/// 手动触发单篇帖子产物导出（`monitor:export-post`）。
fn export_post_command(
    state: &State<'_, AppState>,
    payload: &Value,
) -> Result<Value, String> {
    let post_id = str_field(payload, "postId")
        .or_else(|| str_field(payload, "post_id"))
        .ok_or_else(|| "缺少 postId".to_string())?;
    export_post_product(state, &post_id);
    Ok(json!({ "success": true, "postId": post_id }))
}

/// 读取某帖的视频分析沉淀（字幕 + 抽帧 + 摘要）。
fn video_analysis(state: &State<'_, AppState>, payload: &Value) -> Result<Value, String> {
    let id = str_field(payload, "id").ok_or_else(|| "缺少 id".to_string())?;
    let rec = with_store(state, |store| {
        Ok(store.post_video_analyses.iter().find(|a| a.post_id == id).cloned())
    })?;
    Ok(json!({
        "success": true,
        "analysis": rec.map(|r| serde_json::to_value(&r).unwrap_or_else(|_| json!({}))),
    }))
}

/// 每日 00:00 批量预取的**增量窗口下限**。
///
/// 「单次抓取条数」默认只有 12，那是给手动点「抓取」用的单次窗口。每日预取要覆盖
/// 一整天的增量，博主单日爆更一旦超过窗口大小，主页里排在窗口之外的帖子
/// 既不在今天的抓取范围、昨天也没抓到 → 永久漏抓。所以预取窗口取
/// 「用户设置值」与该下限的**较大者**，并在汇总里回显实际使用的 limit。
const DAILY_PREFETCH_MIN_LIMIT: usize = 30;

/// 每日预取**存量回填**的条数上限（每个博主每次）。
///
/// 预取只补新帖的话，历史上「从没打开过」的帖永远没有正文（lazy 抓取的遗留），
/// 用户点开仍要现场抓 → 不满足「点开直接看到正文」。这里每晚额外回填一批存量，
/// 逐日消化 backlog。回填对象 = 从没抓过详情（`detail_fetched == false` 且正文为空）的帖，
/// 按时间**倒序**（新的优先，用户更可能先看最近的）。
const DAILY_PREFETCH_BACKFILL_LIMIT: usize = 10;

/// 「补历史」标记的版本前缀。
///
/// v1（有 bug 的版本）在**深翻一条都没抓到**时也会打标记 —— 例如该平台 Chrome 当时没连上，
/// `fetch_monitor` 提前返回 `scanned: 0`。这会把一次临时故障固化成**永久**的「已补过」，
/// 之后无论手动点还是每晚自动跑都不会再补。
///
/// 改成版本化标记：v1 打的标记不带前缀 → 一律视为**无效**（自动补历史会重试），
/// v2 之后只在真抓到内容时才打标记（带 `v2:` 前缀）。
const HISTORY_BACKFILL_FLAG_VERSION: &str = "v2";

/// 判断「已补历史」标记是否有效（只有 v2 之后、且真的抓到了内容的才算）。
fn history_backfill_valid(flag: Option<&str>) -> bool {
    flag.map(|value| value.starts_with(&format!("{HISTORY_BACKFILL_FLAG_VERSION}:")))
        .unwrap_or(false)
}

/// 「补历史」统一上限：主页深翻最大条数 + 单次补正文条数（两者共用）。
///
/// 曾用两个独立常量（旧的 `HISTORY_BACKFILL_LIMIT` / `HISTORY_BACKFILL_DETAIL_LIMIT`，各 200），
/// 导致「滚动能翻出 300 条、detail 只补 200 条」的错位。现合并为单一「近期窗口」上限：
/// 滚动最多抓 `BACKFILL_MAX` 条，本次抓到的帖**全部**补正文，再用存量未抓正文的旧帖补满到
/// `BACKFILL_MAX`。1000+ 帖的博主默认只覆盖最新 500 条（旧帖不抓）；要全量把此值调大即可
/// （每条详情要开一个 CDP tab，耗时线性增加）。
const BACKFILL_MAX: usize = 500;

/// 补历史时并行处理视频的 worker 数。
///
/// 取流、下载、转写都是长任务，串行处理上百条视频耗时很久。多 worker 并行按倍数省时间，
/// 每个 worker 各占 1 个 CDP tab + 1 路 ffmpeg。3 路是实测拐点：再往上并发打开
/// douyin/xhs 视频页容易触发风控 / 取流超时，而磁盘与网络已经接近饱和。
/// 调大更快，但建议保持在 1~4。
const VIDEO_PARALLEL_WORKERS: usize = 3;

/// 监控抓取（每日预取 / 补历史）的统计累加器。
///
/// 把「新帖补全」与「存量回填」两条路径的计数合并到一处。
#[derive(Default)]
struct MonitorFetchStats {
    watched: i64,
    new_posts: i64,
    detail_ok: i64,
    detail_skipped: i64,
    skip_note: Option<String>,
    videos: i64,
    transcribed: i64,
    failed: i64,
    /// 回收的僵尸「downloading」视频数（上次进程被杀留下的）
    recovered_downloading: i64,
    /// 本轮回填的存量帖数
    backfilled: i64,
    /// 待处理的历史视频帖数（历史视频不自动下载，交给用户手动）
    pending_videos: i64,
    /// 因已有「已补历史」标记而被跳过的博主数
    already_backfilled: i64,
    /// 触到深翻上限（BACKFILL_MAX=500）、历史可能还有更多的博主数
    limit_reached: i64,
}

impl MonitorFetchStats {
    /// 补全单帖的正文 / 媒体类型，返回是否**真的落库**。
    ///
    /// 注意：`post_detail` 在「Chrome 未连接 / 页面未解析」时**也返回 Ok**（原因在 note 里），
    /// 所以不能拿 Ok 当成功——必须回读记录看 content / detail_fetched 是否真的落库。
    /// 否则统计会说「已补全」而实际正文还是空的（用户点开看不到正文，且无从排查）。
    fn enrich_detail(&mut self, state: &State<'_, AppState>, id: &str) -> bool {
        let mut detail_note: Option<String> = None;
        match post_detail(state, &json!({ "id": id, "refresh": false })) {
            Ok(result) => {
                detail_note = result
                    .get("note")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(String::from);
            }
            Err(error) => {
                self.failed += 1;
                crate::logging::emit_legacy_line(
                    crate::logging::event::LogSource::Host,
                    crate::logging::event::LogLevel::Warn,
                    "monitor",
                    "monitor.daily.detail_failed",
                    format!("[monitor daily] detail fetch for {id} failed: {error}"),
                    json!({ "postId": id, "error": error }),
                    None,
                );
            }
        }
        let enriched = with_store(state, |store| {
            Ok(store
                .monitored_posts
                .iter()
                .find(|p| p.id == id)
                .map(|p| p.detail_fetched || !p.content.trim().is_empty())
                .unwrap_or(false))
        })
        .unwrap_or(false);
        if enriched {
            self.detail_ok += 1;
        } else {
            self.detail_skipped += 1;
            if self.skip_note.is_none() {
                self.skip_note = detail_note;
            }
        }
        enriched
    }
}

/// 回收僵尸「downloading」视频状态。
///
/// `download_video` 开始时把状态置为 `downloading`，成功才改 `done`、失败改 `failed`。
/// 若进程在下载途中被杀（重启 / 崩溃），状态会**永远卡在 downloading**。
/// 而自动下载的守卫是「仅 `none` 才下」→ 这条视频将再也不会被自动重试，永久卡死。
/// 每日预取是后台单线程串行执行，不会有并发下载，所以这里无条件把残留的
/// `downloading` 复位成 `none`，让本轮重新下载。
fn recover_stuck_downloading(state: &State<'_, AppState>) -> Result<i64, String> {
    with_store_mut(state, |store| {
        let mut recovered = 0i64;
        for post in store.monitored_posts.iter_mut() {
            if post.video_status == "downloading" {
                post.video_status = "none".to_string();
                recovered += 1;
            }
        }
        Ok(recovered)
    })
}

/// 每日 00:00 批量预取：列表抓取（按 URL 去重，仅新增）→ 逐条补全正文/媒体类型 →
/// 视频帖自动下载 + 语音转写，保证用户次日 09:00 打开即见全文、视频已转写。
///
/// 设计要点：
/// - 复用 `fetch_monitor`（URL 去重，仅回传新帖），ad-hoc 逻辑完全不动；
/// - `post_detail` 先把 `detail_fetched` / `content` / `media_type` 落库，二次打开秒开；
/// - 额外**回填存量**：从没抓过详情的旧帖也补上（否则点开仍要现场抓）；
/// - 仅当 `media_type == "video"` 且平台为小红书 / 抖音时，才 `download_video` + `analyze_video`；
/// - 视频仅在该帖 `video_status == "none"` 时自动下（幂等：已下过 / 失败过不重复下）；
/// - 每条帖独立 try，单帖失败不阻断其余；全程在常驻调度器的 `spawn_blocking` 内同步执行。
pub fn run_daily_prefetch(app: AppHandle) -> Result<Value, String> {
    let state = app.state::<AppState>();
    let now = crate::now_i64();
    // 顺手清理上轮残留的孤儿 SQLite WAL/回滚副文件（贴合「每日」节奏，保证垃圾不积累）。
    let _ = crate::sqlite_cleanup::cleanup_orphaned_sqlite_sidecars(&app);
    let mut stats = MonitorFetchStats::default();

    // 先回收上次残留的僵尸 downloading 状态，否则那批视频永远不会被自动重试
    stats.recovered_downloading = recover_stuck_downloading(&state)?;

    let watches: Vec<crate::CompetitorWatchRecord> = with_store(&state, |store| {
        Ok(store
            .competitor_watchlist
            .iter()
            .filter(|watch| watch.enabled)
            .cloned()
            .collect())
    })?;

    // 每日增量窗口：取「用户设置的单次抓取条数」与下限的较大者
    let daily_limit = resolve_scan_limit(&state, &json!({})).max(DAILY_PREFETCH_MIN_LIMIT);

    // —— 自动补历史（仅一次）——
    // 从没补过历史的博主（首次接入 / 老数据）先深翻一次主页，把更早的历史帖一次性落库。
    // 补过之后打上 `history_backfilled_at` 标记，后续每晚不再重复深翻（深翻要滚 ~35s，不能天天做）。
    let pending_history: Vec<String> = watches
        .iter()
        .filter(|watch| watch.history_backfilled_at.is_none())
        .map(|watch| watch.id.clone())
        .collect();
    for watch_id in pending_history {
        match run_history_backfill(
            app.clone(),
            &json!({ "watchId": watch_id, "quiet": true }),
        ) {
            Ok(result) => {
                let added = result.get("newPosts").and_then(Value::as_i64).unwrap_or(0);
                let enriched = result.get("detailEnriched").and_then(Value::as_i64).unwrap_or(0);
                if added > 0 {
                    println!(
                        "[monitor daily] history backfill for {watch_id}: {added} posts, {enriched} enriched"
                    );
                }
            }
            Err(error) => {
                stats.failed += 1;
                crate::logging::emit_legacy_line(
                    crate::logging::event::LogSource::Host,
                    crate::logging::event::LogLevel::Warn,
                    "monitor",
                    "monitor.daily.history_failed",
                    format!("[monitor daily] history backfill failed: {error}"),
                    json!({ "watchId": watch_id, "error": error }),
                    None,
                );
            }
        }
    }

    for watch in &watches {
        stats.watched += 1;
        let fetch = match fetch_monitor(
            &state,
            &json!({ "watchId": watch.id, "limit": daily_limit }),
            None,
            false,
        ) {
            Ok(value) => value,
            Err(error) => {
                stats.failed += 1;
                crate::logging::emit_legacy_line(
                    crate::logging::event::LogSource::Host,
                    crate::logging::event::LogLevel::Warn,
                    "monitor",
                    "monitor.daily.fetch_failed",
                    format!(
                        "[monitor daily] list fetch for {} failed: {}",
                        watch.name, error
                    ),
                    json!({ "watchId": watch.id, "error": error }),
                    None,
                );
                continue;
            }
        };
        let created = fetch
            .get("created")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        stats.new_posts += created.len() as i64;

        // 1) 新帖：补全正文 / 媒体类型（让 09:00 打开秒开）
        let mut targets: Vec<String> = Vec::new();
        for post in &created {
            if let Some(id) = post.get("id").and_then(Value::as_str) {
                targets.push(id.to_string());
            }
        }

        // 2) 存量回填：历史上从没抓过详情的旧帖（lazy 抓取遗留），点开仍要现场抓。
        //    只回填小红书/抖音（只有这两个平台支持本地抓正文），按时间倒序取前 N 条。
        let backlog: Vec<String> = with_store(&state, |store| {
            let mut pending: Vec<(i64, String)> = store
                .monitored_posts
                .iter()
                .filter(|p| p.watch_id.as_deref() == Some(watch.id.as_str()))
                .filter(|p| Platform::parse(&p.platform).map(|x| x.is_cdp()).unwrap_or(false))
                .filter(|p| !p.detail_fetched && p.content.trim().is_empty())
                .map(|p| {
                    (
                        p.created_at.parse::<i64>().unwrap_or(0),
                        p.id.clone(),
                    )
                })
                .collect();
            // 新的优先（用户更可能先看最近的）
            pending.sort_by(|a, b| b.0.cmp(&a.0));
            Ok(pending
                .into_iter()
                .take(DAILY_PREFETCH_BACKFILL_LIMIT)
                .map(|(_, id)| id)
                .collect())
        })?;
        stats.backfilled += backlog.len() as i64;
        for id in backlog {
            if !targets.contains(&id) {
                targets.push(id);
            }
        }

        for id in targets {
            let enriched = stats.enrich_detail(&state, &id);
            // 正文没抓到就别折腾视频了（大概率是 Chrome 没连上）
            if !enriched {
                continue;
            }

            // 3) 视频帖自动下载 + 转写（仅小红书 / 抖音，且尚未下载过）
            let rec = with_store(&state, |store| {
                Ok(store.monitored_posts.iter().find(|p| p.id == id).cloned())
            })?;
            let (is_video, platform_ok, status_none) = rec
                .as_ref()
                .map(|r| {
                    (
                        r.media_type == "video" || r.post_url.contains("/video/"),
                        Platform::parse(&r.platform).map(|p| p.is_cdp()).unwrap_or(false),
                        r.video_status == "none",
                    )
                })
                .unwrap_or((false, false, false));

            if is_video && platform_ok && status_none {
                match download_video(&app, &state, &json!({ "id": id })) {
                    Ok(_) => {
                        stats.videos += 1;
                        match analyze_video(&app, &state, &json!({ "id": id })) {
                            Ok(_) => stats.transcribed += 1,
                            Err(error) => {
                                stats.failed += 1;
                                crate::logging::emit_legacy_line(
                                    crate::logging::event::LogSource::Host,
                                    crate::logging::event::LogLevel::Warn,
                                    "monitor",
                                    "monitor.daily.transcribe_failed",
                                    format!(
                                        "[monitor daily] transcribe for {id} failed: {error}"
                                    ),
                                    json!({ "postId": id, "error": error }),
                                    None,
                                );
                            }
                        }
                    }
                    Err(error) => {
                        stats.failed += 1;
                        crate::logging::emit_legacy_line(
                            crate::logging::event::LogSource::Host,
                            crate::logging::event::LogLevel::Warn,
                            "monitor",
                            "monitor.daily.download_failed",
                            format!("[monitor daily] video download for {id} failed: {error}"),
                            json!({ "postId": id, "error": error }),
                            None,
                        );
                    }
                }
            }
        }
    }

    let summary = json!({
        "success": true,
        "watched": stats.watched,
        // 实际使用的增量窗口（每博主每次抓取的上限）
        "limit": daily_limit,
        "newPosts": stats.new_posts,
        "detailEnriched": stats.detail_ok,
        // 正文没抓到的条数 + 首个原因（如「Chrome 未连接」），便于一眼定位为什么打开不秒显
        "detailSkipped": stats.detail_skipped,
        "skipNote": stats.skip_note,
        // 本轮回填的存量帖数
        "backfilled": stats.backfilled,
        // 回收的僵尸 downloading 数
        "recoveredDownloading": stats.recovered_downloading,
        "videosDownloaded": stats.videos,
        "videosTranscribed": stats.transcribed,
        "failed": stats.failed,
        "ranAt": now,
    });
    println!("[monitor daily] prefetch summary: {}", summary);
    // 通知前端：常驻 APP 的帖子列表是内存旧快照，预取补全正文/视频后必须刷新，
    // 否则用户次日打开仍是「未抓正文」状态（openDetail 命中不到本地缓存 → 不秒开）。
    let _ = app.emit("monitor:prefetch-done", summary.clone());
    Ok(summary)
}

/// 历史视频并行处理：把 `video_ids` 分给若干 worker 线程，逐个「下载 → 抽帧/转写」。
///
/// 由 `run_history_backfill` 在**补正文之前**调用，与 detail 抓取重叠进行：
/// 总耗时从 `detail + video` 降到约 `max(detail, video)`。
/// - 每个视频独立 CDP target（`navigate_and_eval_settle` 内部 `create_target`），与 detail 互不干扰；
/// - 每条独立容错：下载失败跳过转写，但进度照常推进（不会卡在某个坏链接）；
/// - 进度用原子计数累加，`fetch_add` 返回值唯一 → `videos-done` 恰好触发一次。
fn spawn_video_workers(app: AppHandle, watch_id: String, video_ids: Vec<String>) {
    spawn_video_workers_ex(app, watch_id, video_ids, false)
}

/// 带模式的视频 worker：`patch_only` 为真时只补音轨（视频轨不重下、帧不重抽）。
fn spawn_video_workers_ex(
    app: AppHandle,
    watch_id: String,
    video_ids: Vec<String>,
    patch_only: bool,
) {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicI64, Ordering};
    use std::sync::{Arc, Mutex};

    let total = video_ids.len() as i64;
    if total <= 0 {
        return;
    }
    let workers = VIDEO_PARALLEL_WORKERS.min(total as usize).max(1);
    let queue = Arc::new(Mutex::new(video_ids.into_iter().collect::<VecDeque<String>>()));
    let done = Arc::new(AtomicI64::new(0));
    // 下载成功（已抽帧）的 id 收集起来，等下载阶段整体结束后再串行补转写。
    // 转写要等 ASR（单条 10~40s），串进下载 worker 会把 3 路并行拖成事实上的 1 路。
    let pending_transcribe = Arc::new(Mutex::new(Vec::<String>::new()));

    // 先推一条 0/total，让前端进度条立刻出现（否则要等第一个视频跑完才有事件）
    let _ = app.emit(
        "monitor:backfill-progress",
        json!({ "watchId": watch_id, "scanned": 0, "total": total, "phase": "videos" }),
    );

    for _ in 0..workers {
        let app = app.clone();
        let wid = watch_id.clone();
        let wid_log = watch_id.clone();
        let queue = Arc::clone(&queue);
        let done = Arc::clone(&done);
        let pending_transcribe = Arc::clone(&pending_transcribe);
        let spawned = std::thread::Builder::new()
            .name("backfill-videos".to_string())
            .spawn(move || {
                let state = app.state::<AppState>();
                loop {
                    let next = queue.lock().ok().and_then(|mut q| q.pop_front());
                    let Some(id) = next else { break };
                    let id_for_analyze = id.clone();
                    if patch_only {
                        // 只补音轨：帧 / 分析记录都沿用已有的，补完直接排队转写。
                        if let Err(error) =
                            download_video(&app, &state, &json!({ "id": id, "patchAudio": true }))
                        {
                            println!("[monitor history] patch audio {id} failed: {error}");
                        } else if let Ok(mut list) = pending_transcribe.lock() {
                            list.push(id_for_analyze);
                        }
                    } else if let Err(error) = download_video(&app, &state, &json!({ "id": id })) {
                        println!("[monitor history] background video download {id} failed: {error}");
                    } else if let Err(error) = analyze_video(
                        &app,
                        &state,
                        &json!({ "id": id_for_analyze, "skipTranscript": true }),
                    ) {
                        println!("[monitor history] background video analyze {id} failed: {error}");
                    } else if let Ok(mut list) = pending_transcribe.lock() {
                        list.push(id_for_analyze);
                    }
                    let finished = done.fetch_add(1, Ordering::Relaxed) + 1;
                    let _ = app.emit(
                        "monitor:backfill-progress",
                        json!({ "watchId": wid, "scanned": finished, "total": total, "phase": "videos" }),
                    );
                    if finished >= total {
                        let _ = app.emit(
                            "monitor:backfill-progress",
                            json!({ "watchId": wid, "scanned": total, "total": total, "phase": "videos-done" }),
                        );
                        let _ = app.emit(
                            "monitor:prefetch-done",
                            json!({ "source": "backfill-videos", "watchId": wid, "videos": total }),
                        );
                        println!("[monitor history] background videos finished: {total}");
                        let batch = pending_transcribe
                            .lock()
                            .map(|mut list| std::mem::take(&mut *list))
                            .unwrap_or_default();
                        spawn_transcribe_workers(app.clone(), wid.clone(), batch);
                    }
                }
            });
        if let Err(error) = spawned {
            crate::logging::emit_legacy_line(
                crate::logging::event::LogSource::Host,
                crate::logging::event::LogLevel::Warn,
                "monitor",
                "monitor.history.video_thread_failed",
                format!("[monitor history] spawn video worker failed: {error}"),
                json!({ "watchId": wid_log, "error": error.to_string() }),
                None,
            );
        }
    }
}

/// 转写并行度。ASR 服务普遍有并发限流（fun-asr 一类），2 路是稳妥值。
/// 与下载并行度分开：下载受网络/磁盘约束，转写受服务端限流约束，混在一起调只会顾此失彼。
const TRANSCRIBE_PARALLEL_WORKERS: usize = 2;

/// 下载阶段结束后单独跑的语音转写队列。
///
/// 为何后置：转写要「抽音频 → 上传 → 等 ASR 返回」，单条 10~40s。串进下载 worker 会让
/// 每路 worker 在每条视频后多等几十秒，3 路并行被拖成事实上的 1 路。这里等下载全部结束
/// （帧已抽好）再接棒，两条流水线互不阻塞。
fn spawn_transcribe_workers(app: AppHandle, watch_id: String, ids: Vec<String>) {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicI64, Ordering};
    use std::sync::{Arc, Mutex};

    let total = ids.len() as i64;
    if total <= 0 {
        return;
    }
    let workers = TRANSCRIBE_PARALLEL_WORKERS.min(total as usize).max(1);
    let queue = Arc::new(Mutex::new(ids.into_iter().collect::<VecDeque<String>>()));
    let done = Arc::new(AtomicI64::new(0));

    let _ = app.emit(
        "monitor:backfill-progress",
        json!({ "watchId": watch_id, "scanned": 0, "total": total, "phase": "transcribe" }),
    );

    for _ in 0..workers {
        let app = app.clone();
        let wid = watch_id.clone();
        let queue = Arc::clone(&queue);
        let done = Arc::clone(&done);
        let _ = std::thread::Builder::new()
            .name("backfill-transcribe".to_string())
            .spawn(move || {
                let state = app.state::<AppState>();
                loop {
                    let next = queue.lock().ok().and_then(|mut q| q.pop_front());
                    let Some(id) = next else { break };
                    if let Err(error) = transcribe_video_only(&app, &state, &json!({ "id": id })) {
                        println!("[monitor history] background transcribe {id} failed: {error}");
                    }
                    let finished = done.fetch_add(1, Ordering::Relaxed) + 1;
                    let _ = app.emit(
                        "monitor:backfill-progress",
                        json!({ "watchId": wid, "scanned": finished, "total": total, "phase": "transcribe" }),
                    );
                    if finished >= total {
                        let _ = app.emit(
                            "monitor:prefetch-done",
                            json!({ "source": "backfill-transcribe", "watchId": wid, "transcribed": total }),
                        );
                        println!("[monitor history] background transcribe finished: {total}");
                    }
                }
            });
    }
}

/// 批量补音轨：给「已下载但无音轨」的视频重新取流，**只下音频轨**再合并。
///
/// 为什么单开一个入口：抖音 DASH 只给视频轨，历史下载的 54 条全是无声 mp4。
/// 整体重下要再拉 2GB+ 且每条数分钟；补音轨只多下几十 KB 音频 + 一次取流，
/// 3 路并行下几十条几分钟就能补完。补完自动排队转写。
fn patch_audio_batch(app: AppHandle, payload: &Value) -> Result<Value, String> {
    let state = app.state::<AppState>();
    let watch_id = str_field(payload, "watchId").or_else(|| str_field(payload, "watch_id"));
    let ids: Vec<String> = with_store(&state, |store| {
        Ok(store
            .monitored_posts
            .iter()
            .filter(|p| {
                watch_id
                    .as_deref()
                    .map(|w| p.watch_id.as_deref() == Some(w))
                    .unwrap_or(true)
            })
            .filter(|p| p.video_status == "done" && p.video_has_audio == Some(false))
            .map(|p| p.id.clone())
            .collect())
    })?;
    let total = ids.len() as i64;
    if total == 0 {
        return Ok(json!({
            "success": true,
            "queued": 0,
            "note": "没有需要补音轨的视频（都已含音轨，或尚未下载）",
        }));
    }
    spawn_video_workers_ex(app, watch_id.unwrap_or_default(), ids, true);
    Ok(json!({ "success": true, "queued": total }))
}

/// 补历史：把博主主页**更早的历史帖**也抓下来落库。
///
/// 日常抓取只取主页最新的 N 条（增量窗口 30），更早的历史永远进不来 ——
/// 首次接入尤其明显（只拿到最近 30 条），而且随着博主继续发新帖，
/// 那些没抓过的历史帖只会不断滑出窗口，最终永久丢失。
///
/// 这里用大 limit **深翻**主页（滚动加载到底），把能翻到的历史帖一次性落库；
/// 并限量补全一批正文（最近的优先），让最新的一批立刻能秒开。
///
/// - 目标：`payload.watchId` 指定单个博主；缺省则对全部 enabled 博主执行；
/// - `force`：无视 `history_backfilled_at` 标记强制重补（手动按钮用）；
/// - 历史视频：**自动下载 + 抽帧/转写**，且与「补正文」并行（`spawn_video_workers`），
///   进度经 `monitor:backfill-progress`(phase=videos) 推给前端；不阻塞本调用返回。
pub fn run_history_backfill(app: AppHandle, payload: &Value) -> Result<Value, String> {
    let state = app.state::<AppState>();
    let now = crate::now_i64();
    let watch_id = str_field(payload, "watchId").or_else(|| str_field(payload, "watch_id"));
    let force = payload
        .get("force")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    // quiet=true：被每日预取内部调用时不再单独发事件（外层会统一发一次）
    let quiet = payload
        .get("quiet")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let watches: Vec<crate::CompetitorWatchRecord> = with_store(&state, |store| {
        Ok(store
            .competitor_watchlist
            .iter()
            .filter(|watch| watch.enabled)
            .filter(|watch| watch_id.is_none() || watch_id.as_deref() == Some(watch.id.as_str()))
            .cloned()
            .collect())
    })?;

    let mut stats = MonitorFetchStats::default();
    let mut scanned_total = 0i64;
    // 主页声明的作品总数（进度条分母）：取各博主中的最大值
    let mut profile_total_max = 0i64;
    // 每个博主的滚动诊断（真机排查「只翻到首屏」用）
    let mut scroll_diags: Vec<(String, Value)> = Vec::new();

    for watch in &watches {
        // 已补过且不是强制 → 跳过（避免每晚重复深翻）
        if !force && history_backfill_valid(watch.history_backfilled_at.as_deref()) {
            stats.already_backfilled += 1;
            continue;
        }
        stats.watched += 1;

        // 深翻：limit 拉满，滚动加载直到翻不动
        let result = match fetch_monitor(
            &state,
            &json!({ "watchId": watch.id, "limit": BACKFILL_MAX }),
            Some(app.clone()),
            !quiet,
        ) {
            Ok(value) => value,
            Err(error) => {
                stats.failed += 1;
                crate::logging::emit_legacy_line(
                    crate::logging::event::LogSource::Host,
                    crate::logging::event::LogLevel::Warn,
                    "monitor",
                    "monitor.history.fetch_failed",
                    format!(
                        "[monitor history] deep fetch for {} failed: {}",
                        watch.name, error
                    ),
                    json!({ "watchId": watch.id, "error": error }),
                    None,
                );
                continue;
            }
        };
        let scanned = result.get("scanned").and_then(Value::as_i64).unwrap_or(0);
        let exhausted = result
            .get("historyExhausted")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        // 滚动诊断：告诉用户「为什么只翻到 N 条」（首屏就停 / 滚错容器 / 页面不给更多）
        if let Some(diag) = result.get("scrollDiag").filter(|value| !value.is_null()) {
            scroll_diags.push((watch.name.clone(), diag.clone()));
        }
        scanned_total += scanned;
        // 主页声明的作品总数（进度条分母）：取各博主中的最大值
        let watch_profile_total = result
            .get("profileTotal")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        if watch_profile_total > profile_total_max {
            profile_total_max = watch_profile_total;
        }

        // —— 关键守卫：一条都没抓到就不能打「已补历史」标记 ——
        // 典型场景：该平台 Chrome 没连上（fetch 提前返回 scanned=0）。
        // 若此时照样打标记，这个博主就被**永久**排除在补历史之外，
        // 用户手动点也只会得到「已补过」——一个临时故障变成了永久状态。
        if scanned == 0 {
            stats.failed += 1;
            let note = result
                .get("note")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("未抓到任何帖子")
                .to_string();
            if stats.skip_note.is_none() {
                stats.skip_note = Some(note.clone());
            }
            crate::logging::emit_legacy_line(
                crate::logging::event::LogSource::Host,
                crate::logging::event::LogLevel::Warn,
                "monitor",
                "monitor.history.empty_result",
                format!(
                    "[monitor history] deep fetch for {} got nothing: {}",
                    watch.name, note
                ),
                json!({ "watchId": watch.id, "note": note }),
                None,
            );
            // 不打标记 → 下一晚仍会自动重试
            continue;
        }

        let created = result
            .get("created")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        // created 顺序 = 主页 DOM 顺序 = 时间倒序（新的在前）
        stats.new_posts += created.len() as i64;

        // 深翻建出来的是**历史帖**，不该带「新抓取」徽标（一次几百条全标「新」会显得很坏）。
        // 只清 is_new 标记，created_at 等原始信息保留。
        let backfilled_ids: Vec<String> = created
            .iter()
            .filter_map(|post| post.get("id").and_then(Value::as_str).map(String::from))
            .collect();
        if !backfilled_ids.is_empty() {
            with_store_mut(&state, |store| {
                for post in store.monitored_posts.iter_mut() {
                    if backfilled_ids.iter().any(|id| id == &post.id) {
                        post.is_new = false;
                    }
                }
                Ok::<(), String>(())
            })?;
        }

        // —— 历史视频：与「补正文」并行 ——
        // 原实现要等 detail 全补完才串行下载视频，总耗时 = detail + video；
        // 现在提前启动视频任务（重叠），总耗时 ≈ max(detail, video)。
        // 取流与 detail 各开独立 CDP target（navigate_and_eval_settle 内部 create_target），互不干扰。
        // 幂等：只处理 video_status=="none" 的帖。
        let video_ids: Vec<String> = with_store(&state, |store| {
            Ok(store
                .monitored_posts
                .iter()
                .filter(|p| p.watch_id.as_deref() == Some(watch.id.as_str()))
                .filter(|p| p.media_type == "video" || p.post_url.contains("/video/"))
                .filter(|p| Platform::parse(&p.platform).map(|x| x.is_cdp()).unwrap_or(false))
                .filter(|p| p.video_status == "none")
                .map(|p| p.id.clone())
                .collect())
        })?;
        stats.pending_videos += video_ids.len() as i64;
        if !video_ids.is_empty() {
            spawn_video_workers(app.clone(), watch.id.clone(), video_ids);
        }

        // 补全正文：本次滚出的帖【全部】补正文（不再受固定 200 截断），
        // 再用「存量里还没抓过正文」的旧帖补满到 BACKFILL_MAX（近期窗口 500）。
        let mut detail_targets: Vec<String> = created
            .iter()
            .filter_map(|post| post.get("id").and_then(Value::as_str).map(String::from))
            .collect();
        // 新帖不够填满窗口时，用「存量里还没抓过正文」的补上（同样是最近的优先）
        if detail_targets.len() < BACKFILL_MAX {
            let backlog: Vec<String> = with_store(&state, |store| {
                let mut pending: Vec<(i64, String)> = store
                    .monitored_posts
                    .iter()
                    .filter(|p| p.watch_id.as_deref() == Some(watch.id.as_str()))
                    .filter(|p| Platform::parse(&p.platform).map(|x| x.is_cdp()).unwrap_or(false))
                    .filter(|p| !p.detail_fetched && p.content.trim().is_empty())
                    .map(|p| (p.created_at.parse::<i64>().unwrap_or(0), p.id.clone()))
                    .collect();
                pending.sort_by(|a, b| b.0.cmp(&a.0));
                Ok(pending
                    .into_iter()
                    .map(|(_, id)| id)
                    .filter(|id| !detail_targets.contains(id))
                    .take(BACKFILL_MAX - detail_targets.len())
                    .collect())
            })?;
            stats.backfilled += backlog.len() as i64;
            detail_targets.extend(backlog);
        }

        for id in &detail_targets {
            stats.enrich_detail(&state, id);
        }

        // 打上「已补历史」标记，后续不再每晚重复深翻。
        // 只有上面确认 scanned > 0（真的抓到了）才会走到这里 —— Chrome 没连上时不会打标记。
        with_store_mut(&state, |store| {
            if let Some(slot) = store
                .competitor_watchlist
                .iter_mut()
                .find(|entry| entry.id == watch.id)
            {
                slot.history_backfilled_at = Some(format!("{HISTORY_BACKFILL_FLAG_VERSION}:{now}"));
                slot.updated_at = now.to_string();
            }
            Ok::<(), String>(())
        })?;
        if !exhausted {
            // 触到深翻上限（BACKFILL_MAX=500）而非翻到底 → 这位博主历史可能还有更多，值得再翻
            stats.limit_reached += 1;
        }
    }

    // 汇总滚动诊断 → 一句话提示（让用户不用翻日志就知道「为什么只翻到首屏」）
    let mut hint: Option<String> = None;
    for (name, diag) in &scroll_diags {
        let scrolled = diag
            .get("scrolled")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let counts: Vec<i64> = diag
            .get("counts")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(|v| v.as_i64()).collect::<Vec<i64>>())
            .unwrap_or_default();
        let grew = counts.len() >= 2 && counts[counts.len() - 1] > counts[0];
        let text = if !scrolled {
            format!("{name}：页面没有发生滚动（滚动容器可能选错）")
        } else if !grew {
            format!(
                "{name}：滚动后没有加载出更多内容（停在 {} 条，共 {} 轮）",
                counts.first().copied().unwrap_or(0),
                counts.len()
            )
        } else {
            format!(
                "{name}：滚动加载正常（{} → {} 条）",
                counts.first().copied().unwrap_or(0),
                counts.last().copied().unwrap_or(0)
            )
        };
        if hint.is_none() {
            hint = Some(text);
        }
    }

    let summary = json!({
        "success": true,
        "watched": stats.watched,
        // 深翻时实际滚动到的条数
        "scanned": scanned_total,
        // 深翻上限
        "limit": BACKFILL_MAX,
        // 主页声明的作品总数（进度条分母；未知为 0）
        "profileTotal": profile_total_max,
        // 其中 URL 未见过的 = 真正补进来的历史帖
        "newPosts": stats.new_posts,
        "detailEnriched": stats.detail_ok,
        "detailSkipped": stats.detail_skipped,
        "skipNote": stats.skip_note,
        "backfilled": stats.backfilled,
        // 因已有标记被跳过的博主数（手动 force 时恒为 0）
        "alreadyBackfilled": stats.already_backfilled,
        // 触到 200 上限（历史可能还有更多）的博主数
        "limitReached": stats.limit_reached,
        // 滚动诊断明细（每个博主：滚了多少轮 / 每轮锚点数 / 实际滚动的容器）
        "scrollDiags": scroll_diags
            .iter()
            .map(|(name, diag)| json!({ "name": name, "diag": diag }))
            .collect::<Vec<_>>(),
        // 一句话诊断提示
        "hint": hint,
        // 历史视频已转后台线程自动「下载+转写」，这里报启动处理的数量
        "pendingVideos": stats.pending_videos,
        "failed": stats.failed,
        "ranAt": now,
    });
    println!("[monitor history] backfill summary: {}", summary);
    // 前端列表同样是内存旧快照，补完要刷新才看得到新帖
    if !quiet {
        let _ = app.emit("monitor:prefetch-done", summary.clone());
    }
    Ok(summary)
}

/// 计划任务入口：抓取所有到期的「启用中」监控项（按各自 interval_hours 判断）。
pub fn run_due_fetches(state: &State<'_, AppState>, now: i64) -> Result<Value, String> {
    let due: Vec<crate::CompetitorWatchRecord> = with_store_mut(state, |store| {
        Ok(store
            .competitor_watchlist
            .iter()
            .filter(|watch| watch.enabled && is_watch_due(watch, now))
            .cloned()
            .collect())
    })?;

    let mut checked = 0i64;
    let mut total_new = 0i64;
    for watch in &due {
        checked += 1;
        match fetch_monitor(state, &json!({ "watchId": watch.id }), None, false) {
            Ok(result) => {
                if let Some(created) = result.get("created").and_then(Value::as_array) {
                    total_new += created.len() as i64;
                }
            }
            Err(error) => {
                crate::logging::emit_legacy_line(
                    crate::logging::event::LogSource::Host,
                    crate::logging::event::LogLevel::Warn,
                    "monitor",
                    "monitor.scheduler.fetch_failed",
                    format!("[monitor scheduler] fetch for {} failed: {}", watch.name, error),
                    json!({ "watchId": watch.id, "error": error }),
                    None,
                );
            }
        }
    }
    Ok(json!({ "success": true, "checked": checked, "totalNew": total_new }))
}

fn is_watch_due(watch: &crate::CompetitorWatchRecord, now: i64) -> bool {
    match watch
        .last_fetch_at
        .as_ref()
        .and_then(|value| value.parse::<i64>().ok())
    {
        None => true,
        Some(last) => {
            let interval_ms = watch.interval_hours.max(1) * 3_600_000;
            now.saturating_sub(last) >= interval_ms
        }
    }
}

/// 启动对标监控的常驻计划任务（独立于 Redclaw runner，应用启动即生效）。
///
/// 两条常驻循环：
/// 1. 每 5 分钟检查一次，对到期且启用的监控项执行「列表抓取」（保持现有 ad-hoc / 自动逻辑，
///    只落列表、不补全正文，开销低、频次高，负责把新帖尽快纳入监控）。
/// 2. 每天本地 00:00 触发一次「批量预取」：列表(URL 去重)→逐条补全正文→视频帖下载+转写，
///    保证用户次日 09:00 打开任意新帖即可秒见全文、视频已转写（需求 #每日00点预取）。
pub fn start_monitor_scheduler(app: AppHandle) {
    // 循环 1：5 分钟到期检查（列表抓取）
    {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let mut ticker = interval(std::time::Duration::from_secs(300));
            // 跳过启动时的即时 tick，避免冷启动立即抓全量。
            ticker.tick().await;
            loop {
                ticker.tick().await;
                let app_handle = app.clone();
                let _ = tauri::async_runtime::spawn_blocking(move || {
                    let state = app_handle.state::<AppState>();
                    let now = crate::now_i64();
                    if let Err(error) = run_due_fetches(&state, now) {
                        crate::logging::emit_legacy_line(
                            crate::logging::event::LogSource::Host,
                            crate::logging::event::LogLevel::Warn,
                            "monitor",
                            "monitor.scheduler.run_due_failed",
                            format!("[monitor scheduler] run_due_fetches failed: {error}"),
                            json!({ "error": error }),
                            None,
                        );
                    }
                })
                .await;
            }
        });
    }

    // 循环 2：每天本地 00:00 批量预取（列表→补全正文→视频下载+转写）
    {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                let now = crate::now_i64();
                // 计算下一个本地 00:00（时区取本机，与 task_policy 的每日调度同源）。
                let next = match crate::scheduler::task_policy::next_daily_timestamp_in_timezone(
                    0, 0, now, None,
                ) {
                    Ok(ts) => ts,
                    Err(error) => {
                        crate::logging::emit_legacy_line(
                            crate::logging::event::LogSource::Host,
                            crate::logging::event::LogLevel::Warn,
                            "monitor",
                            "monitor.scheduler.daily_next_failed",
                            format!("[monitor scheduler] compute next 00:00 failed: {error}"),
                            json!({ "error": error }),
                            None,
                        );
                        // 兜底：1 小时后重试，避免死循环
                        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
                        continue;
                    }
                };
                let wait_ms = (next.saturating_sub(now)).max(0) as u64;
                tokio::time::sleep(std::time::Duration::from_millis(wait_ms)).await;

                let app_handle = app.clone();
                let _ = tauri::async_runtime::spawn_blocking(move || {
                    if let Err(error) = run_daily_prefetch(app_handle) {
                        crate::logging::emit_legacy_line(
                            crate::logging::event::LogSource::Host,
                            crate::logging::event::LogLevel::Warn,
                            "monitor",
                            "monitor.scheduler.daily_failed",
                            format!("[monitor scheduler] run_daily_prefetch failed: {error}"),
                            json!({ "error": error }),
                            None,
                        );
                    }
                })
                .await;
                // 跑完后循环回到顶部，重新计算下一个 00:00（= 次日），天然每天只触发一次。
            }
        });
    }
}
