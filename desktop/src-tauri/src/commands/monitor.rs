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
use crate::store::types::{DeconstructionSegment, MonitoredPostRecord, PostVideoAnalysisRecord};
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
/// Web 用户只能操作自己的对象：对带 `id` / `watchId` 的通道做归属校验。
/// 找不到对象时不在这里报错，交给具体 handler 给出更准确的提示。
fn guard_owned_target(
    state: &State<'_, AppState>,
    channel: &str,
    payload: &Value,
) -> Result<(), String> {
    if crate::identity::is_admin() {
        return Ok(());
    }
    const OWNED_CHANNELS: &[&str] = &[
        "monitor:watch-remove",
        "monitor:post-remove",
        "monitor:post-detail",
        "monitor:fetch",
        "monitor:backfill-history",
        "monitor:download-video",
        "monitor:analyze-video",
        "monitor:transcribe-video",
        "monitor:video-analysis",
        "monitor:deconstruct",
        "monitor:remix-generate",
        "monitor:inject-topic",
        "monitor:export-post",
    ];
    if !OWNED_CHANNELS.contains(&channel) {
        return Ok(());
    }

    // 删除类操作只认归属者本人：被分配（sharedTo）的用户可以查看、抓取、下载，
    // 但不能反过来删掉别人的帖子/博主。
    const OWNER_STRICT_CHANNELS: &[&str] = &["monitor:watch-remove", "monitor:post-remove"];
    let strict = OWNER_STRICT_CHANNELS.contains(&channel);

    if let Some(id) = payload
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        // 返回 (归属人, 被分配给谁)，由下面的判定决定用严格还是放宽规则。
        let target = with_store(state, |store| {
            if let Some(post) = store.monitored_posts.iter().find(|item| item.id == id) {
                let shared = post
                    .watch_id
                    .as_ref()
                    .and_then(|watch_id| store.competitor_watchlist.iter().find(|w| &w.id == watch_id))
                    .map(|watch| watch.shared_to.clone())
                    .unwrap_or_default();
                return Ok(Some((post.owner.clone(), shared)));
            }
            if let Some(watch) = store.competitor_watchlist.iter().find(|item| item.id == id) {
                return Ok(Some((watch.owner.clone(), watch.shared_to.clone())));
            }
            Ok(None)
        })?;
        if let Some((owner, shared)) = target {
            let allowed = if strict {
                crate::identity::owner_matches(&owner)
            } else {
                crate::identity::can_access(&owner, &shared)
            };
            if !allowed {
                return Err("对象不存在".to_string());
            }
        }
    }

    if let Some(watch_id) = payload
        .get("watchId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let target = with_store(state, |store| {
            Ok(store
                .competitor_watchlist
                .iter()
                .find(|item| item.id == watch_id)
                .map(|item| (item.owner.clone(), item.shared_to.clone())))
        })?;
        if let Some((owner, shared)) = target {
            let allowed = if strict {
                crate::identity::owner_matches(&owner)
            } else {
                crate::identity::can_access(&owner, &shared)
            };
            if !allowed {
                return Err("博主不存在".to_string());
            }
        }
    }

    Ok(())
}

