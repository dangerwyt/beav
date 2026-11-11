use super::*;
use crate::{AppStore, MonitoredPostRecord};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

fn post_visible(store: &AppStore, owner: Option<&str>, post: &MonitoredPostRecord) -> bool {
    crate::identity::can_access_as(owner, &post.owner, &[])
        || post.watch_id.as_deref().is_some_and(|id| store.competitor_watchlist.iter().any(|watch| {
            watch.id == id && crate::identity::can_access_as(owner, &watch.owner, &watch.shared_to)
        }))
}

fn visible_monitor_files(store: &AppStore, owner: Option<&str>) -> (HashSet<String>, Vec<String>) {
    let mut ids = HashSet::new();
    let mut paths = Vec::new();
    for post in store.monitored_posts.iter().filter(|post| post_visible(store, owner, post)) {
        ids.insert(post.id.clone());
        paths.extend(post.video_local_path.iter().cloned());
        paths.extend(post.video_preview_path.iter().cloned());
        paths.extend(post.cover_url.iter().cloned());
        paths.extend(post.images.iter().cloned());
    }
    for analysis in store.post_video_analyses.iter().filter(|item| ids.contains(&item.post_id)) {
        paths.push(analysis.local_path.clone());
        paths.extend(analysis.frames.iter().cloned());
    }
    (ids, paths)
}

pub(super) fn file_allowed(
    state: &State<'_, AppState>, owner: Option<&str>, canonical: &Path, roots: &[PathBuf],
) -> Result<bool, String> {
    if owner.is_none() { return Ok(true); }
    let (ids, paths) = with_store(state, |store| Ok(visible_monitor_files(&store, owner)))?;
    // Resolve paths after releasing the store lock. Canonical paths also reject symlink escapes.
    if paths.iter().any(|path| Path::new(path).canonicalize().is_ok_and(|path| path == canonical)) {
        return Ok(true);
    }
    if monitor_export_allowed(canonical, roots, &ids, owner) { return Ok(true); }
    crate::media_runtime::web_artifact_allowed(state, owner, canonical)
}

fn monitor_export_allowed(
    canonical: &Path, roots: &[PathBuf], ids: &HashSet<String>, owner: Option<&str>,
) -> bool {
    for dir in canonical.parent().into_iter().flat_map(Path::ancestors) {
        if !roots.iter().any(|root| dir.starts_with(root)) { break; }
        if let Ok(id) = std::fs::read_to_string(dir.join(".beav-post-id")) {
            return ids.contains(id.trim());
        }
        if canonical.extension().is_some_and(|ext| ext == "zip") {
            if let Ok(content) = std::fs::read_to_string(dir.join(".beav-bundle.json")) {
                if let Ok(manifest) = serde_json::from_str::<Value>(&content) {
                    let same_user = manifest.get("owner").and_then(Value::as_str) == owner;
                    let visible = manifest.get("postIds").and_then(Value::as_array).is_some_and(|posts| {
                        !posts.is_empty() && posts.iter().all(|id| id.as_str().is_some_and(|id| ids.contains(id)))
                    });
                    return same_user && visible;
                }
            }
        }
    }
    false
}

fn monitor_event_payload(store: &AppStore, owner: Option<&str>, payload: &Value) -> Option<Value> {
    let watch_id = payload.get("watchId").and_then(Value::as_str)?;
    store.competitor_watchlist.iter().any(|watch| watch.id == watch_id
        && crate::identity::can_access_as(owner, &watch.owner, &watch.shared_to))
        .then(|| payload.clone())
}

