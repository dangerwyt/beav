use super::*;

/// 设置单次抓取条数（持久化到 store，重启后保留）。范围 [1, 200]，越界回落默认 12。
/// 兼容旧调用：抓取条数已固定为 `SCAN_LIMIT`，这里不再落库（忽略传入值）。
pub(super) fn set_scan_limit(_state: &State<'_, AppState>, _payload: &Value) -> Result<Value, String> {
    Ok(json!({
        "success": true,
        "limit": SCAN_LIMIT,
        "note": "单次抓取条数已固定为 30，不再支持自定义",
    }))
}

/// 读取当前抓取条数：恒定返回 `SCAN_LIMIT`，供前端显示静态提示。
pub(super) fn get_scan_limit(_state: &State<'_, AppState>) -> Result<Value, String> {
    Ok(json!({ "success": true, "limit": SCAN_LIMIT, "fixed": true }))
}

/// 配置异常倍数阈值（持久化到 store）。合理区间 1.0~20.0，越界/缺失回落 3.0。
pub(super) fn set_spike_threshold(state: &State<'_, AppState>, payload: &Value) -> Result<Value, String> {
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
pub(super) fn get_spike_threshold(state: &State<'_, AppState>) -> Result<Value, String> {
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
pub(super) fn project_post_for_list(
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

pub(super) fn post_list(state: &State<'_, AppState>, payload: &Value) -> Result<Value, String> {
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
pub(super) fn retro(state: &State<'_, AppState>, payload: &Value) -> Result<Value, String> {
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

pub(super) fn inject_topic(state: &State<'_, AppState>, payload: &Value) -> Result<Value, String> {
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