pub fn handle_monitor_channel(
    app: &AppHandle,
    state: &State<'_, AppState>,
    channel: &str,
    payload: &Value,
) -> Option<Result<Value, String>> {
    if !channel.starts_with("monitor:") {
        return None;
    }
    // 全局/危险操作仅管理员（桌面端）可用；web 用户只能操作自己的数据。
    //
    // 注意：扫码登录（chrome-qr-start/status）与连接测试（chrome-test）**不**在此列表里。
    // 平台账号是服务器上所有用户共享的，抓取本身对所有 web 用户开放；
    // cookie 过期时必须能在 web 端续上登录态（管理员不一定在服务器前）。
    // 退出登录（chrome-shutdown）与打开服务器 Chrome（chrome-launch）仍限管理员：
    // 前者会断掉所有人共用的会话，后者会在服务器上弹窗口。
    const ADMIN_ONLY: &[&str] = &[
        "monitor:clear",
        // 注：monitor:clear-posts **不**在列表里 —— 它按身份区分作用域
        // （管理员清全库 / web 用户只清自己的），web 用户有清理自己数据的合理需求。
        "monitor:retro",
        "monitor:set-export-root",
        "monitor:pick-export-root",
        "monitor:set-obsidian-vault",
        "monitor:pick-obsidian-vault",
        "monitor:patch-audio-batch",
        "monitor:run-daily",
        "monitor:run-due",
        "monitor:set-scan-limit",
        "monitor:set-spike-threshold",
        "monitor:chrome-shutdown",
        "monitor:chrome-launch",
        // 分配博主给 web 用户：只有管理员能做（他也是唯一能看见全部用户表的人）。
        "monitor:watch-assign",
        "monitor:web-users",
        // 清孤儿数据会跨用户删记录，只允许管理员。
        "monitor:purge-orphans",
    ];
    if !crate::identity::is_admin() && ADMIN_ONLY.contains(&channel) {
        return Some(Err("该操作仅管理员可用".to_string()));
    }
    if let Err(error) = guard_owned_target(state, channel, payload) {
        return Some(Err(error));
    }
    Some(
        (|| match channel {
            "monitor:watch-add" => watch_add(state, payload),
            "monitor:watch-list" => watch_list(state, payload),
            "monitor:watch-remove" => watch_remove(state, payload),
            "monitor:watch-assign" => watch_assign(state, payload),
            "monitor:web-users" => list_web_users(state),
            "monitor:purge-orphans" => purge_orphans(state),
            "monitor:fetch" => fetch_monitor(state, payload, None, false),
            "monitor:post-list" => post_list(state, payload),
            "monitor:inject-topic" => inject_topic(state, payload),
            "monitor:run-due" => run_due_fetches(state, now_i64()),
            "monitor:chrome-status" => chrome_status(app, payload),
            "monitor:chrome-launch" => chrome_launch(app, payload),
            "monitor:chrome-test" => chrome_test(app, payload),
            "monitor:chrome-qr-start" => chrome_qr_start(app, payload),
            "monitor:chrome-qr-status" => chrome_qr_status(app, payload),
            "monitor:chrome-login-debug" => chrome_login_debug(app, payload),
            "monitor:chrome-export-cookies" => chrome_export_cookies(app, payload),
            "monitor:chrome-import-cookies" => chrome_import_cookies(app, payload),
            "monitor:chrome-shutdown" => chrome_shutdown(app, payload),
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
            "monitor:remix-generate" => remix_generate(state, payload),
            "monitor:remix-list" => remix_list(state, payload),
            "monitor:material-files" => material_files(state, payload),
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
    // 必须校验端口是否还活着：用户手动关掉 Chrome 后端口号仍留在会话表里，
    // 不校验就会一直显示"已就绪"，实际抓取全失败。
    let alive = port != 0 && headless_browser::chrome_session_alive(platform);
    let ready_platforms = headless_browser::ready_platforms();
    Ok(json!({
        "chromeInstalled": path.is_some(),
        "chromePath": path.as_ref().map(|p| p.display().to_string()),
        "platform": platform,
        "debugPort": if alive { port } else { 0 },
        "debugReady": alive,
        "readyPlatforms": ready_platforms,
    }))
}

/// 扫码登录：打开登录页并截图二维码，返回 data URL 给前端显示。
///
/// 支持小红书 / 抖音 / B 站（都用手机对应 App 扫官方二维码）。
///
/// 保护：服务器上的平台账号是**所有用户共享**的 —— 任何人一扫码就等于把
/// 服务器换成他自己的账号（把原来的会话顶掉）。所以已登录时默认拒绝，
/// 需要显式 `force` 才允许更换。
fn chrome_qr_start(app: &AppHandle, payload: &Value) -> Result<Value, String> {
    let platform = payload
        .get("platform")
        .and_then(Value::as_str)
        .unwrap_or("xiaohongshu");
    let spec = headless_browser::qr_platform_spec(platform)
        .ok_or_else(|| "暂时只支持小红书 / 抖音 / B 站扫码登录".to_string())?;
    let force = payload
        .get("force")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let mut port = headless_browser::active_chrome_port(platform);
    if port == 0 {
        chrome_launch(
            app,
            &json!({ "platform": platform, "url": spec.login_url }),
        )?;
        port = headless_browser::active_chrome_port(platform);
    }
    if port == 0 {
        return Err("Chrome 未就绪（启动失败）".to_string());
    }
    let ws_url = tauri::async_runtime::block_on(headless_browser::ws_url_for_port(port))?;

    // 已登录 → 拒绝扫码，避免把服务器上共享的账号顶掉。
    if !force {
        let already = tauri::async_runtime::block_on(headless_browser::login_logged_in(
            &ws_url, platform,
        ))
        .unwrap_or(false);
        if already {
            return Err(
                "服务器已登录该平台（账号对所有用户共享）。如需更换账号，请先在桌面端「登录管理」点「退出登录」后再扫码。"
                    .to_string(),
            );
        }
    }

    let capture = tauri::async_runtime::block_on(headless_browser::capture_login_qr(
        &ws_url, &spec,
    ))?;
    let image = capture
        .data_url
        .ok_or_else(|| "未能获取二维码图像".to_string())?;
    Ok(json!({
        "success": true,
        "image": image,
        "port": port,
        // 诊断信息：便于定位"扫出来是过期码"这类问题
        "kind": capture.kind,
        "refreshed": capture.refreshed,
        "expired": capture.expired,
        "snippet": capture.snippet,
        // 小红书风控态：需要扫弹窗里的"请通过验证"二维码
        "needVerify": capture.need_verify,
    }))
}

/// 查询扫码登录是否完成（看平台会话 cookie）。
fn chrome_qr_status(_app: &AppHandle, payload: &Value) -> Result<Value, String> {
    let platform = payload
        .get("platform")
        .and_then(Value::as_str)
        .unwrap_or("xiaohongshu");
    // refresh=true：跳过缓存强制重检（用户/管理员刚在服务器上手动登录完，想立即确认）
    let refresh = payload
        .get("refresh")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if refresh {
        headless_browser::invalidate_login_state(platform);
    }
    let port = headless_browser::active_chrome_port(platform);
    if port == 0 {
        return Ok(json!({ "success": true, "loggedIn": false, "note": "Chrome 未启动" }));
    }
    let ws_url = tauri::async_runtime::block_on(headless_browser::ws_url_for_port(port))?;
    let logged_in =
        tauri::async_runtime::block_on(headless_browser::login_logged_in(&ws_url, platform))?;
    Ok(json!({ "success": true, "loggedIn": logged_in }))
}

/// 登录态诊断：把服务器 Chrome 里该平台页面的真实状态取回来（只回 cookie 名，不回值）。
fn chrome_login_debug(_app: &AppHandle, payload: &Value) -> Result<Value, String> {
    let platform = payload
        .get("platform")
        .and_then(Value::as_str)
        .unwrap_or("xiaohongshu");
    let port = headless_browser::active_chrome_port(platform);
    if port == 0 {
        return Err("Chrome 未启动".to_string());
    }
    let ws_url = tauri::async_runtime::block_on(headless_browser::ws_url_for_port(port))?;
    tauri::async_runtime::block_on(headless_browser::login_debug(&ws_url, platform))
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
        "note": format!("已为你打开登录页，请在弹出的 Chrome 中完成登录。每个平台有独立 profile（cookie 隔离）。{} 不会保存你的密码，只复用这次登录的 cookie。", app_brand_display_name()),
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

/// 导出该平台的登录 cookie（跨机器迁移登录态用，绕开平台风控验证）。
fn chrome_export_cookies(_app: &AppHandle, payload: &Value) -> Result<Value, String> {
    let platform = payload
        .get("platform")
        .and_then(Value::as_str)
        .unwrap_or("xiaohongshu");
    let port = headless_browser::active_chrome_port(platform);
    if port == 0 {
        return Err("Chrome 未启动：请先「打开 Chrome 登录」并完成登录".to_string());
    }
    let ws_url = tauri::async_runtime::block_on(headless_browser::ws_url_for_port(port))?;
    tauri::async_runtime::block_on(headless_browser::export_login_cookies(&ws_url, platform))
}

/// 导入登录 cookie（把另一台机器导出的 JSON 写进服务器 Chrome）。
fn chrome_import_cookies(_app: &AppHandle, payload: &Value) -> Result<Value, String> {
    let platform = payload
        .get("platform")
        .and_then(Value::as_str)
        .unwrap_or("xiaohongshu");
    let cookies = payload
        .get("cookies")
        .and_then(Value::as_array)
        .ok_or_else(|| "缺少 cookies 数组".to_string())?;
    let port = headless_browser::active_chrome_port(platform);
    if port == 0 {
        return Err("Chrome 未启动：请先「打开 Chrome 登录」把 Chrome 拉起来再导入".to_string());
    }
    let ws_url = tauri::async_runtime::block_on(headless_browser::ws_url_for_port(port))?;
    let count =
        tauri::async_runtime::block_on(headless_browser::import_login_cookies(&ws_url, cookies))?;
    Ok(json!({ "success": true, "imported": count }))
}

fn chrome_shutdown(app: &AppHandle, payload: &Value) -> Result<Value, String> {
    let platform = payload
        .get("platform")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    headless_browser::shutdown_chrome();
    // 真·退出登录：删掉该平台的 Chrome profile（cookie 就存在里面）。
    // 只关进程不删 profile 的话，下次启动仍是登录状态 —— 按钮名不副实。
    let mut cleared = false;
    if let Some(platform) = platform {
        headless_browser::invalidate_login_state(platform);
        if let Ok(dir) = headless_browser::profile_dir_for(app, platform) {
            // 给进程一点时间释放文件句柄（Windows 上尤其需要）
            std::thread::sleep(std::time::Duration::from_millis(700));
            let mut removed = false;
            for _ in 0..3 {
                if std::fs::remove_dir_all(&dir).is_ok() {
                    removed = true;
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            cleared = removed;
        }
    }
    Ok(json!({
        "success": true,
        "cleared": cleared,
        "note": if cleared {
            "已退出登录：该平台的 Chrome 登录态（profile）已清除，下次抓取需重新登录"
        } else {
            "已关闭 Chrome 进程；未指定平台或 profile 删除失败，登录态可能仍保留"
        }
    }))
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
    let owner = crate::identity::owner_or_admin();
    let platform = str_field(payload, "platform").unwrap_or_default();
    let profile_url = str_field(payload, "profileUrl")
        .or_else(|| str_field(payload, "profile_url"))
        .map(|url| crate::tools::headless_browser::normalize_profile_url(&url));
    // 判重键：同一用户 + 同一平台 + 同一主页（归一化后的裸 URL）。
    //
    // 为什么必须判重：同一博主建两个监控项时，两个 watch 会轮流抓同一批帖子，
    // 而落库按 (owner, 裸URL) 去重 → 帖子只会归属其中**一个** watch，
    // 另一个 watch 的卡片永远显示「已收录 0 篇」，用户会以为抓取坏了。
    // 重复添加同一个博主属于误操作，直接提示而不是默默建第二条。
    let dedupe_key = watch_dedupe_key(&owner, &platform, profile_url.as_deref());
    if let Some(key) = dedupe_key.as_ref() {
        let duplicate = with_store(state, |store| {
            Ok(store
                .competitor_watchlist
                .iter()
                .find(|item| {
                    watch_dedupe_key(&item.owner, &item.platform, item.profile_url.as_deref())
                        .as_ref()
                        == Some(key)
                })
                .cloned())
        })?;
        if let Some(existing) = duplicate {
            // 返回 Ok({success:false}) 而不是 Err：IPC 通道会把 Err 包成
            // 「host request failed for "monitor:watch-add"」的通用文案，
            // 把这里的具体原因盖掉，用户就看不到「已存在」这个关键信息了。
            return Ok(json!({
                "success": false,
                "duplicate": true,
                "error": format!(
                    "「{}」已在监控列表中（ID {}），请勿重复添加同一博主",
                    existing.name, existing.id
                ),
            }));
        }
    }
    let record = crate::CompetitorWatchRecord {
        id: make_id("watch"),
        owner,
        // 新建的博主默认不分配：只有创建者（以及 admin）可见，之后由管理员手动分配。
        shared_to: Vec::new(),
        name,
        platform,
        profile_url,
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

/// 监控项判重键：`owner + 平台 + 归一化主页 URL`。
///
/// 主页 URL 要去掉 query：小红书的 `xsec_token` / `xsec_source` 每次会话都变，
/// 带 token 比较会把同一个博主判成两个。
/// 主页 URL 缺失时返回 `None` —— 没有 URL 就无从判重。
fn watch_dedupe_key(owner: &str, platform: &str, profile_url: Option<&str>) -> Option<String> {
    let url = profile_url?.trim();
    if url.is_empty() {
        return None;
    }
    let bare = url.split('?').next().unwrap_or(url).trim_end_matches('/');
    if bare.is_empty() {
        return None;
    }
    Some(format!(
        "{}|{}|{}",
        owner.trim(),
        platform.trim().to_ascii_lowercase(),
        bare.to_ascii_lowercase()
    ))
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
        // 按请求身份过滤：桌面端（admin）看全部；web 用户看「自己的」+「被分配给他的」。
        items.retain(|item| crate::identity::can_access(&item.owner, &item.shared_to));
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

/// 把博主分配（或取消分配）给指定 web 用户（`monitor:watch-assign`）。
///
/// 只改 `shared_to`，不动 `owner`：抓取产物仍记在 owner 名下，
/// 分配 = 让被分配者在 web 端「看得见、能抓、能下载」；删除权仍只归 owner。
/// 仅管理员可调用（`ADMIN_ONLY` 拦截）。
fn watch_assign(state: &State<'_, AppState>, payload: &Value) -> Result<Value, String> {
    let id = str_field(payload, "id").ok_or_else(|| "缺少 id".to_string())?;
    let users: Vec<String> = payload
        .get("users")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .collect()
        })
        .unwrap_or_default();
    with_store_mut(state, |store| {
        let Some(watch) = store.competitor_watchlist.iter_mut().find(|item| item.id == id) else {
            return Err("博主不存在".to_string());
        };
        let mut next: Vec<String> = Vec::new();
        for user in users {
            // admin 本来就全可见；owner 本人也本来就可见 —— 写进名单没有意义。
            if user == "admin" || crate::identity::owner_equals(&watch.owner, &user) {
                continue;
            }
            if !next.contains(&user) {
                next.push(user);
            }
        }
        watch.shared_to = next;
        watch.updated_at = now_i64().to_string();
        Ok(json!({ "success": true, "sharedTo": watch.shared_to }))
    })
}

/// 列出可分配的 web 用户（`monitor:web-users`）。
///
/// 账号表不归 daemon 管（由 web 网关维护 `users.json`），所以这里按几个常见位置找它：
/// 环境变量 → 当前工作目录 → store 所在目录 → 可执行文件目录。
/// 找不到就返回空列表，前端退化成「手输用户名」，不当作错误。
fn list_web_users(state: &State<'_, AppState>) -> Result<Value, String> {
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(custom) = std::env::var("BEAV_USERS_FILE") {
        let custom = custom.trim();
        if !custom.is_empty() {
            candidates.push(std::path::PathBuf::from(custom));
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        for rel in ["users.json", "../users.json", "desktop/users.json", "../../users.json"] {
            candidates.push(cwd.join(rel));
        }
    }
    if let Some(dir) = state.store_path.parent() {
        candidates.push(dir.join("users.json"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("users.json"));
        }
    }
    let Some(path) = candidates.into_iter().find(|candidate| candidate.exists()) else {
        return Ok(json!({ "users": [], "found": false }));
    };
    let raw = std::fs::read_to_string(&path)
        .map_err(|error| format!("读取 {} 失败: {error}", path.display()))?;
    let value: Value = serde_json::from_str(&raw)
        .map_err(|error| format!("解析 {} 失败: {error}", path.display()))?;
    let users: Vec<Value> = value
        .get("users")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let username = item.get("username").and_then(Value::as_str)?.trim();
                    // admin 不需要「被分配」：桌面端本来就全可见。
                    if username.is_empty() || username == "admin" {
                        return None;
                    }
                    Some(json!({
                        "username": username,
                        "role": item.get("role").and_then(Value::as_str).unwrap_or("user"),
                    }))
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(json!({ "users": users, "found": true, "path": path.display().to_string() }))
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
///
/// 归属校验：web 用户只能删自己的帖子（`guard_owned_target` 已按 id 校验过归属，
/// 这里再兜一层，防止越权删到别人的数据）。
fn post_remove(state: &State<'_, AppState>, payload: &Value) -> Result<Value, String> {
    let id = str_field(payload, "id").ok_or_else(|| "缺少 id".to_string())?;
    with_store_mut(state, |store| {
        let before = store.monitored_posts.len();
        let mut removed = false;
        store.monitored_posts.retain(|item| {
            if item.id != id {
                return true;
            }
            // retain 的返回值语义是「**保留**」，所以要取反：
            // 只有「不属于当前身份」的记录才保留，不被 web 用户删掉别人的帖子。
            // （原先少写了 `!`：管理员对任何 owner 都匹配 → 恒保留 → 谁点了删除都
            //   只会得到「动态不存在」，帖子其实一条都没删掉。）
            if crate::identity::owner_matches(&item.owner) {
                removed = true;
                return false;
            }
            true
        });
        if !removed || store.monitored_posts.len() == before {
            return Err("动态不存在".to_string());
        }
        // 级联清掉这条帖子的互动快照：快照只按 post_id 关联，帖子没了就永远是孤儿，
        // 但还会被「数据复盘」按作者聚合进走势里。
        let before_snapshots = store.post_snapshots.len();
        store.post_snapshots.retain(|snapshot| snapshot.post_id != id);
        let removed_snapshots = before_snapshots - store.post_snapshots.len();
        Ok(json!({ "success": true, "removedSnapshots": removed_snapshots }))
    })
}

/// 清理孤儿数据（`monitor:purge-orphans`，管理员专用）。
///
/// 两种孤儿：
/// 1. 帖子：`watchId` 指向一个已不存在的博主（删博主不级联删帖子）——
///    界面里永远显示不出来，但还占着库；
/// 2. 快照：`postId` 指向一条已不存在的帖子（帖子被删时没级联，或按老版本删的）。
///
/// 快照这类残留必须按「现存帖子集合」反查，不能只跟着本次删掉的帖子走 ——
/// 帖子早在上一版删掉了，那时还没级联，留下的快照就永远清不到。
///
/// `watchId` 为空的帖子不算孤儿（临时抓取进来的帖子本来就不挂在博主下）。
fn purge_orphans(state: &State<'_, AppState>) -> Result<Value, String> {
    with_store_mut(state, |store| {
        let known: std::collections::HashSet<String> =
            store.competitor_watchlist.iter().map(|watch| watch.id.clone()).collect();
        let before_posts = store.monitored_posts.len();
        store.monitored_posts.retain(|post| {
            !post
                .watch_id
                .as_ref()
                .is_some_and(|watch_id| !known.contains(watch_id))
        });
        let removed_posts = before_posts - store.monitored_posts.len();

        let alive: std::collections::HashSet<String> =
            store.monitored_posts.iter().map(|post| post.id.clone()).collect();
        let before_snapshots = store.post_snapshots.len();
        store
            .post_snapshots
            .retain(|snapshot| alive.contains(&snapshot.post_id));
        let removed_snapshots = before_snapshots - store.post_snapshots.len();
        Ok(json!({
            "success": true,
            "removedPosts": removed_posts,
            "removedSnapshots": removed_snapshots,
        }))
    })
}

/// 清空全部动态（仅 `monitored_posts`，不动对标博主列表）。
///
/// 权限分两种：
/// - 管理员（桌面端）→ 清空**全库**（服务器上所有用户的数据都在同一个 store 里）。
/// - web 用户 → 只清**自己**的帖子。整库清空是危险操作，web 用户没有权限；
///   但「清空我的动态」是合理需求，不该被整个挡掉（原先一律 ADMIN_ONLY，
///   结果 web 端点「清空动态」只会弹「仅管理员可用」，用户完全没法清理自己的数据）。
///
/// 带上 store 路径与「本进程看到的条数」——服务器上是 web 网关 + 桌面版两个进程，
/// 排查「清空了 0 条」时需要一眼看出是数据在别的 store 里，还是本进程本来就没有。
fn clear_posts(state: &State<'_, AppState>) -> Result<Value, String> {
    let is_admin = crate::identity::is_admin();
    with_store_mut(state, |store| {
        let before = store.monitored_posts.len();
        if is_admin {
            store.monitored_posts.clear();
        } else {
            // 只清当前用户自己的（空 owner 的旧数据归 admin，普通用户清不到）。
            store
                .monitored_posts
                .retain(|item| !crate::identity::owner_matches(&item.owner));
        }
        let removed = before - store.monitored_posts.len();
        Ok(json!({
            "success": true,
            "removedPosts": removed,
            "scope": if is_admin { "all" } else { "own" },
            "storePath": state.store_path.display().to_string(),
        }))
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
fn resolve_watch_id(
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
const SCAN_LIMIT: usize = 30;

/// 解析「单次抓取条数」：恒定返回 `SCAN_LIMIT`，忽略请求 payload 与历史持久化配置。
///
/// 保留函数签名是为了让所有调用点（fetch_monitor / adhoc 抓取 / 每日预取）无需改动，
/// 也避免旧前端传 `limit` 时报错。
/// 单次抓取条数。默认 `SCAN_LIMIT`（日常增量只要最近 30 条）；
/// 补历史深翻会在 payload 里显式传 `limit = BACKFILL_MAX(500)`，必须尊重它——
/// 否则滚动循环里的 `captured >= limit` 会在第一屏就把 500 的目标截成 30。
fn resolve_scan_limit(_state: &State<'_, AppState>, payload: &Value) -> usize {
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

/// 全局抓取互斥：同一时刻只允许一轮抓取（手动补历史 / 定时增量 / 多 watch 深翻）。
/// 否则会同时开多个 CDP 标签抢同一个 Chrome，互相拖慢甚至触发平台限流。
static MONITOR_FETCH_INFLIGHT: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

struct MonitorFetchGuard;

impl MonitorFetchGuard {
    fn acquire() -> Option<Self> {
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

fn fetch_monitor(
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
        // 同样按「归属用户 + URL」限定：否则两个用户监控同一博主时，
        // 后抓的会把先抓的记录抢走（watch_id 被覆盖），先抓的用户卡片上的条数归零。
        let scope_owner = watch
            .as_ref()
            .map(|item| item.owner.clone())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(crate::identity::owner_or_admin);
        let (existing, original_watch_exists): (Option<crate::MonitoredPostRecord>, bool) =
            with_store_mut(state, |store| {
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
            with_store_mut(state, |store| {
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

/// 列表投影：只返回卡片渲染 + 详情预取需要的字段，**剔除 `content` / `images`**。
///
/// 真实正文动辄 1~5 KB/条，一屏 500 条时就是 1~2 MB 的纯浪费；而列表卡片只显示
/// 标题 / 摘要 / 互动数，正文与图片列表要到详情弹窗才用得上 —— 那里由 `post_detail`
/// 单独取（命中 store 缓存时是纯内存读，不联网）。
///
/// 仍保留 `hasContent` 布尔：卡片据此决定「拆解」按钮是否可点，
/// 否则前端为了一个真假值又得把整篇正文拿下来。
fn project_post_for_list(
    item: &crate::MonitoredPostRecord,
    transcript_status: &str,
    // 选题中心里仍存在的来源 URL 集合（「已入选题」是派生状态，不能只看静态标记）
    injected_urls: &std::collections::HashSet<String>,
) -> Value {
    json!({
        "id": item.id,
        "owner": item.owner,
        "watchId": item.watch_id,
        "platform": item.platform,
        "author": item.author,
        "authorUrl": item.author_url,
        "postUrl": item.post_url,
        "title": item.title,
        "mediaType": item.media_type,
        "coverUrl": item.cover_url,
        "detailFetched": item.detail_fetched,
        // 正文被投影掉了，但卡片要据此禁用「拆解」按钮，所以单独给一个布尔。
        "hasContent": !item.content.trim().is_empty(),
        "excerpt": item.excerpt,
        "publishedAt": item.published_at,
        "likes": item.likes,
        "comments": item.comments,
        "collects": item.collects,
        "prevLikes": item.prev_likes,
        "prevComments": item.prev_comments,
        "isNew": item.is_new,
        "spike": item.spike,
        "spikeRatio": item.spike_ratio,
        "capturedVia": item.captured_via,
        // 「已入选题」是派生状态：选题中心里被删掉后，这里必须跟着消失。
        // 只读静态标记会留下假徽标（实测：加入后去选题中心放弃，监控列表仍显示已加入）。
        "topicInjected": item.topic_injected && injected_urls.contains(&item.post_url),
        "deconstructed": item.deconstructed,
        "videoStatus": item.video_status,
        "videoLocalPath": item.video_local_path,
        "videoPreviewPath": item.video_preview_path,
        "videoSize": item.video_size,
        "videoDownloadedAt": item.video_downloaded_at,
        "videoError": item.video_error,
        "videoHasAudio": item.video_has_audio,
        // 语音转写状态，来自 post_video_analyses（列表筛选要用，所以一并投影）：
        // "" = 没跑过分析；pending = 已抽帧待转写；ok / no-audio / not-configured / failed。
        "transcriptStatus": transcript_status,
        "createdAt": item.created_at,
        "updatedAt": item.updated_at,
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
    // 纯读：用读锁，避免每次切换博主都触发一次整库落盘（2MB JSON）。
    with_store(state, |store| {
        // 可见博主：自己的 + 被分配给我的。帖子只冗余了 owner，
        // 「被分配」记在 watch 上，所以这里按 watchId 做一次归属传播。
        let visible_watch_ids: std::collections::HashSet<&str> = store
            .competitor_watchlist
            .iter()
            .filter(|watch| crate::identity::can_access(&watch.owner, &watch.shared_to))
            .map(|watch| watch.id.as_str())
            .collect();
        // 只持有**引用**并就地过滤/排序/截断，最后才投影成 JSON。
        // 早期实现是 `iter().cloned().collect()` 先把全库克隆一遍再 retain ——
        // 服务器上帖子总量上万时，那份 clone 与「只看这一个博主的 500 条」毫无关系，纯浪费。
        let mut matched: Vec<&crate::MonitoredPostRecord> = store
            .monitored_posts
            .iter()
            .filter(|item| {
                crate::identity::owner_matches(&item.owner)
                    || item
                        .watch_id
                        .as_deref()
                        .is_some_and(|watch_id| visible_watch_ids.contains(watch_id))
            })
            .filter(|item| match &watch_id {
                Some(id) => item.watch_id.as_deref() == Some(id.as_str()),
                None => true,
            })
            .filter(|item| !spike_only || item.spike)
            .collect();
        matched.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| right.created_at.cmp(&left.created_at))
        });
        if let Some(max) = limit {
            matched.truncate(max.max(1));
        }
        // 转写状态单独存在 post_video_analyses 里，这里建一次 postId → status 映射，
        // 让列表能按「语音转写状态」筛选（只读引用，不额外 clone）。
        let transcript_by_post: std::collections::HashMap<&str, &str> = store
            .post_video_analyses
            .iter()
            .map(|analysis| {
                (
                    analysis.post_id.as_str(),
                    analysis.transcript_status.as_str(),
                )
            })
            .collect();
        let injected_urls: std::collections::HashSet<String> = store
            .topic_center
            .iter()
            .flat_map(|topic| topic.source_refs.iter())
            .filter_map(|reference| {
                reference
                    .get("url")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .collect();
        // 列表走投影：正文/图片不随列表下发（详情弹窗单独经 post_detail 取）。
        // 截断之后再投影，所以 limit 生效时连投影开销都只付前 N 条。
        Ok(Value::Array(
            matched
                .iter()
                .map(|item| {
                    let status = transcript_by_post
                        .get(item.id.as_str())
                        .copied()
                        .unwrap_or("");
                    project_post_for_list(item, status, &injected_urls)
                })
                .collect(),
        ))
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

    // 纯读：用读锁，避免复盘统计触发整库落盘。
    with_store(state, |store| {
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
/// 拆解输入的字符预算（按 Unicode 字符计，不是字节）。
const DECONSTRUCT_BODY_CLIP: usize = 1500;
const DECONSTRUCT_TRANSCRIPT_CLIP: usize = 2000;
/// 口播大纲的抽段步长与上限：一小时视频的 SRT 有几万字，必须压成能塞进上下文的骨架。
const DECONSTRUCT_OUTLINE_STEP_SECS: i64 = 90;
const DECONSTRUCT_OUTLINE_MAX: usize = 20;

fn clip_chars(text: &str, max: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max {
        return trimmed.to_string();
    }
    let mut out: String = trimmed.chars().take(max).collect();
    out.push('…');
    out
}

fn format_hms(total: i64) -> String {
    let secs = total.max(0);
    format!("{:02}:{:02}:{:02}", secs / 3600, (secs % 3600) / 60, secs % 60)
}

/// 解析 SRT 成 `(起始秒, 文本)`。脏行直接跳过——拆解不该因为字幕格式问题整条失败。
fn srt_cues(srt: &str) -> Vec<(i64, String)> {
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
fn srt_outline(cues: &[(i64, String)], step_secs: i64, max_lines: usize) -> String {
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
const DECONSTRUCT_GUIDE: &str = include_str!("remix_prompts/deconstruction.md");
const STORYBOARD_GUIDE: &str = include_str!("remix_prompts/storyboard.md");
const REWRITE_GUIDE: &str = include_str!("remix_prompts/rewrite-playbook.md");

const DECONSTRUCT_SYSTEM_PROMPT: &str = "你是资深内容拆解专家，负责把已验证的爆款内容拆成可复用的方法论。\
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
    let settings = with_store(state, |store| Ok(store.settings.clone()))?;
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

// —— 二创产出：分镜脚本 / 多平台改写 ——

/// 目标平台的写作规范（改写用）。集中一处，避免 prompt 里散落平台细节。
fn remix_platform_guide(target: &str) -> (&'static str, &'static str) {
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
fn render_storyboard_markdown(raw: &str, target_seconds: i64) -> String {
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
fn remix_generate(state: &State<'_, AppState>, payload: &Value) -> Result<Value, String> {
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

    let settings = with_store(state, |store| Ok(store.settings.clone()))?;
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

    export_post_product(state, &post.id);

    Ok(json!({ "success": true, "remix": saved }))
}

/// 导出素材包：确保产物已落盘，返回文件清单（打包交给前端调 file:save-zip）。
///
/// 只负责打包，不管对方怎么用：清单里的 name 保留「平台/日期/作者/标题/文件」
/// 相对路径，解压后就是一份完整的素材目录。
fn material_files(state: &State<'_, AppState>, payload: &Value) -> Result<Value, String> {
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
    for id in ids {
        export_post_product(state, &id); // 幂等：已有产物只是补齐
        let post = with_store(state, |store| {
            Ok(store.monitored_posts.iter().find(|p| p.id == id).cloned())
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
        let dir = root.join("_素材包");
        std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
        let target = dir.join(format!("{name}.zip"));
        let written = crate::commands::file_ops::archive::write_zip_archive(state, &files, target)?;
        return Ok(json!({ "success": true, "count": files.len(), "bundle": written }));
    }
    Ok(json!({ "success": true, "count": files.len(), "files": files }))
}

/// 递归收集产物文件（产物目录只有「二创/」一层子目录，深度给 3 足够）。
fn collect_bundle_files(dir: &Path, root: &Path, out: &mut Vec<Value>) {
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
fn remix_list(state: &State<'_, AppState>, payload: &Value) -> Result<Value, String> {
    let post_id = str_field(payload, "postId")
        .or_else(|| str_field(payload, "post_id"))
        .unwrap_or_default();
    with_store(state, |store| {
        let mut items: Vec<Value> = store
            .post_remixes
            .iter()
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
    let mut cmd = Command::new(&ff);
    cmd.arg("-y")
        .arg("-headers")
        .arg(&headers)
        .arg("-i")
        .arg(audio_url)
        .arg("-rw_timeout")
        .arg("20000000")
        .arg("-vn")
        .arg(&tmp_audio);
    let out = run_command_with_timeout(&mut cmd, std::time::Duration::from_secs(180))
        .map_err(|e| format!("下载音频轨失败：{e}"))?;
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
/// 带看门狗的进程执行：网络类 ffmpeg（拉 B站/抖音流）可能在「连接已建立但不再给数据」
/// 或 URL 过期时挂死，`-rw_timeout` 并不总能生效。这里按进程级超时兜底：到点强杀并返回
/// `TimedOut`，让上层按失败处理、队列继续推进，而不是整批卡死。
fn run_command_with_timeout(
    command: &mut Command,
    timeout: std::time::Duration,
) -> std::io::Result<std::process::Output> {
    use std::io::Read;
    let mut child = command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();
    let stdout_handle = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        if let Some(pipe) = stdout_pipe.as_mut() {
            let _ = pipe.read_to_end(&mut buffer);
        }
        buffer
    });
    let stderr_handle = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        if let Some(pipe) = stderr_pipe.as_mut() {
            let _ = pipe.read_to_end(&mut buffer);
        }
        buffer
    });
    let started = std::time::Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!("进程超过 {}s 无响应，已强制结束", timeout.as_secs()),
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    };
    let stdout = stdout_handle.join().unwrap_or_default();
    let stderr = stderr_handle.join().unwrap_or_default();
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

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
        let mut cmd = Command::new(&ff);
        cmd.arg("-y")
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
            .arg(&dest_str);
        let output = run_command_with_timeout(&mut cmd, std::time::Duration::from_secs(600))
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
/// 抽帧结果能否直接复用（= 视频没变过）。
///
/// 判据：目录里已有帧，且最新一帧的时间不早于「视频下载完成时间」。
/// `downloaded_at_secs == 0` 表示老数据没有该字段，此时只要目录里有帧就复用
/// （没有更可靠的判据，重抽一次的代价远大于用旧帧的风险）。
fn frames_are_reusable(
    has_frames: bool,
    latest_frame_secs: Option<i64>,
    downloaded_at_secs: i64,
) -> bool {
    if !has_frames {
        return false;
    }
    match latest_frame_secs {
        Some(secs) => downloaded_at_secs == 0 || secs >= downloaded_at_secs,
        None => false,
    }
}

/// 新抓到的正文能否覆盖旧正文。
///
/// - 旧内容是**脏数据**（含「举报发布时间」这类界面噪音）→ 无条件覆盖，
///   哪怕新内容更短。否则垃圾因为"更长"会一直锁在库里（71 字噪音 vs 48 字干净文案）。
/// - 否则要求新内容不短于旧内容：平台改版后提取器可能只拿得到标题级文本，
///   无条件覆盖会把已存好的完整正文冲成标题，而拆解/摘要依赖全文。
fn should_replace_content(old: &str, new: &str) -> bool {
    let old_is_noisy = old.contains("举报发布时间") || old.contains("举报\n发布时间");
    old_is_noisy || new.chars().count() >= old.chars().count()
}

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

    let read_frames = || -> Vec<String> {
        std::fs::read_dir(&frames_dir)
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
            .unwrap_or_default()
    };

    let existing_frames = read_frames();

    // 帧还能不能直接复用：目录里有帧，且最新一帧不早于「视频下载完成时间」。
    //
    // 为什么要这个判断：抽帧是 `fps=1/5` 解全片，46 分钟的视频要抽 550 帧，
    // 每次都重跑纯属白烧 CPU —— 尤其是「转写失败后点重新分析」这种场景，
    // 本来只想重跑 ASR，却连抽帧一起重来。重新下载会刷新 video_downloaded_at，
    // 旧帧自动失效、照常重抽，所以不需要额外的 force 开关。
    let latest_frame_secs = existing_frames
        .iter()
        .filter_map(|p| std::fs::metadata(p).ok()?.modified().ok())
        .filter_map(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .max();
    let downloaded_at_secs = record.video_downloaded_at.map(|ms| ms / 1000).unwrap_or(0);
    let video_exists = Path::new(&local_path).exists();
    let frames_reusable = frames_are_reusable(
        !existing_frames.is_empty(),
        latest_frame_secs,
        downloaded_at_secs,
    );

    let frames: Vec<String> = if frames_reusable {
        existing_frames
    } else if !video_exists {
        // 分析完会 `delete_original_after_analysis` 删掉原片、只留代理，
        // 而 video_local_path 仍指向已删的原片 —— 此时抽帧必然失败（且错误被
        // `let _ =` 吞掉）。没有源文件就老老实实用现有帧，别做无用功。
        existing_frames
    } else {
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
        read_frames()
    };

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
///   知识库根 / 平台(抖音) / 日期(YYYYMMDD) / 作者 / 标题前40字[+帖子ID] / 具体内容文件
/// 触发：视频帖 `analyze_video` 完成后、转写补齐(`transcribe_video_only`)后、以及
///       图文/视频帖 `deconstruct` 完成后自动导出；也可经 `monitor:export-post` 手动触发
///       （便于导出未拆解的图文帖）。
/// 内容为：原图(`images`)、抽帧图(`frames`)、正文、转写全文、转写时间轴(SRT)、
///         视频摘要、爆文拆解(六要素 JSON+MD)、帖子信息(JSON 溯源)。
/// 设计为幂等：重复导出只覆盖/补齐，不会重复造目录；标题前 40 字撞名时追加帖子 ID
/// （通过 `.beav-post-id` 标记文件识别「自己的」目录，跨日期也能找回，避免重导出时产生重复目录）。

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

/// 详情页抽到的作者是否允许覆盖记录里已有的作者。
///
/// 背景：列表抓取阶段 `author` 写的是 `watch.name`（该监控项的博主本人），这是权威值。
/// 详情页的 `author` 来自页面兜底链（SSR → `_ROUTER_DATA.item_list[0]` → 作用域 DOM），
/// 会误抓正文里的 `@提及`（如「@抖音科技」「@抖音精选 App官方账号」）或推荐位的其他作者。
/// 一旦让它覆盖，同一个博主会因为不同帖子的 SSR 命中情况不同而裂成多个导出目录
/// （导出路径是 `导出根/平台/日期/作者/标题`）。
///
/// 因此：**有监控归属且当前作者非空时，拒绝详情页覆盖**；
/// 仅「无归属的临时帖（adhoc）」或「当前作者为空」才采用详情页的值。
fn detail_author_may_override(watch_id: Option<&str>, current_author: &str) -> bool {
    let has_watch = watch_id.map(str::trim).is_some_and(|value| !value.is_empty());
    let has_author = !current_author.trim().is_empty();
    !(has_watch && has_author)
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

/// 导出四级目录取标题前多少个字符（按 Unicode 字符计，空标题回退「未命名」）。
///
/// 历史：最早写死 10 —— 中文场景下明显不够，`【抖音独家】` 这类栏目前缀就占 6 个字符，
/// 剩 4 个只装得下 `Open`，于是 `【抖音独家】OpenAI Images 2.5 重磅发布` 与
/// `【抖音独家】OpenAI 为何紧急暂停 Astra 测试？` 会撞成同一个目录名，
/// 后者只能靠 `_{postId}` 后缀区分，目录树可读性很差。
///
/// 之后调到 20 缓解了碰撞，但实测仍然会切在词中间
/// （`【抖音独家】别再被骗了！AI根本没有在思`），Finder / Spotlight 里都难辨认。
/// 现在取 40：存量老数据本来就是 40 字上下的完整句式，新数据保持同一风格；
/// 40 个汉字 = 120 字节，离文件系统单段 255 字节上限仍很远。
/// 真撞了还有 `_{postId}` 后缀兜底（见 `try_export_post_product`）。
///
/// 注意：**会**影响已导出的目录 —— 复用旧目录时按当前前缀就地改名
/// （`try_export_post_product` 的 rename 分支），调这个值等于一次惰性重命名：
/// 只在该帖下次被自动导出时发生，不扫全库、不复制内容；目标名被占用则跳过改名。
const EXPORT_TITLE_PREFIX_LEN: usize = 40;

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
                // 年份只做格式校验（月份/日期才有取值范围）
                if let (Ok(_y), Ok(m), Ok(d)) = (
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

/// 归档目录的日期：取**发布时间**，不是导出时间。
///
/// 原来恒返回「今天」：同一篇 11/4 的帖子，11/5 导一次落 20261105、11/6 再导落 20261106，
/// 博主没更新的日子每天都会漂出一个新日期目录（实测 11/6 凌晨重抓 11/4 的更新）。
/// 内容只该有一个归档日期；抓不到发布时间才退回当天。
fn export_date_yyyymmdd(post: &MonitoredPostRecord) -> String {
    post.published_at
        .as_deref()
        .and_then(first_date_in)
        .unwrap_or_else(today_yyyymmdd)
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

/// 跨日期 / 跨作者名，在「平台」一级目录下找这篇帖子已经导出过的目录。
///
/// 为什么需要它：导出目录里的日期是**导出当天**，同一篇帖子在不同日子被再次自动导出
/// （转写重试成功、重新下载、重新抽帧、重新分析都会触发 `export_post_product`）时会落到
/// 另一个 `日期/作者` 分支，`find_own_export_dir(l3)` 按定义找不到，于是又建一份 ——
/// 素材库里就出现同一篇帖子的两个目录（内容各写一半，谁也删不掉谁）。
/// 作者名被清洗过（老数据 `@抖音科技` → 正确昵称）也会换作者目录，所以这里不按作者名
/// 定位，而是把每个日期目录下的作者目录都翻一遍，只认 `.beav-post-id`。
fn find_export_dir_for_post(l1: &Path, post_id: &str) -> Option<PathBuf> {
    let dates = std::fs::read_dir(l1).ok()?;
    for date_entry in dates.flatten() {
        let date_dir = date_entry.path();
        if !date_dir.is_dir() {
            continue;
        }
        let Ok(authors) = std::fs::read_dir(&date_dir) else {
            continue;
        };
        for author_entry in authors.flatten() {
            let author_dir = author_entry.path();
            if !author_dir.is_dir() {
                continue;
            }
            if let Some(found) = find_own_export_dir(&author_dir, post_id) {
                return Some(found);
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
    // 受管模式（Web 多实例）：导出根由实例启动参数固定，忽略 store 里的用户设置。
    if let Ok(raw) = std::env::var("BEAV_EXPORT_ROOT") {
        let raw = raw.trim();
        if !raw.is_empty() {
            return Ok(PathBuf::from(raw));
        }
    }
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
/// 用详情页抓到的干净文案纠正列表页留下的脏标题。
///
/// 背景：抖音**列表页**的 title 是「播放量 + 文案 + 文案重复一遍」拼出来的
/// （`1.8万日本资产泡沫… 日本资产泡沫…`），而**详情页**抓到的 desc 是干净的
/// （`日本资产泡沫…`）。`post_detail` 原本从不更新 title，于是脏标题永久留存，
/// 只能靠事后清洗脚本回洗 —— 而新抓的帖子还会持续产生脏标题。
///
/// 规则（保守，宁可不改）：
/// - 仅当**现有 title 以数字开头**（疑似播放量前缀）才考虑修正；
/// - 详情文案里若**也出现**同一个数字（可隔 ≤4 个非数字字符）→
///   说明那个数字属于文案（`4000亿…` / `30+亿…`），**不能剥**；
/// - 详情文案里**没有**这个数字 → 它是列表页粘上来的播放量前缀，该剥；
///   但剥掉后剩下的文案必须**出现在详情文案里**，否则说明判断错了，不改。
///
/// 详情文案里的「`第8集 | `」这类集数前缀会被剥掉：它属于正文编排，不属于标题。
fn corrected_title_from_detail(list_title: &str, detail_text: &str) -> Option<String> {
    let raw = list_title.trim();
    if raw.is_empty() {
        return None;
    }
    // 只处理「以数字开头」的疑似播放量前缀
    let lead: String = raw.chars().take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '+').collect();
    if lead.is_empty() {
        return None;
    }
    // 剥掉数字（及其后的单位）后的剩余文案
    let mut rest = raw[lead.len()..].trim_start();
    for unit in ["万", "w", "W", "k", "K"] {
        if let Some(stripped) = rest.strip_prefix(unit) {
            rest = stripped.trim_start();
            break;
        }
    }
    if rest.len() < 6 {
        return None;
    }
    // 详情文案（剥掉集数前缀）里若**也出现**同一个数字 → 那个数字属于文案
    // （`4000亿…` / `30+亿…` / `2025车圈…`），不能剥。
    let detail = strip_episode_prefix(detail_text);
    if detail.is_empty() {
        return None;
    }
    let digits: String = lead.chars().filter(|c| c.is_ascii_digit() || *c == '.').collect();
    if digits.is_empty() {
        return None;
    }
    if detail_contains_number_near(&detail, &digits) {
        return None;
    }
    // 详情里**没有**这个数字 → 它是列表页粘上来的播放量前缀，该剥。
    // 但剥掉后剩余文案必须**出现在详情里**，否则说明剥错了 → 不改。
    if !detail.contains(rest) {
        return None;
    }
    Some(rest.to_string())
}

/// 剥掉抖音文案开头的集数前缀：`第8集 | ` / `第 3 期｜` / `EP12 ` 等。
fn strip_episode_prefix(raw: &str) -> String {
    let text = raw.trim();
    let bytes: Vec<char> = text.chars().collect();
    // 「第 + 数字 + 集/期 + 可选分隔符」
    if bytes.first().map(|c| *c == '第').unwrap_or(false) {
        let mut idx = 1usize;
        // 数字与「集/期」之间可能有空格：`第 3 期｜…`
        while idx < bytes.len() && (bytes[idx].is_ascii_digit() || bytes[idx].is_whitespace()) {
            idx += 1;
        }
        if idx > 1 && idx < bytes.len() && (bytes[idx] == '集' || bytes[idx] == '期') {
            idx += 1;
            // 跳过可选的分隔符与空白
            while idx < bytes.len() && (bytes[idx].is_whitespace() || matches!(bytes[idx], '|' | '｜' | '·' | '・')) {
                idx += 1;
            }
            return bytes[idx..].iter().collect::<String>().trim().to_string();
        }
    }
    text.to_string()
}

/// 详情文案里是否「紧邻着」（可隔 ≤4 个非数字字符）出现指定数字。
fn detail_contains_number_near(detail: &str, digits: &str) -> bool {
    if digits.is_empty() {
        return false;
    }
    let chars: Vec<char> = detail.chars().collect();
    let dchars: Vec<char> = digits.chars().collect();
    if dchars.len() > chars.len() {
        return false;
    }
    for start in 0..=chars.len().saturating_sub(dchars.len()) {
        // 前面最多允许 4 个非数字字符
        let mut back = 0usize;
        while back <= 4 && start > back {
            let c = chars[start - back - 1];
            if c.is_ascii_digit() {
                break;
            }
            back += 1;
        }
        if back > 4 {
            continue;
        }
        if chars[start..start + dchars.len()] == dchars[..] {
            return true;
        }
    }
    false
}

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

    // 导出根 → 平台 → 日期 → 作者 → 标题前 20 字
    //
    // 注意 `date` 取的是**导出当天**（不是发布时间），所以同一篇帖子在不同日子被
    // 自动导出（转写重试成功、重新下载、重新抽帧都会触发）会算出一组全新的 l3。
    // 只用 `find_own_export_dir(l3)` 去重的话跨天必然找不到 → 再建一份同名素材。
    // 所以这里按 `.beav-post-id` 在**整个平台目录**下横向找一遍，先复用再说。
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

    let title_prefix = title_export_prefix(&post.title, EXPORT_TITLE_PREFIX_LEN);
    // 先看当天当天作者（快路径），再横向扫平台目录下的历史日期 / 历史作者名。
    let l4 = match find_own_export_dir(&l3, post_id)
        .or_else(|| find_export_dir_for_post(&l1, post_id))
    {        Some(existing) => {
            // 复用旧目录时标题可能已被清洗/纠正：沿用旧名会让素材库一直顶着脏标题，
            // 所以在同一个作者目录内就地改名。目标名已被占用（别的帖子同名前缀）时保持不动，
            // 宁可留旧名也不覆盖别人的目录。
            let renamed = match existing.file_name().and_then(|name| name.to_str()) {
                Some(current) if current != title_prefix => existing
                    .parent()
                    .map(|parent| parent.join(&title_prefix))
                    .filter(|target| !target.exists())
                    .and_then(|target| std::fs::rename(&existing, &target).ok().map(|_| target)),
                _ => None,
            };
            renamed.unwrap_or(existing)
        }
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

    // 爆文拆解（六要素 + 结构骨架）
    if let Some(d) = &decon {
        let json = serde_json::to_string_pretty(d).unwrap_or_default();
        let _ = std::fs::write(l4.join("爆文拆解.json"), json);
        let bullets = |items: &[String]| -> String {
            if items.is_empty() {
                "（无）".to_string()
            } else {
                items
                    .iter()
                    .map(|item| format!("- {item}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        };
        // 结构骨架单独成表：时间码是拆解里最有用、也最容易被编造的字段，
        // 摆在明面上才能一眼看出模型有没有照着真实字幕走。
        let structure_md = if d.structure.is_empty() {
            "（本条没有可用的时间轴结构）".to_string()
        } else {
            let mut table =
                String::from("| 时间码 | 段落功能 | 占比 | 这一段做了什么 |\n| --- | --- | --- | --- |\n");
            for segment in &d.structure {
                table.push_str(&format!(
                    "| {} | {} | {} | {} |\n",
                    if segment.start.trim().is_empty() {
                        "—"
                    } else {
                        segment.start.trim()
                    },
                    segment.role,
                    segment
                        .weight
                        .map(|weight| format!("{weight:.0}%"))
                        .unwrap_or_else(|| "—".to_string()),
                    segment.gist
                ));
            }
            table
        };
        let md = format!(
            "# 爆文拆解 · {}\n\n作者：{}\n\n> 拆解依据：{}\n\n## 选题角度\n{}\n\n## 文案结构\n{}\n\n## 结构骨架\n{}\n\n## 开场白 / 前3秒钩子\n{}\n\n## 标题句式\n{}\n\n## 金句 / 钩子\n{}\n\n## 可复用方法论\n{}\n\n---\n\n### 拆解所用原文\n{}\n",
            d.post_title,
            d.author,
            d.source_evidence,
            d.topic_angle,
            d.copy_structure,
            structure_md,
            d.opening,
            bullets(&d.title_patterns),
            bullets(&d.hooks),
            d.reusable_takeaway,
            d.raw_content
        );
        let _ = std::fs::write(l4.join("爆文拆解.md"), md);
    }

    // 二创产出（分镜脚本 / 各平台改写）：落到「二创/」子目录，和拆解并列
    let remixes: Vec<crate::PostRemixRecord> = with_store(state, |store| {
        Ok(store
            .post_remixes
            .iter()
            .filter(|item| item.post_id == post_id)
            .cloned()
            .collect())
    })?;
    if !remixes.is_empty() {
        let remix_dir = l4.join("二创");
        if std::fs::create_dir_all(&remix_dir).is_ok() {
            for item in &remixes {
                let file_name = if item.kind == "rewrite" {
                    format!(
                        "改写-{}.md",
                        if item.target.trim().is_empty() {
                            "default"
                        } else {
                            item.target.trim()
                        }
                    )
                } else {
                    "分镜脚本.md".to_string()
                };
                let label = if item.kind == "rewrite" { "多平台改写" } else { "分镜脚本" };
                let body = format!(
                    "# {label} · {}\n\n> 生成依据：{}\n\n{}\n",
                    item.post_title, item.source_evidence, item.content
                );
                let _ = std::fs::write(remix_dir.join(file_name), body);
            }
        }
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
    if crate::managed_dirs_locked() {
        return Err("导出目录由服务器统一管理（Web 多用户模式），不能在客户端修改".to_string());
    }
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
    if crate::managed_dirs_locked() {
        return Err("导出目录由服务器统一管理（Web 多用户模式），不能在客户端选择".to_string());
    }
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

/// 补历史的年份下限：只收录 `BACKFILL_YEAR_MIN` 年（含）以后发布的动态，
/// 更早的历史帖在补完正文、拿到发布日期后直接删掉（连同互动快照）。
const BACKFILL_YEAR_MIN: i32 = 2026;

/// 判断 `published_at` 是否早于补历史年份下限。
///
/// 日期文案随平台而异：`2024-03-15`（小红书 ISO 截断）/ `2024年3月15日` / `03-15`（无年份）
/// / `3天前`（相对日期）。规则取保守侧：
/// - 能解析出**四位年份**且 < 下限 → 早于下限（要删）；
/// - 解析不出年份（无年份短格式、相对日期通常都表示近期）→ 视为不早于下限，
///   宁可多留，也不能因为解析失败把 2026 年的帖子误删。
fn published_before_cutoff(published_at: Option<&str>) -> bool {
    let Some(text) = published_at.map(str::trim).filter(|s| !s.is_empty()) else {
        return false;
    };
    extract_four_digit_year(text).is_some_and(|year| year < BACKFILL_YEAR_MIN)
}

/// 从文本里取出第一段**独立**的四位年份数字（前后都不是数字才算边界）。
///
/// 只取独立四位段是为了避开两种误配：五位以上数字（时间戳 `1793952000000`）
/// 和被夹在更长数字里的子串。2026 年以后平台若改文案，这里只会退化为
/// 「解析不出年份 → 保留」，不会反向误删。
fn extract_four_digit_year(text: &str) -> Option<i32> {
    let chars: Vec<char> = text.chars().collect();
    // 短于 4 个字符不可能含四位年份。必须先行返回：`len - 4` 在 len < 4 时会下溢，
    // 写成 `0..=len.saturating_sub(4)` 则退化成 `0..=0`，随后 `chars[0..4]` 直接 panic
    // （"3天前" 这种相对日期正好 3 个字符，是最常见的踩点）。
    if chars.len() < 4 {
        return None;
    }
    for i in 0..=(chars.len() - 4) {
        let window = &chars[i..i + 4];
        if !window.iter().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let left_ok = i == 0 || !chars[i - 1].is_ascii_digit();
        let right_ok = i + 4 >= chars.len() || !chars[i + 4].is_ascii_digit();
        if left_ok && right_ok {
            let year: String = window.iter().collect();
            return year.parse().ok();
        }
    }
    None
}

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
    /// 本次因发布年份早于 `BACKFILL_YEAR_MIN` 被丢弃的历史帖数
    dropped_old_posts: i64,
    /// 待处理的历史视频帖数（历史视频不自动下载，交给用户手动）
    pending_videos: i64,
    /// 因已有「已补历史」标记而被跳过的博主数
    already_backfilled: i64,
    /// 服务器该平台未登录而跳过的博主数（游客视角只能拿到首屏）
    not_logged_in: i64,
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

    // —— 每日任务前置：按平台自动拉起 Chrome ——
    // 平台账号是共享的，cookie 存在各自 profile 里，凌晨不需要人扫码；但「用户前一天
    // 把 Chrome 关掉」才是常态。不主动拉起的话整轮会是：列表抓不到 → newPosts=0 →
    // 正文/视频全 0，界面上只看到一堆 0，真相只留在一条 warn 日志里（2026-11-03 那轮
    // 就是这么静默失败的）。这里先补齐，拉不起来的平台记进 summary.chromeMissing。
    let mut chrome_missing: Vec<String> = Vec::new();
    {
        // 只处理需要 CDP 的平台（小红书 / 抖音 / B 站），并按平台去重。
        // 注意不能让某个博主缺少 platform 字段就整批跳过，回退到从主页 URL 推断。
        let mut platforms: Vec<String> = Vec::new();
        for watch in &watches {
            let platform = if watch.platform.trim().is_empty() {
                watch
                    .profile_url
                    .as_deref()
                    .map(platform_of)
                    .unwrap_or_default()
            } else {
                watch.platform.clone()
            };
            let is_cdp = Platform::parse(&platform)
                .map(|parsed| parsed.is_cdp())
                .unwrap_or(false);
            if is_cdp && !platforms.iter().any(|known| known == &platform) {
                platforms.push(platform);
            }
        }
        for platform in platforms {
            // 会话还活着就直接复用（launch 内部也会复用，这里先判掉省掉 8s 等待）。
            if headless_browser::chrome_session_alive(&platform) {
                continue;
            }
            let Some(chrome_path) = headless_browser::detect_chrome_path() else {
                chrome_missing.push(platform);
                continue;
            };
            // 用 home_url 而不是 login_url：绝大多数时候 profile 里的登录态还在，
            // 直接开首页就能抓；真掉登录才需要人去「登录管理」重新扫码。
            let landing = headless_browser::qr_platform_spec(&platform)
                .map(|spec| spec.home_url)
                .unwrap_or("about:blank");
            let launched = headless_browser::profile_dir_for(&app, &platform).and_then(|profile| {
                headless_browser::launch_chrome_for_login(
                    &chrome_path,
                    &profile,
                    &platform,
                    landing,
                )
            });
            match launched {
                Ok(port) => {
                    println!("[monitor daily] auto-launched chrome for {platform} on port {port}");
                    crate::logging::emit_legacy_line(
                        crate::logging::event::LogSource::Host,
                        crate::logging::event::LogLevel::Info,
                        "monitor",
                        "monitor.daily.chrome_launched",
                        format!("[monitor daily] auto-launched chrome for {platform}: port {port}"),
                        json!({ "platform": platform, "port": port }),
                        None,
                    );
                }
                Err(error) => {
                    chrome_missing.push(platform.clone());
                    crate::logging::emit_legacy_line(
                        crate::logging::event::LogSource::Host,
                        crate::logging::event::LogLevel::Warn,
                        "monitor",
                        "monitor.daily.chrome_launch_failed",
                        format!(
                            "[monitor daily] auto-launch chrome for {platform} failed: {error}"
                        ),
                        json!({ "platform": platform, "error": error }),
                        None,
                    );
                }
            }
        }
    }

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

    // 视频候选先攒着，等所有博主都处理完再统一跑两阶段流水线（见下方 run_videos_two_phase）。
    // 原来是在「博主 → 帖子」的双重循环里「下载一条 → 转写一条」串行做，
    // 10 条视频就要串 10 轮，每轮都含 下载(~30s) + 抽帧 + 转写(~20s)。
    let mut pending_videos: Vec<String> = Vec::new();

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

            // 3) 只登记视频候选（仅小红书 / 抖音，且尚未下载过）——
            //    真正的下载 + 转写在所有博主处理完后由 run_videos_two_phase 统一并行跑。
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
                pending_videos.push(id);
            }
        }
    }

    // 3) 视频阶段：两阶段并行（下载 VIDEO_PARALLEL_WORKERS 路 → 转写 TRANSCRIBE_PARALLEL_WORKERS 路）。
    //    与补历史的 spawn_video_workers_ex 同构，区别只在于这里必须**同步等待并拿回计数**
    //    写进 summary，否则前端看到的 videosDownloaded / videosTranscribed 会恒为 0。
    let (videos_done, transcribed_done, video_failed) =
        run_videos_two_phase(&app, pending_videos);
    stats.videos += videos_done;
    stats.transcribed += transcribed_done;
    stats.failed += video_failed;

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
        // 自动拉起失败的平台（拉不起来 → 该平台整轮抓不到东西，界面要显式提示，
        // 否则用户只看到一排 0，以为「今天没更新」）。
        "chromeMissing": chrome_missing,
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

/// 每日预取的视频阶段：**同步**跑两阶段并行流水线并返回统计。
///
/// - 阶段一（`VIDEO_PARALLEL_WORKERS` 路）：下载 → 抽帧。
///   抽帧用 `analyze_video({ skipTranscript: true })`，它会建好 `post_video_analyses`
///   记录并把 `transcript_status` 置为 `pending`，阶段二据此接棒。
/// - 阶段二（`TRANSCRIBE_PARALLEL_WORKERS` 路）：`transcribe_video_only` 只补语音转写。
///
/// 为什么不在下载 worker 里顺手转写：转写是「抽音频 → 上传 → 等 ASR 返回」，单条 10~40s，
/// 串进去会把并行度吃掉（补历史那边已有同样结论）。分阶段后两条流水线互不阻塞。
///
/// 与 `spawn_video_workers_ex` 的区别只有一点：那个是 fire-and-forget（后台补历史用），
/// 这里是**同步等待**——每日预取要把 `videosDownloaded` / `videosTranscribed` / `failed`
/// 写进 summary 返回给前端，异步版本这三个数会恒为 0。
///
/// 返回 `(下载成功数, 转写成功数, 失败数)`；失败数含下载失败、抽帧失败、转写失败。
fn run_videos_two_phase(app: &AppHandle, video_ids: Vec<String>) -> (i64, i64, i64) {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicI64, Ordering};
    use std::sync::{Arc, Mutex};

    let total = video_ids.len();
    if total == 0 {
        return (0, 0, 0);
    }

    let queue = Arc::new(Mutex::new(video_ids.into_iter().collect::<VecDeque<String>>()));
    let pending = Arc::new(Mutex::new(Vec::<String>::new()));
    let downloaded = Arc::new(AtomicI64::new(0));
    let transcribed = Arc::new(AtomicI64::new(0));
    let failed = Arc::new(AtomicI64::new(0));

    // 阶段一：并行下载 + 抽帧（跳过转写）
    let download_workers = VIDEO_PARALLEL_WORKERS.min(total).max(1);
    std::thread::scope(|scope| {
        for _ in 0..download_workers {
            let app = app.clone();
            let queue = Arc::clone(&queue);
            let pending = Arc::clone(&pending);
            let downloaded = Arc::clone(&downloaded);
            let failed = Arc::clone(&failed);
            scope.spawn(move || {
                let state = app.state::<AppState>();
                loop {
                    // 锁中毒也要取回内层数据：否则一次 panic 会把剩余队列静默吐掉。
                    let next = queue
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .pop_front();
                    let Some(id) = next else { break };

                    match download_video(&app, &state, &json!({ "id": id.clone() })) {
                        // `download_video` 在「平台不支持 / Chrome 未连接」这类前置条件下
                        // 返回的是 `Ok({success:false, note})` 而不是 Err —— 那种情况并没有
                        // 落盘。以前只判 Err 就 `downloaded.fetch_add`，会让 videosDownloaded
                        // 虚高，而 videoLocalPath 仍是空、阶段二逐条失败。这里判 success 字段。
                        Ok(value) => {
                            let ok = value
                                .get("success")
                                .and_then(Value::as_bool)
                                .unwrap_or(true);
                            if !ok {
                                let note = value
                                    .get("note")
                                    .and_then(Value::as_str)
                                    .unwrap_or("未知原因")
                                    .to_string();
                                failed.fetch_add(1, Ordering::Relaxed);
                                crate::logging::emit_legacy_line(
                                    crate::logging::event::LogSource::Host,
                                    crate::logging::event::LogLevel::Warn,
                                    "monitor",
                                    "monitor.daily.download_skipped",
                                    format!("[monitor daily] video download for {id} skipped: {note}"),
                                    json!({ "postId": id, "note": note }),
                                    None,
                                );
                                continue;
                            }
                        }
                        Err(error) => {
                            failed.fetch_add(1, Ordering::Relaxed);
                            crate::logging::emit_legacy_line(
                                crate::logging::event::LogSource::Host,
                                crate::logging::event::LogLevel::Warn,
                                "monitor",
                                "monitor.daily.download_failed",
                                format!("[monitor daily] video download for {id} failed: {error}"),
                                json!({ "postId": id, "error": error }),
                                None,
                            );
                            continue;
                        }
                    }
                    downloaded.fetch_add(1, Ordering::Relaxed);

                    match analyze_video(
                        &app,
                        &state,
                        &json!({ "id": id.clone(), "skipTranscript": true }),
                    ) {
                        Ok(_) => pending
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .push(id),
                        Err(error) => {
                            failed.fetch_add(1, Ordering::Relaxed);
                            crate::logging::emit_legacy_line(
                                crate::logging::event::LogSource::Host,
                                crate::logging::event::LogLevel::Warn,
                                "monitor",
                                "monitor.daily.analyze_failed",
                                format!("[monitor daily] video analyze for {id} failed: {error}"),
                                json!({ "postId": id, "error": error }),
                                None,
                            );
                        }
                    }
                }
            });
        }
    });

    // 阶段二：并行转写（只补转写，不重下不重抽帧）
    let batch: Vec<String> = std::mem::take(
        &mut *pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
    );
    let batch_total = batch.len();
    if batch_total > 0 {
        let tqueue = Arc::new(Mutex::new(batch.into_iter().collect::<VecDeque<String>>()));
        let transcribe_workers = TRANSCRIBE_PARALLEL_WORKERS.min(batch_total).max(1);
        std::thread::scope(|scope| {
            for _ in 0..transcribe_workers {
                let app = app.clone();
                let tqueue = Arc::clone(&tqueue);
                let transcribed = Arc::clone(&transcribed);
                let failed = Arc::clone(&failed);
                scope.spawn(move || {
                    let state = app.state::<AppState>();
                    loop {
                        let next = tqueue
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .pop_front();
                        let Some(id) = next else { break };
                        match transcribe_video_only(&app, &state, &json!({ "id": id.clone() })) {
                            Ok(_) => {
                                transcribed.fetch_add(1, Ordering::Relaxed);
                            }
                            Err(error) => {
                                failed.fetch_add(1, Ordering::Relaxed);
                                crate::logging::emit_legacy_line(
                                    crate::logging::event::LogSource::Host,
                                    crate::logging::event::LogLevel::Warn,
                                    "monitor",
                                    "monitor.daily.transcribe_failed",
                                    format!("[monitor daily] transcribe for {id} failed: {error}"),
                                    json!({ "postId": id, "error": error }),
                                    None,
                                );
                            }
                        }
                    }
                });
            }
        });
    }

    (
        downloaded.load(Ordering::Relaxed),
        transcribed.load(Ordering::Relaxed),
        failed.load(Ordering::Relaxed),
    )
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
    // 落库诊断累计（抓到的条数 / 已存在 / 域名不符丢弃）
    let mut scraped_total = 0i64;
    let mut updated_total = 0i64;
    let mut dropped_total = 0i64;
    // 实际执行了几次 fetch（>1 说明被重复触发了）
    let mut fetch_runs = 0i64;
    // 主页声明的作品总数（进度条分母）：取各博主中的最大值
    let mut profile_total_max = 0i64;
    // 每个博主的滚动诊断（真机排查「只翻到首屏」用）
    let mut scroll_diags: Vec<(String, Value)> = Vec::new();
    // 每个博主的视频诊断：(名称, 帖总数, mediaType=video 的数, mediaType 缺失的数, 本次待下载数)
    let mut video_diag: Vec<(String, i64, i64, i64, i64)> = Vec::new();

    for watch in &watches {
        // 该平台本次的登录态（写进滚动诊断，便于一次判断「未登录」还是「页面不给更多」）
        let mut login_state: Option<bool> = None;
        // 已补过且不是强制 → 跳过（避免每晚重复深翻）
        if !force && history_backfill_valid(watch.history_backfilled_at.as_deref()) {
            stats.already_backfilled += 1;
            continue;
        }
        stats.watched += 1;

        // 前置检查：服务器该平台未登录时，深翻只能拿到**游客视角的首屏**（通常 3-5 条），
        // 表现和"翻页坏了"一模一样。账号被顶掉/cookie 过期都会走到这里，
        // 所以先说清楚，别让人误判成抓取 bug。
        if let Some(platform) = crate::platform::Platform::parse(&watch.platform) {
            let platform_key = platform.as_str();
            let port = headless_browser::active_chrome_port(platform_key);
            let logged_in = if port == 0 {
                false
            } else {
                match tauri::async_runtime::block_on(headless_browser::ws_url_for_port(port)) {
                    Ok(ws_url) => tauri::async_runtime::block_on(
                        headless_browser::login_logged_in(&ws_url, platform_key),
                    )
                    .unwrap_or(false),
                    Err(_) => false,
                }
            };
            if !logged_in {
                stats.not_logged_in += 1;
                let note = format!(
                    "服务器未登录{}，只拿到游客视角的首屏；请先扫码登录平台账号再补历史",
                    platform.label()
                );
                if stats.skip_note.is_none() {
                    stats.skip_note = Some(note.clone());
                }
                crate::logging::emit_legacy_line(
                    crate::logging::event::LogSource::Host,
                    crate::logging::event::LogLevel::Warn,
                    "monitor",
                    "monitor.history.not_logged_in",
                    format!("[monitor history] {}: {}", watch.name, note),
                    json!({ "watchId": watch.id, "platform": platform_key }),
                    None,
                );
                continue;
            }
            login_state = Some(true);
        }

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
        fetch_runs += 1;
        let scanned = result.get("scanned").and_then(Value::as_i64).unwrap_or(0);
        scraped_total += result.get("scrapedPosts").and_then(Value::as_i64).unwrap_or(0);
        updated_total += result.get("updatedExisting").and_then(Value::as_i64).unwrap_or(0);
        dropped_total += result.get("droppedDomain").and_then(Value::as_i64).unwrap_or(0);
        let exhausted = result
            .get("historyExhausted")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        // 滚动诊断：告诉用户「为什么只翻到 N 条」（首屏就停 / 滚错容器 / 页面不给更多）
        if let Some(diag) = result.get("scrollDiag").filter(|value| !value.is_null()) {
            // 带上登录态：游客视角只能拿到首屏几条，有登录态还停在几条 = 页面不给更多
            let mut diag = diag.clone();
            if let Some(obj) = diag.as_object_mut() {
                obj.insert(
                    "loggedIn".to_string(),
                    json!(login_state.unwrap_or(false)),
                );
            }
            scroll_diags.push((watch.name.clone(), diag.clone()));
            // 落日志：桌面端补历史时也能直接 grep 到，不用靠界面
            crate::logging::emit_legacy_line(
                crate::logging::event::LogSource::Host,
                crate::logging::event::LogLevel::Warn,
                "monitor",
                "monitor.history.scroll_diag",
                format!("[monitor history] {} scroll diag: {}", watch.name, diag),
                json!({ "watchId": watch.id, "diag": diag }),
                None,
            );
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

        // —— 历史视频：必须在「补正文」之后启动 ——
        //
        // 视频与否由 `media_type` 决定，而 `media_type` 只有**抓详情页**时才会被修正
        // （post_detail 里 detail.mediaType 落库）。列表抓取拿不到媒体类型，
        // 新帖的默认值是 "image"。
        //
        // 原实现把视频筛选放在补正文**之前**（想靠并行省时间），于是筛选时
        // media_type 全是 "image" → 一个视频帖都选不出来 → 「补齐了 47 篇但
        // 一个视频都没下载」。小红书 URL 是 /explore/xxx，兜底条件
        // `post_url.contains("/video/")` 也匹配不上。
        //
        // 每日增量路径（run_daily_prefetch）本来就是「先 enrich_detail 再判视频」，
        // 补历史这里改成同样的顺序：先补正文（顺带把 media_type 修正准），
        // 再挑出视频帖丢给 worker。
        //
        // 补全正文：本次滚出的帖【全部】补正文（不再受固定 200 截断），
        // 再用「存量里还没抓过正文」的旧帖补满到 BACKFILL_MAX（近期窗口 500）。
        let mut detail_targets: Vec<String> = created
            .iter()
            .filter_map(|post| post.get("id").and_then(Value::as_str).map(String::from))
            .collect();
        // 新帖不够填满窗口时，用「存量里还没抓过正文」的补上（同样是最近的优先）。
        // 年份下限同样生效：已知发布日期早于下限的存量帖不再花详情抓取的额度。
        if detail_targets.len() < BACKFILL_MAX {
            let backlog: Vec<String> = with_store(&state, |store| {
                let mut pending: Vec<(i64, String)> = store
                    .monitored_posts
                    .iter()
                    .filter(|p| p.watch_id.as_deref() == Some(watch.id.as_str()))
                    .filter(|p| Platform::parse(&p.platform).map(|x| x.is_cdp()).unwrap_or(false))
                    .filter(|p| !p.detail_fetched && p.content.trim().is_empty())
                    .filter(|p| !published_before_cutoff(p.published_at.as_deref()))
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

        // —— 年份下限：BACKFILL_YEAR_MIN 年以前发布的历史动态直接丢弃 ——
        //
        // 日期只有补完正文才拿得到（主页列表不带发布时间），所以删除动作放在这里。
        // 两个位置约束：
        // 1. 必须先于「视频挑选」执行，否则下架前的老视频会照常排进下载队列，
        //    用户看到的就是「补完历史，开始下载 2019 年的视频」；
        // 2. 快照随帖一起删（与 post_remove 的级联语义一致），不留给复盘当孤儿。
        let dropped_ids: Vec<String> = with_store_mut(&state, |store| {
            let stale: Vec<String> = store
                .monitored_posts
                .iter()
                .filter(|p| detail_targets.contains(&p.id))
                .filter(|p| published_before_cutoff(p.published_at.as_deref()))
                .map(|p| p.id.clone())
                .collect();
            if !stale.is_empty() {
                store.monitored_posts.retain(|p| !stale.contains(&p.id));
                store.post_snapshots.retain(|s| !stale.contains(&s.post_id));
            }
            Ok(stale)
        })?;
        if !dropped_ids.is_empty() {
            // 计数修正：new_posts 在上文按 created 全量计过，被年份砍掉的要扣回去。
            // （backlog 回填的计数 stats.backfilled 同理不再修正 —— 该数字表示「补了
            // 多少条正文」，正文确实补了，只是落库前被删，统计口径不变。）
            let dropped_created = created
                .iter()
                .filter(|post| {
                    post.get("id")
                        .and_then(Value::as_str)
                        .is_some_and(|id| dropped_ids.iter().any(|x| x == id))
                })
                .count() as i64;
            stats.new_posts -= dropped_created;
            stats.dropped_old_posts = dropped_ids.len() as i64;
            crate::logging::emit_legacy_line(
                crate::logging::event::LogSource::Host,
                crate::logging::event::LogLevel::Info,
                "monitor",
                "monitor.history.dropped_old",
                format!(
                    "[monitor history] {}: dropped {} posts published before {BACKFILL_YEAR_MIN}",
                    watch.name,
                    dropped_ids.len()
                ),
                json!({ "watchId": watch.id, "dropped": dropped_ids.len(), "minYear": BACKFILL_YEAR_MIN }),
                None,
            );
        }

        // 补完正文后，media_type 已被详情页修正为真实的 "video" / "image"，
        // 这时才能准确挑出视频帖（顺序要求见上面的说明）。
        // 幂等：只处理 video_status=="none" 的帖；取流与详情各开独立 CDP target，互不干扰。
        //
        // 同时统计该博主的媒体类型分布 —— 「补齐了 47 篇但一个视频都没下载」时，
        // 看这行就能立刻分辨是「压根没识别出视频帖」还是「识别出来了但下载失败」。
        let (video_ids, video_total, video_type_video, video_type_missing) = with_store(
            &state,
            |store| {
                let mut ids: Vec<String> = Vec::new();
                let mut total = 0i64;
                let mut typed_video = 0i64;
                let mut missing_type = 0i64;
                for post in store
                    .monitored_posts
                    .iter()
                    .filter(|p| p.watch_id.as_deref() == Some(watch.id.as_str()))
                    .filter(|p| Platform::parse(&p.platform).map(|x| x.is_cdp()).unwrap_or(false))
                {
                    total += 1;
                    if post.media_type.trim().is_empty() {
                        missing_type += 1;
                    } else if post.media_type == "video" {
                        typed_video += 1;
                    }
                    if (post.media_type == "video" || post.post_url.contains("/video/"))
                        && post.video_status == "none"
                    {
                        ids.push(post.id.clone());
                    }
                }
                Ok((ids, total, typed_video, missing_type))
            },
        )?;
        stats.pending_videos += video_ids.len() as i64;
        video_diag.push((watch.name.clone(), video_total, video_type_video, video_type_missing, video_ids.len() as i64));
        if !video_ids.is_empty() {
            spawn_video_workers(app.clone(), watch.id.clone(), video_ids);
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
    // 提示等级：ok = 正常（含"已抓全"），warn = 需要关注
    let mut hint_level: &str = "ok";
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
        let profile_total = diag
            .get("profileTotal")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let captured = counts.last().copied().unwrap_or(0);
        let first = counts.first().copied().unwrap_or(0);
        let grew = counts.len() >= 2 && captured > first;
        // 抓到的数量达到主页声明的作品数 → 已经抓全，不该报警
        let reached_total = profile_total > 0 && captured >= profile_total;
        // 主页声明数解析不到时（小红书新版文案可能不是"47 笔记"），
        // 用滚动位置判断是否真的到底 —— 到底且不再增长就是抓全了。
        let reached_bottom = diag
            .get("reachedBottom")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let eval_errors = diag
            .get("evalErrors")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let all_done = reached_total || (profile_total == 0 && reached_bottom);

        let text = if eval_errors > 0 && !reached_total {
            // 求值失败 = 页面连接断过（标签被关/CDP 断开），结果可能不完整，
            // 绝不能说成"已抓全"。
            format!("{name}：抓取过程中页面连接中断 {eval_errors} 次（标签可能被关闭），本次结果可能不完整")
        } else if reached_total {
            format!("{name}：已抓全 {captured} 篇（与主页声明的 {profile_total} 篇一致）")
        } else if all_done {
            format!("{name}：已翻到主页底部，共抓到 {captured} 篇")
        } else if !scrolled {
            format!("{name}：页面没有发生滚动（滚动容器可能选错）")
        } else if !grew {
            format!(
                "{name}：滚动后没有加载出更多内容（停在 {captured} 条，共 {} 轮）",
                counts.len()
            )
        } else if profile_total > 0 {
            format!("{name}：抓到 {captured} 篇，主页声明 {profile_total} 篇（可能还有未加载的）")
        } else {
            format!("{name}：滚动加载正常（{first} → {captured} 条）")
        };
        if !all_done || eval_errors > 0 {
            hint_level = "warn";
        }
        if hint.is_none() {
            hint = Some(text);
        }
    }

    // 库内实况：直接读 store，定位"新建了 N 条却只显示几条"这类问题
    let (store_for_watch, store_total, store_orphan) = {
        let target_watch = watch_id.clone();
        with_store(&state, |store| {
            let total = store.monitored_posts.len() as i64;
            let for_watch = target_watch
                .as_deref()
                .map(|id| {
                    store
                        .monitored_posts
                        .iter()
                        .filter(|post| post.watch_id.as_deref() == Some(id))
                        .count() as i64
                })
                .unwrap_or(0);
            let orphan = store
                .monitored_posts
                .iter()
                .filter(|post| post.watch_id.is_none())
                .count() as i64;
            Ok((for_watch, total, orphan))
        })?
    };

    let summary = json!({
        "success": true,
        "watched": stats.watched,
        // 深翻时实际滚动到的条数
        "scanned": scanned_total,
        // 落库诊断：抓到多少条、其中多少是已存在（更新）、多少因域名不符丢弃
        "scrapedPosts": scraped_total,
        "updatedExisting": updated_total,
        "droppedDomain": dropped_total,
        // 深翻上限
        "limit": BACKFILL_MAX,
        // 主页声明的作品总数（进度条分母；未知为 0）
        "profileTotal": profile_total_max,
        // 其中 URL 未见过的 = 真正补进来的历史帖
        "newPosts": stats.new_posts,
        // 因发布年份早于下限（BACKFILL_YEAR_MIN）被丢弃的历史帖数
        "droppedBefore2026": stats.dropped_old_posts,
        "detailEnriched": stats.detail_ok,
        "detailSkipped": stats.detail_skipped,
        "skipNote": stats.skip_note,
        "backfilled": stats.backfilled,
        // 因已有标记被跳过的博主数（手动 force 时恒为 0）
        "alreadyBackfilled": stats.already_backfilled,
        // 服务器未登录平台而跳过的博主数（游客视角只能拿到首屏）
        "notLoggedIn": stats.not_logged_in,
        // 触到 200 上限（历史可能还有更多）的博主数
        "limitReached": stats.limit_reached,
        // 滚动诊断明细（每个博主：滚了多少轮 / 每轮锚点数 / 实际滚动的容器）
        "scrollDiags": scroll_diags
            .iter()
            .map(|(name, diag)| json!({ "name": name, "diag": diag }))
            .collect::<Vec<_>>(),
        // 一句话诊断提示
        "hint": hint,
        // "ok" = 正常（含已抓全）/ "warn" = 需要关注
        "hintLevel": hint_level,
        // 历史视频已转后台线程自动「下载+转写」，这里报启动处理的数量
        "pendingVideos": stats.pending_videos,
        // 视频诊断明细（每个博主：帖子总数 / mediaType=video 的数 / mediaType 缺失的数 / 本次待下载数）
        // 用来分辨「压根没识别出视频帖」还是「识别出来了但下载失败」。
        "videoDiags": video_diag
            .iter()
            .map(|(name, total, typed_video, missing_type, pending)| {
                json!({
                    "name": name,
                    "posts": total,
                    "typedVideo": typed_video,
                    "missingType": missing_type,
                    "pending": pending,
                })
            })
            .collect::<Vec<_>>(),
        "failed": stats.failed,
        // 库内实况：直接读 store，回答"新建了 47 为什么只显示 5 条"
        "fetchRuns": fetch_runs,
        "storePostsForWatch": store_for_watch,
        "storePostsTotal": store_total,
        "storePostsOrphan": store_orphan,
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
/// 启动对账：进程重启后，所有 `downloading` 都必然是上次被中断的僵尸任务，
/// 复位为 `none` 并记录原因，否则会永远卡在「下载中」且不再重试。
pub fn reconcile_interrupted_video_downloads(state: &State<'_, AppState>) {
    let _ = with_store_mut(state, |store| {
        let mut reset = 0usize;
        for post in store.monitored_posts.iter_mut() {
            if post.video_status == "downloading" {
                post.video_status = "none".to_string();
                if post.video_error.is_none() {
                    post.video_error = Some("上次下载被中断（应用重启），已复位".to_string());
                }
                reset += 1;
            }
        }
        if reset > 0 {
            crate::logging::emit_legacy_line(
                crate::logging::event::LogSource::Host,
                crate::logging::event::LogLevel::Info,
                "monitor",
                "monitor.video.reset_interrupted",
                format!("[monitor] 启动复位移位下载 {reset} 条"),
                json!({ "reset": reset }),
                None,
            );
        }
        Ok(())
    });
}

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
            Err(error) if error.starts_with("monitor busy") => {
                // 有一轮抓取正在跑（手动补历史等），本轮跳过即可，不算失败。
                continue;
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
            // 受管多用户：由实例管理器集中触发每日任务，实例内不再自己跑，避免重复。
            if std::env::var("BEAV_DISABLE_DAILY_SCHEDULER").ok().as_deref() == Some("1") {
                return;
            }
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

#[cfg(test)]
mod tests {
    use super::{
        clip_chars, detail_author_may_override, extract_four_digit_year, format_hms,
        published_before_cutoff, resolve_watch_id, srt_cues, srt_outline, watch_dedupe_key,
    };

    /// 拆解输入：SRT 必须能解析出真实时间码，大纲按步长抽样且不越界。
    #[test]
    fn srt_outline_keeps_real_timecodes() {
        let srt = "1\n00:00:00,000 --> 00:00:03,000\n开场先说结论：这条视频会颠覆你的认知\n\n\
                   2\n00:00:03,500 --> 00:00:07,000\n因为它讲的东西和常识相反\n\n\
                   3\n00:02:00,000 --> 00:02:04,000\n两分钟处是转折点\n";
        let cues = srt_cues(srt);
        assert_eq!(cues.len(), 3);
        assert_eq!(cues[0].0, 0);
        assert_eq!(cues[2].0, 120);

        // 步长 90s：0s 与 120s 各抽一条，3.5s 那条被步长跳过
        let outline = srt_outline(&cues, 90, 20);
        let lines: Vec<&str> = outline.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("【00:00:00】"));
        assert!(lines[1].starts_with("【00:02:00】"));
    }

    /// 截断按 Unicode 字符，不能按字节（否则中文会被切成乱码）。
    #[test]
    fn clip_chars_is_char_based() {
        assert_eq!(clip_chars("一二三四五", 3), "一二三…");
        assert_eq!(clip_chars("  短句  ", 10), "短句");
        assert_eq!(format_hms(3725), "01:02:05");
    }

    /// 回归守卫：有监控归属的帖子，详情页作者不得覆盖 watch.name。
    ///
    /// 实测事故：同一博主的三条抖音帖子导出成三个目录 ——「@抖音精选 App官方账号」
    /// 「@抖音科技」「拿幸·AI启示录」。前两个是详情页兜底链抓到的正文 `@提及`，
    /// 因为旧代码在 `post_detail` 里无条件 `item.author = detail.author`。
    #[test]
    fn detail_author_never_overrides_watch_author() {
        // 有归属 + 已有作者 → 拒绝覆盖（核心回归）
        assert!(!detail_author_may_override(Some("watch-1"), "拿幸·AI启示录"));
        // 有归属但作者为空 → 允许补一个
        assert!(detail_author_may_override(Some("watch-1"), ""));
        assert!(detail_author_may_override(Some("watch-1"), "   "));
        // 无归属（adhoc 临时帖）→ 允许采用详情页作者
        assert!(detail_author_may_override(None, "任意作者"));
        assert!(detail_author_may_override(None, ""));
        // 空 / 空白 watch_id 视同无归属
        assert!(detail_author_may_override(Some(""), "已有作者"));
        assert!(detail_author_may_override(Some("  "), "已有作者"));
    }

    /// 回归守卫：年份下限判定宁可漏删不可误删。
    ///
    /// 各平台 `published_at` 的实际文案差异很大（ISO 截断 / 中文日期 / 无年份短格式 /
    /// 相对日期）。解析不出年份的一律视为「不早于下限」——2026 年的帖子绝不能被删。
    #[test]
    fn published_before_cutoff_judges_conservatively() {
        // 早于 2026 → 删
        assert!(published_before_cutoff(Some("2024-03-15")));
        assert!(published_before_cutoff(Some("2024年3月15日")));
        assert!(published_before_cutoff(Some("2019-12-31 23:59")));
        // 2026 及以后 → 留
        assert!(!published_before_cutoff(Some("2026-01-01")));
        assert!(!published_before_cutoff(Some("2026-11-01")));
        // 解析不出年份 → 留（相对日期 / 无年份短格式 / 空 / 缺字段）
        assert!(!published_before_cutoff(Some("3天前")));
        assert!(!published_before_cutoff(Some("03-15")));
        assert!(!published_before_cutoff(Some("")));
        assert!(!published_before_cutoff(None));
        // 五位以上数字不是年份（时间戳被误塞进来也不能按前四位判）
        assert!(!published_before_cutoff(Some("1793952000000")));
    }

    /// 独立四位段才算年份：避开长数字子串和粘连数字。
    #[test]
    fn extract_four_digit_year_needs_digit_boundaries() {
        assert_eq!(extract_four_digit_year("2024-03-15"), Some(2024));
        assert_eq!(extract_four_digit_year("发布于 2019年12月"), Some(2019));
        assert_eq!(extract_four_digit_year("03-15"), None);
        assert_eq!(extract_four_digit_year("3天前"), None);
        assert_eq!(extract_four_digit_year("1793952000000"), None);
    }

    /// 回归守卫：已有归属的帖子不能被另一个监控项抢走。
    ///
    /// 实测踩坑：同一博主建两个监控项 / 两个用户监控同一博主时，
    /// 落库若无条件覆盖 watch_id，先抓的一方卡片上的「已收录 N 篇」会凭空归零，
    /// 看起来就像「抓到了却没存进去」。
    #[test]
    fn resolve_watch_id_keeps_existing_owner() {
        // 已有归属且原监控项还在 → 保留原归属，不被抢
        assert_eq!(
            resolve_watch_id(Some("watch-a"), Some("watch-b"), true),
            Some("watch-a".to_string())
        );
        // 孤儿记录（无归属）→ 归给当前监控项
        assert_eq!(
            resolve_watch_id(None, Some("watch-b"), false),
            Some("watch-b".to_string())
        );
        // 新记录 → 归给当前监控项
        assert_eq!(resolve_watch_id(None, None, false), None);
        // 已有归属 + 无当前监控项（临时粘贴单帖）→ 仍保留原归属
        assert_eq!(
            resolve_watch_id(Some("watch-a"), None, true),
            Some("watch-a".to_string())
        );
    }

    /// 回归守卫：原监控项已被删除时，帖子要能被新监控项接管。
    ///
    /// `watch_remove` 不删帖子，所以这些记录会永远挂在不存在的 watch 上 ——
    /// 卡片数恒为 0，再抓也永远接不回来。
    #[test]
    fn resolve_watch_id_reclaims_orphaned_watch() {
        // 原监控项已删 → 归给当前监控项
        assert_eq!(
            resolve_watch_id(Some("deleted-watch"), Some("watch-b"), false),
            Some("watch-b".to_string())
        );
        // 原监控项已删 + 没有当前监控项 → 无归属（不凭空造一个）
        assert_eq!(resolve_watch_id(Some("deleted-watch"), None, false), None);
    }

    /// 回归守卫：同一博主的判重必须忽略会变的 xsec_token。
    ///
    /// 小红书主页链接每次会话都带不同的 `xsec_token` / `xsec_source`，
    /// 带 token 比较会把同一个博主判成两个 → 重复建监控项 → 帖子互相抢归属。
    #[test]
    fn watch_dedupe_key_ignores_volatile_token() {
        let with_token = watch_dedupe_key(
            "alice",
            "xiaohongshu",
            Some("https://www.xiaohongshu.com/user/profile/abc?xsec_token=TOKEN1&xsec_source=pc_feed"),
        );
        let other_token = watch_dedupe_key(
            "alice",
            "xiaohongshu",
            Some("https://www.xiaohongshu.com/user/profile/abc?xsec_token=TOKEN2&xsec_source=pc_search"),
        );
        assert_eq!(with_token, other_token);
        assert!(with_token.is_some());
    }

    #[test]
    fn watch_dedupe_key_separates_users_and_platforms() {
        let base = watch_dedupe_key("alice", "xiaohongshu", Some("https://x.com/u/abc"));
        // 不同用户 → 不同 key（各人可以监控同一个博主）
        assert_ne!(
            base,
            watch_dedupe_key("bob", "xiaohongshu", Some("https://x.com/u/abc"))
        );
        // 不同平台 → 不同 key
        assert_ne!(
            base,
            watch_dedupe_key("alice", "douyin", Some("https://x.com/u/abc"))
        );
        // 不同博主 → 不同 key
        assert_ne!(
            base,
            watch_dedupe_key("alice", "xiaohongshu", Some("https://x.com/u/xyz"))
        );
        // 无主页 URL → 不判重（返回 None）
        assert!(watch_dedupe_key("alice", "xiaohongshu", None).is_none());
        assert!(watch_dedupe_key("alice", "xiaohongshu", Some("   ")).is_none());
    }

    /// 回归守卫：导出目录名按标题前 40 个字符取，中文栏目前缀不该把标题「吃光」。
    ///
    /// 实测事故：`【抖音独家】` 占 6 个字符，取前 10 只剩 `Open`，
    /// 三条标题完全不同的帖子撞成同一个目录名，只能靠 `_{postId}` 后缀区分，
    /// 目录树里一堆 `【抖音独家】Open_mpost-1793...` 根本认不出是哪条。
    #[test]
    fn export_title_prefix_keeps_chinese_titles_distinct() {
        let titles = [
            "【抖音独家】OpenAI Images 2.5 重磅发布",
            "【抖音独家】OpenAI 为何紧急暂停 Astra 测试？",
            "【抖音独家】OpenAI 宣布与微软分家",
        ];
        let prefixes: Vec<String> = titles
            .iter()
            .map(|t| super::title_export_prefix(t, super::EXPORT_TITLE_PREFIX_LEN))
            .collect();
        for (i, a) in prefixes.iter().enumerate() {
            for b in prefixes.iter().skip(i + 1) {
                assert_ne!(a, b, "标题前缀不该相撞：{a}");
            }
        }
        // 按 Unicode 字符切，不会把中文切坏；40 字足够装下完整短标题（尾部空格被 trim 掉）
        assert_eq!(prefixes[0], "【抖音独家】OpenAI Images 2.5 重磅发布");
        // 长标题严格按 40 字截断（常量被调小时会在这里暴露）
        let long = super::title_export_prefix(&"黄".repeat(60), super::EXPORT_TITLE_PREFIX_LEN);
        assert_eq!(long.chars().count(), super::EXPORT_TITLE_PREFIX_LEN);
        // 空标题回退
        assert_eq!(
            super::title_export_prefix("   ", super::EXPORT_TITLE_PREFIX_LEN),
            "未命名"
        );
    }

    /// 回归守卫：视频没变过就不要再抽帧。
    ///
    /// 抽帧是 `fps=1/5` 解全片，46 分钟的视频要抽 550 帧；重复点「分析视频」
    /// 或转写失败后重试时，本来只想重跑 ASR，却每次都把整片重解一遍。
    #[test]
    fn frames_reusable_only_when_video_unchanged() {
        // 没有帧 → 必须抽
        assert!(!super::frames_are_reusable(false, Some(2_000), 1_000));
        assert!(!super::frames_are_reusable(false, None, 1_000));
        // 帧比视频下载时间新 → 复用
        assert!(super::frames_are_reusable(true, Some(2_000), 1_000));
        assert!(super::frames_are_reusable(true, Some(1_000), 1_000));
        // 帧比视频旧 → 说明重新下载过，重抽
        assert!(!super::frames_are_reusable(true, Some(999), 1_000));
        // 老数据没有 video_downloaded_at → 有帧就复用
        assert!(super::frames_are_reusable(true, Some(1), 0));
        // 拿不到帧的 mtime → 不敢复用
        assert!(!super::frames_are_reusable(true, None, 1_000));
    }

    /// 回归守卫：详情抓取时用干净文案纠正列表页的脏标题。
    ///
    /// 实测：抖音列表页 title = 「播放量 + 文案 + 文案重复」，
    /// 详情页 desc 是干净的。修不好就只能靠事后清洗脚本回洗，新数据还会持续变脏。
    #[test]
    fn corrected_title_strips_play_count_prefix_only_when_detail_agrees() {
        // 典型场景：列表脏、详情干净 → 剥掉播放量前缀
        assert_eq!(
            super::corrected_title_from_detail(
                "1.8万日本资产泡沫，哪哪都是泡泡~ #财经 #日本经济",
                "日本资产泡沫，哪哪都是泡泡~ #财经 #日本经济"
            )
            .as_deref(),
            Some("日本资产泡沫，哪哪都是泡泡~ #财经 #日本经济")
        );
        // 详情带集数前缀，剥集数后仍应识别为「数字属于文案」→ 不改
        assert_eq!(
            super::corrected_title_from_detail(
                "30+亿美元！新加坡史上最大洗钱案 #洗钱罪",
                "第8集 | 30+亿美元！新加坡史上最大洗钱案 #洗钱罪"
            ),
            None
        );
        // 标题不以数字开头（小红书/B 站的正常标题）→ 不动
        assert_eq!(
            super::corrected_title_from_detail(
                "10年没涨价的洗衣液，究竟藏了什么猫腻？ #洗衣液",
                "10年没涨价的洗衣液，究竟藏了什么猫腻？ #洗衣液"
            ),
            None
        );
        // 详情里没有这个数字 → 说明它不是播放量前缀，只是标题内容 → 不动
        assert_eq!(
            super::corrected_title_from_detail(
                "4000亿家电一哥，如何转型？ #原来家还可以这样",
                "4000亿家电一哥，如何转型？ #原来家还可以这样"
            ),
            None
        );
        // 剥掉后剩余文案不在详情里 → 判断不可靠 → 不动
        assert_eq!(
            super::corrected_title_from_detail(
                "1.8万完全不相干的文案 #话题",
                "另一篇完全无关的正文内容"
            ),
            None
        );
    }

    /// 回归守卫：集数前缀剥离。
    #[test]
    fn strip_episode_prefix_handles_common_forms() {
        assert_eq!(super::strip_episode_prefix("第8集 | 30+亿美元！"), "30+亿美元！");
        assert_eq!(super::strip_episode_prefix("第 3 期｜真实标题"), "真实标题");
        assert_eq!(super::strip_episode_prefix("第8集真实标题"), "真实标题");
        // 不是集数前缀 → 原样返回
        assert_eq!(super::strip_episode_prefix("日本资产泡沫"), "日本资产泡沫");
        assert_eq!(super::strip_episode_prefix("第8"), "第8");
        assert_eq!(super::strip_episode_prefix(""), "");
    }

    /// 回归守卫：正文覆盖要按「脏/净」而不只是长度。
    ///
    /// 实测事故：抖音坏提取抓出的噪音版 71 字比干净文案 48 字长，
    /// 只比长度会让垃圾永久锁在库里，重新抓多少次都不生效。
    #[test]
    fn content_replace_prefers_clean_over_longer_noisy() {
        let noisy = "在慕尼黑宫殿参加了一场闭门会议... #德国7.0万20492712969举报发布时间：2025-09-19 17:30";
        let clean = "世界500强CFO纯干货分享 零基础看懂财报——利润表#财务 #会计 #财务报表 #职场 #财报";
        // 旧的脏 → 允许用更短的干净版覆盖（核心回归）
        assert!(super::should_replace_content(noisy, clean));
        // 旧的干净、旧更长 → 不覆盖（防止被标题级内容冲掉）
        assert!(!super::should_replace_content(clean, "世界500强CFO"));
        // 旧的干净、新不短 → 覆盖
        assert!(super::should_replace_content(clean, &format!("{clean}补充")));
        // 等长 → 覆盖（视为刷新）
        assert!(super::should_replace_content(clean, clean));
        // 旧为空 → 覆盖
        assert!(super::should_replace_content("", "任意内容"));
    }
}
