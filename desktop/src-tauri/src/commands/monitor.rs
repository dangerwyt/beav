mod access;
use access::*;
mod chrome;
use chrome::*;
mod watch;
use watch::*;
mod capture;
use capture::*;
mod posts;
use posts::*;
mod creation;
use creation::*;
mod video;
use video::*;
mod export;
use export::*;
mod schedule;
use schedule::*;
pub use schedule::{reconcile_interrupted_video_downloads, start_monitor_scheduler};

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
    // 扫码登录（chrome-qr-start/status）不在此列表里。
    // 平台账号是服务器上所有用户共享的，抓取本身对所有 web 用户开放；
    // cookie 过期时必须能在 web 端续上登录态（管理员不一定在服务器前）。
    // 退出登录、打开服务器 Chrome、执行调试脚本和读写 Cookie 仅限管理员。
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
        "monitor:chrome-test",
        "monitor:chrome-export-cookies",
        "monitor:chrome-import-cookies",
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

fn str_field(payload: &Value, key: &str) -> Option<String> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests;