pub(super) fn event_payload(
    state: &State<'_, AppState>, owner: Option<&str>, request_owner: Option<&str>, event: &str, payload: &Value,
) -> Option<Value> {
    if owner.is_none() { return Some(payload.clone()); }
    match event {
        "monitor:backfill-progress" | "monitor:prefetch-done" if payload.get("watchId").is_some() => {
            with_store(state, |store| Ok(monitor_event_payload(&store, owner, payload))).ok().flatten()
        }
        "generation:job-updated" | "generation:job-log" => {
            let job_id = payload.get("jobId").and_then(Value::as_str)?;
            crate::media_runtime::web_job_visible(state, owner, job_id).ok()
                .filter(|visible| *visible).map(|_| payload.clone())
        }
        // Invalidation subscribers refetch through IPC; no data or aggregate counts cross users.
        "monitor:prefetch-done" | "settings:updated" | "data:changed"
        | "knowledge:catalog-updated" | "knowledge:changed" | "knowledge:docs-updated"
        | "knowledge:file-index-updated" | "knowledge:note-updated"
        | "knowledge:new-youtube-video" | "knowledge:youtube-video-updated"
        | "advisors:changed" | "plugins:changed" | "auth:data-changed" | "auth:state-changed"
        | "space:changed" | "llm-readiness:state-changed" => Some(json!({})),
        // Chat/wander events emitted by a Web request return only to that authenticated user.
        // Untagged background logs remain admin-only; payload fields cannot grant access.
        "runtime:event" | "chat:session-title-updated" | "wander:progress" | "wander:result" => {
            request_event_payload(owner, request_owner, payload)
        }
        _ => None,
    }
}

fn request_event_payload(owner: Option<&str>, request_owner: Option<&str>, payload: &Value) -> Option<Value> {
    (owner.is_some() && owner == request_owner).then(|| payload.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_events_reach_only_the_authenticated_request_user() {
        let payload = json!({"sessionId":"chat-1", "content":"private reply", "owner":"bob"});
        assert_eq!(request_event_payload(Some("alice"), Some("alice"), &payload), Some(payload.clone()));
        assert!(request_event_payload(Some("bob"), Some("alice"), &payload).is_none());
        assert!(request_event_payload(Some("alice"), None, &payload).is_none());
    }

    #[test]
    fn monitor_events_and_files_follow_current_sharing() {
        let mut store = crate::persistence::default_store();
        store.competitor_watchlist.push(crate::CompetitorWatchRecord {
            id: "shared".into(), owner: "admin".into(), shared_to: vec!["alice".into()], ..Default::default()
        });
        store.monitored_posts.push(MonitoredPostRecord {
            id: "post".into(), owner: "admin".into(), watch_id: Some("shared".into()),
            video_local_path: Some("private.mp4".into()), ..Default::default()
        });
        let progress = json!({"watchId": "shared", "scanned": 2, "total": 5});
        assert_eq!(monitor_event_payload(&store, Some("alice"), &progress), Some(progress.clone()));
        assert!(monitor_event_payload(&store, Some("bob"), &progress).is_none());
        assert!(visible_monitor_files(&store, Some("alice")).0.contains("post"));
        assert!(visible_monitor_files(&store, Some("bob")).1.is_empty());
        store.competitor_watchlist[0].shared_to.clear();
        assert!(monitor_event_payload(&store, Some("alice"), &progress).is_none());
    }

    #[test]
    fn exported_files_and_bundles_require_visible_posts() {
        let root = std::env::temp_dir().join(crate::make_id("web-access-test"));
        let post = root.join("post");
        std::fs::create_dir_all(&post).unwrap();
        std::fs::write(post.join(".beav-post-id"), "p1").unwrap();
        let roots = vec![root.clone()];
        let ids = HashSet::from(["p1".to_string()]);
        assert!(monitor_export_allowed(&post.join("video.mp4"), &roots, &ids, Some("alice")));
        assert!(!monitor_export_allowed(&post.join("video.mp4"), &roots, &HashSet::new(), Some("alice")));
        let bundle = root.join("bundle");
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::write(bundle.join(".beav-bundle.json"), json!({"owner":"alice","postIds":["p1"]}).to_string()).unwrap();
        assert!(monitor_export_allowed(&bundle.join("export.zip"), &roots, &ids, Some("alice")));
        assert!(!monitor_export_allowed(&bundle.join("export.zip"), &roots, &ids, Some("bob")));
        assert!(!monitor_export_allowed(&bundle.join("export.zip"), &roots, &HashSet::new(), Some("alice")));
        std::fs::remove_dir_all(root).unwrap();
    }
}
