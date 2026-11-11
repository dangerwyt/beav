use super::*;

/// 落一条互动快照（供「数据复盘」画走势）。
///
/// 关键护栏：三项互动数加起来必须为正数才落。web 抓取路径（fetch_monitor）拿不到点赞数，
/// 写的是全零；若不拦，这些 0 值会混进按天聚合，把走势线「砸」出假下跌。只有 CDP 列表 /
/// 详情页产出的真实互动才值得入历史。
pub(super) fn push_snapshot(
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

pub(super) fn watch_add(state: &State<'_, AppState>, payload: &Value) -> Result<Value, String> {
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
pub(super) fn watch_dedupe_key(owner: &str, platform: &str, profile_url: Option<&str>) -> Option<String> {
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

pub(super) fn watch_list(state: &State<'_, AppState>, payload: &Value) -> Result<Value, String> {
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

pub(super) fn watch_remove(state: &State<'_, AppState>, payload: &Value) -> Result<Value, String> {
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
pub(super) fn watch_assign(state: &State<'_, AppState>, payload: &Value) -> Result<Value, String> {
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
pub(super) fn list_web_users(state: &State<'_, AppState>) -> Result<Value, String> {
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
pub(super) fn clear_data(state: &State<'_, AppState>) -> Result<Value, String> {
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
pub(super) fn post_remove(state: &State<'_, AppState>, payload: &Value) -> Result<Value, String> {
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
pub(super) fn purge_orphans(state: &State<'_, AppState>) -> Result<Value, String> {
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
pub(super) fn clear_posts(state: &State<'_, AppState>) -> Result<Value, String> {
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
