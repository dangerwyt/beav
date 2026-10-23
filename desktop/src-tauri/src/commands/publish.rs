use crate::persistence::with_store_mut;
use crate::store::types::PublishLogRecord;
use crate::platform::Platform;
use crate::{app_brand_display_name, make_id, now_i64, AppState};
use serde_json::{json, Value};
use tauri::{AppHandle, State};

/// 发布环节通道分发。
///
/// 支持的通道：
/// - publish:list        列出发布登记（按创建时间倒序）
/// - publish:add         登记一条发布（平台 / 链接 / 标题 / 状态 / 指标）
/// - publish:update      更新一条发布（状态、指标、链接、备注等）
/// - publish:remove      删除一条发布登记
/// - publish:platforms   返回已知平台及其创作者后台 URL（前端快捷跳转用）
pub fn handle_publish_channel(
    _app: &AppHandle,
    state: &State<'_, AppState>,
    channel: &str,
    payload: &Value,
) -> Option<Result<Value, String>> {
    if !channel.starts_with("publish:") {
        return None;
    }
    Some(
        (|| match channel {
            "publish:list" => publish_list(state),
            "publish:add" => publish_add(state, payload),
            "publish:update" => publish_update(state, payload),
            "publish:remove" => publish_remove(state, payload),
            "publish:platforms" => Ok(platforms()),
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

fn int_field(payload: &Value, key: &str) -> i64 {
    payload.get(key).and_then(Value::as_i64).unwrap_or(0)
}

fn publish_list(state: &State<'_, AppState>) -> Result<Value, String> {
    with_store_mut(state, |store| {
        let mut items: Vec<Value> = store
            .publish_log
            .iter()
            .map(|record| serde_json::to_value(record).map_err(|error| error.to_string()))
            .collect::<Result<_, _>>()?;
        // 按创建时间倒序（新登记在前）
        items.sort_by(|a, b| {
            let ak = a.get("createdAt").and_then(Value::as_i64).unwrap_or(0);
            let bk = b.get("createdAt").and_then(Value::as_i64).unwrap_or(0);
            bk.cmp(&ak)
        });
        Ok(json!({ "success": true, "items": items }))
    })
}

fn publish_add(state: &State<'_, AppState>, payload: &Value) -> Result<Value, String> {
    let platform = str_field(payload, "platform").ok_or_else(|| "缺少 platform".to_string())?;
    let title = str_field(payload, "title").ok_or_else(|| "缺少 title".to_string())?;
    let now = now_i64();
    let record = PublishLogRecord {
        id: make_id("pub"),
        platform,
        post_url: str_field(payload, "postUrl"),
        title,
        published_at: str_field(payload, "publishedAt"),
        status: str_field(payload, "status").unwrap_or_else(|| "planned".to_string()),
        views: int_field(payload, "views"),
        likes: int_field(payload, "likes"),
        comments: int_field(payload, "comments"),
        collects: int_field(payload, "collects"),
        linked_topic_id: str_field(payload, "linkedTopicId"),
        notes: str_field(payload, "notes"),
        created_at: now,
        updated_at: now,
    };
    with_store_mut(state, |store| {
        store.publish_log.push(record.clone());
        let value =
            serde_json::to_value(&record).map_err(|error| error.to_string())?;
        Ok(json!({ "success": true, "item": value }))
    })
}

fn publish_update(state: &State<'_, AppState>, payload: &Value) -> Result<Value, String> {
    let id = str_field(payload, "id").ok_or_else(|| "缺少 id".to_string())?;
    let now = now_i64();
    with_store_mut(state, |store| {
        let Some(item) = store.publish_log.iter_mut().find(|record| record.id == id) else {
            return Err("发布记录不存在".to_string());
        };
        if let Some(value) = str_field(payload, "platform") {
            item.platform = value;
        }
        if let Some(value) = str_field(payload, "postUrl") {
            item.post_url = Some(value);
        }
        if let Some(value) = str_field(payload, "title") {
            item.title = value;
        }
        if let Some(value) = str_field(payload, "publishedAt") {
            item.published_at = Some(value);
        }
        if let Some(value) = str_field(payload, "status") {
            item.status = value;
        }
        if let Some(value) = payload.get("views").and_then(Value::as_i64) {
            item.views = value;
        }
        if let Some(value) = payload.get("likes").and_then(Value::as_i64) {
            item.likes = value;
        }
        if let Some(value) = payload.get("comments").and_then(Value::as_i64) {
            item.comments = value;
        }
        if let Some(value) = payload.get("collects").and_then(Value::as_i64) {
            item.collects = value;
        }
        if let Some(value) = str_field(payload, "linkedTopicId") {
            item.linked_topic_id = Some(value);
        }
        if let Some(value) = str_field(payload, "notes") {
            item.notes = Some(value);
        }
        item.updated_at = now;
        let value =
            serde_json::to_value(item).map_err(|error| error.to_string())?;
        Ok(json!({ "success": true, "item": value }))
    })
}

fn publish_remove(state: &State<'_, AppState>, payload: &Value) -> Result<Value, String> {
    let id = str_field(payload, "id").ok_or_else(|| "缺少 id".to_string())?;
    with_store_mut(state, |store| {
        let before = store.publish_log.len();
        store.publish_log.retain(|record| record.id != id);
        let removed = before - store.publish_log.len();
        Ok(json!({ "success": true, "removed": removed }))
    })
}

fn platforms() -> Value {
    json!([
        { "key": Platform::Xiaohongshu.as_str(), "label": "小红书", "consoleUrl": "https://creator.xiaohongshu.com/" },
        { "key": Platform::Douyin.as_str(), "label": "抖音", "consoleUrl": "https://creator.douyin.com/" },
        { "key": Platform::Bilibili.as_str(), "label": "哔哩哔哩", "consoleUrl": "https://member.bilibili.com/" },
        { "key": "wechat_public", "label": "微信公众号", "consoleUrl": "https://mp.weixin.qq.com/" },
        { "key": Platform::Weibo.as_str(), "label": "微博", "consoleUrl": "https://weibo.com/" },
        { "key": "other", "label": "其他", "consoleUrl": "" }
    ])
}
