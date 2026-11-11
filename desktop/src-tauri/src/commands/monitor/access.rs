use super::*;

/// 普通用户可访问自己的帖子，以及共享监控项下的帖子。
pub(super) fn post_accessible(store: &AppStore, post: &MonitoredPostRecord) -> bool {
    crate::identity::owner_matches(&post.owner) || post.watch_id.as_deref().is_some_and(|id| {
        store.competitor_watchlist.iter().any(|watch| watch.id == id
            && crate::identity::can_access(&watch.owner, &watch.shared_to))
    })
}

/// 先校验全部标识（含别名和批量 ids），避免批量操作执行一半才发现越权。
pub(super) fn guard_store_target(store: &AppStore, channel: &str, payload: &Value) -> Result<(), String> {
    if crate::identity::is_admin() { return Ok(()); }
    let strict = matches!(channel, "monitor:watch-remove" | "monitor:post-remove");
    let mut ids = Vec::new();
    for key in ["id", "postId", "post_id", "watchId", "watch_id"] {
        if let Some(value) = payload.get(key) {
            ids.push(value.as_str().filter(|id| !id.trim().is_empty())
                .ok_or_else(|| "对象标识无效".to_string())?);
        }
    }
    if let Some(values) = payload.get("ids") {
        for value in values.as_array().ok_or_else(|| "ids 必须是数组".to_string())? {
            ids.push(value.as_str().filter(|id| !id.trim().is_empty())
                .ok_or_else(|| "对象标识无效".to_string())?);
        }
    }
    for id in ids {
        let allowed = if let Some(post) = store.monitored_posts.iter().find(|post| post.id == id) {
            if strict { crate::identity::owner_matches(&post.owner) } else { post_accessible(store, post) }
        } else if let Some(watch) = store.competitor_watchlist.iter().find(|watch| watch.id == id) {
            if strict { crate::identity::owner_matches(&watch.owner) }
            else { crate::identity::can_access(&watch.owner, &watch.shared_to) }
        } else { false };
        if !allowed { return Err("对象不存在或无权访问".to_string()); }
    }
    Ok(())
}

pub(super) fn guard_owned_target(state: &State<'_, AppState>, channel: &str, payload: &Value) -> Result<(), String> {
    if !matches!(channel, "monitor:watch-remove" | "monitor:post-remove" | "monitor:post-detail"
        | "monitor:fetch" | "monitor:backfill-history" | "monitor:download-video" | "monitor:analyze-video"
        | "monitor:transcribe-video" | "monitor:video-analysis" | "monitor:deconstruct"
        | "monitor:remix-generate" | "monitor:inject-topic" | "monitor:export-post"
        | "monitor:material-files" | "monitor:remix-list") { return Ok(()); }
    with_store(state, |store| guard_store_target(&store, channel, payload))
}
