use crate::agent::{
    execute_prepared_session_agent_turn, AssistantDaemonTurn, PreparedSessionAgentTurn,
};
use crate::knowledge;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::process::Stdio;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{channel as mpsc_channel, RecvTimeoutError, Sender},
    Arc, Mutex, OnceLock,
};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Listener, Manager, State};

use crate::store::assistant as assistant_store;
use crate::{
    background_command, now_iso, payload_string, run_curl_json, url_encode_component, with_store,
    AppState, AssistantSidecarRuntime, AssistantStateRecord,
};

pub(crate) fn value_to_i64_string(value: Option<&Value>) -> Option<String> {
    value.and_then(|item| {
        item.as_i64()
            .map(|number| number.to_string())
            .or_else(|| item.as_str().map(ToString::to_string))
    })
}

fn normalize_endpoint_path(path: Option<&str>, fallback: &str) -> String {
    let candidate = path
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback);
    let with_leading_slash = if candidate.starts_with('/') {
        candidate.to_string()
    } else {
        format!("/{candidate}")
    };
    if with_leading_slash.len() > 1 {
        with_leading_slash.trim_end_matches('/').to_string()
    } else {
        with_leading_slash
    }
}

fn assistant_base_url(state: &AssistantStateRecord) -> String {
    let host = state.host.trim();
    if host.is_empty() || state.port <= 0 {
        return String::new();
    }
    format!("http://{}:{}", host, state.port)
}

fn assistant_channel_public_value(channel: &Value, base_url: &str, fallback_path: &str) -> Value {
    let mut value = channel.clone();
    let endpoint_path = normalize_endpoint_path(
        value.get("endpointPath").and_then(|item| item.as_str()),
        fallback_path,
    );
    if let Some(object) = value.as_object_mut() {
        object.insert("endpointPath".to_string(), json!(endpoint_path.clone()));
        object.insert(
            "webhookUrl".to_string(),
            json!(if base_url.is_empty() {
                String::new()
            } else {
                format!("{base_url}{endpoint_path}")
            }),
        );
    }
    value
}

fn knowledge_api_public_value(channel: &Value, base_url: &str, fallback_path: &str) -> Value {
    let endpoint_path = normalize_endpoint_path(
        channel.get("endpointPath").and_then(|item| item.as_str()),
        fallback_path,
    );
    json!({
        "endpointPath": endpoint_path,
        "webhookUrl": if base_url.is_empty() {
            String::new()
        } else {
            format!("{base_url}{endpoint_path}")
        }
    })
}

fn normalize_request_path(path: &str) -> String {
    let without_query = path.split_once('?').map(|(head, _)| head).unwrap_or(path);
    let clean = without_query
        .split_once('#')
        .map(|(head, _)| head)
        .unwrap_or(without_query);
    normalize_endpoint_path(Some(clean), "/")
}

fn assistant_route_kind_for_path(path: &str, state: &AssistantStateRecord) -> &'static str {
    let normalized = normalize_request_path(path);
    let feishu_path = normalize_endpoint_path(
        state
            .feishu
            .get("endpointPath")
            .and_then(|item| item.as_str()),
        "/hooks/feishu/events",
    );
    if state
        .feishu
        .get("enabled")
        .and_then(|item| item.as_bool())
        .unwrap_or(false)
        && normalized == feishu_path
    {
        return "feishu";
    }
    let weixin_path = normalize_endpoint_path(
        state
            .weixin
            .get("endpointPath")
            .and_then(|item| item.as_str()),
        "/hooks/weixin/relay",
    );
    if state
        .weixin
        .get("enabled")
        .and_then(|item| item.as_bool())
        .unwrap_or(false)
        && normalized == weixin_path
    {
        return "weixin";
    }
    let relay_path = normalize_endpoint_path(
        state
            .relay
            .get("endpointPath")
            .and_then(|item| item.as_str()),
        "/hooks/channel/relay",
    );
    if state
        .relay
        .get("enabled")
        .and_then(|item| item.as_bool())
        .unwrap_or(true)
        && normalized == relay_path
    {
        return "relay";
    }
    "generic"
}

fn knowledge_api_endpoint_path(state: &AssistantStateRecord) -> String {
    let _ = state;
    "/api/knowledge".to_string()
}

fn is_knowledge_api_path(path: &str, state: &AssistantStateRecord) -> bool {
    let normalized = normalize_request_path(path);
    let base_path = knowledge_api_endpoint_path(state);
    normalized == base_path || normalized.starts_with(&format!("{base_path}/"))
}

fn is_accounts_api_path(path: &str) -> bool {
    let normalized = normalize_request_path(path);
    normalized == "/api/accounts" || normalized.starts_with("/api/accounts/")
}

fn is_browser_ipc_path(path: &str) -> bool {
    let normalized = normalize_request_path(path);
    normalized == "/api/ipc" || normalized.starts_with("/api/ipc/")
}

fn extract_bearer_or_token(headers: &HashMap<String, String>) -> String {
    let auth = headers
        .get("authorization")
        .or_else(|| headers.get("x-auth-token"))
        .cloned()
        .unwrap_or_default();
    auth.strip_prefix("Bearer ")
        .unwrap_or(&auth)
        .trim()
        .to_string()
}

fn extract_json_text(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return payload_string(&value, "text")
            .or_else(|| payload_string(&value, "content"))
            .or_else(|| value.as_str().map(ToString::to_string))
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty());
    }
    Some(trimmed.to_string())
}

fn extract_feishu_prompt(parsed: &Value) -> Option<String> {
    parsed
        .pointer("/event/text")
        .and_then(|value| value.as_str())
        .and_then(extract_json_text)
        .or_else(|| {
            parsed
                .pointer("/event/message/content")
                .and_then(|value| value.as_str())
                .and_then(extract_json_text)
        })
        .or_else(|| {
            parsed
                .get("text")
                .and_then(|value| value.as_str())
                .and_then(extract_json_text)
        })
}

fn resolve_feishu_receive_target(
    body: &Value,
    prefer_chat_id: bool,
) -> Option<(&'static str, String)> {
    let chat_id = body
        .pointer("/event/message/chat_id")
        .and_then(|value| value.as_str())
        .map(ToString::to_string)
        .filter(|value| !value.trim().is_empty());
    let open_id = body
        .pointer("/event/sender/sender_id/open_id")
        .and_then(|value| value.as_str())
        .map(ToString::to_string)
        .filter(|value| !value.trim().is_empty());
    let user_id = body
        .pointer("/event/sender/sender_id/user_id")
        .and_then(|value| value.as_str())
        .map(ToString::to_string)
        .filter(|value| !value.trim().is_empty());

    if prefer_chat_id {
        chat_id
            .map(|value| ("chat_id", value))
            .or_else(|| open_id.map(|value| ("open_id", value)))
            .or_else(|| user_id.map(|value| ("user_id", value)))
    } else {
        open_id
            .map(|value| ("open_id", value))
            .or_else(|| user_id.map(|value| ("user_id", value)))
            .or_else(|| chat_id.map(|value| ("chat_id", value)))
    }
}

fn fetch_feishu_tenant_access_token(app_id: &str, app_secret: &str) -> Result<String, String> {
    let response = run_curl_json(
        "POST",
        "https://open.feishu.cn/open-apis/auth/v3/tenant_access_token/internal",
        None,
        &[],
        Some(json!({
            "app_id": app_id,
            "app_secret": app_secret
        })),
    )?;
    if response
        .get("code")
        .and_then(|value| value.as_i64())
        .unwrap_or(0)
        != 0
    {
        let code = response.get("code").cloned().unwrap_or(Value::Null);
        let message = response
            .get("msg")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown error");
        return Err(format!("Feishu token error {code}: {message}"));
    }
    response
        .get("tenant_access_token")
        .and_then(|value| value.as_str())
        .map(ToString::to_string)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "Feishu token missing tenant_access_token".to_string())
}

fn send_feishu_text_reply(
    assistant_state: &AssistantStateRecord,
    body: &Value,
    reply: &str,
) -> Result<Value, String> {
    let app_id = assistant_state
        .feishu
        .get("appId")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Feishu appId 未配置，无法回消息".to_string())?;
    let app_secret = assistant_state
        .feishu
        .get("appSecret")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Feishu appSecret 未配置，无法回消息".to_string())?;
    let prefer_chat_id = assistant_state
        .feishu
        .get("replyUsingChatId")
        .and_then(|value| value.as_bool())
        .unwrap_or(true);
    let (receive_id_type, receive_id) = resolve_feishu_receive_target(body, prefer_chat_id)
        .ok_or_else(|| "Feishu 事件里缺少可回复的 receive_id".to_string())?;
    let tenant_access_token = fetch_feishu_tenant_access_token(app_id, app_secret)?;
    let response = run_curl_json(
        "POST",
        &format!(
            "https://open.feishu.cn/open-apis/im/v1/messages?receive_id_type={}",
            url_encode_component(receive_id_type)
        ),
        Some(tenant_access_token.as_str()),
        &[],
        Some(json!({
            "receive_id": receive_id,
            "msg_type": "text",
            "content": serde_json::to_string(&json!({ "text": reply }))
                .map_err(|error| error.to_string())?,
        })),
    )?;
    if response
        .get("code")
        .and_then(|value| value.as_i64())
        .unwrap_or(0)
        != 0
    {
        let code = response.get("code").cloned().unwrap_or(Value::Null);
        let message = response
            .get("msg")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown error");
        return Err(format!("Feishu send message error {code}: {message}"));
    }
    Ok(json!({
        "receiveIdType": receive_id_type,
        "receiveId": receive_id,
        "messageId": response.pointer("/data/message_id").cloned().unwrap_or(Value::Null)
    }))
}

pub(crate) fn assistant_state_value(state: &AssistantStateRecord) -> Value {
    let base_url = assistant_base_url(state);
    json!({
        "enabled": state.enabled,
        "autoStart": state.auto_start,
        "keepAliveWhenNoWindow": state.keep_alive_when_no_window,
        "host": state.host,
        "port": state.port,
        "listening": state.listening,
        "lockState": state.lock_state,
        "blockedBy": state.blocked_by,
        "lastError": state.last_error,
        "activeTaskCount": state.active_task_count,
        "queuedPeerCount": state.queued_peer_count,
        "inFlightKeys": state.in_flight_keys,
        "feishu": assistant_channel_public_value(&state.feishu, &base_url, "/hooks/feishu/events"),
        "relay": assistant_channel_public_value(&state.relay, &base_url, "/hooks/channel/relay"),
        "weixin": assistant_channel_public_value(&state.weixin, &base_url, "/hooks/weixin/relay"),
        "knowledgeApi": knowledge_api_public_value(&state.knowledge_api, &base_url, "/api/knowledge"),
    })
}

pub(crate) fn emit_assistant_log(app: &AppHandle, line: &str) {
    let _ = app.emit(
        "assistant:daemon-log",
        json!({
            "at": now_iso(),
            "level": "info",
            "message": line,
        }),
    );
}

pub(crate) fn emit_assistant_status(app: &AppHandle, state: &AssistantStateRecord) {
    let mut value = assistant_state_value(state);
    if let Ok(acp_value) = with_store(&app.state::<AppState>(), |store| {
        Ok(crate::acp_gateway_public_value(&store))
    }) {
        if let Some(object) = value.as_object_mut() {
            object.insert("acpGateway".to_string(), acp_value);
        }
    }
    let _ = app.emit("assistant:daemon-status", value);
}

pub(crate) fn http_json_response(
    stream: &mut TcpStream,
    status_code: u16,
    status_text: &str,
    body: Value,
) -> Result<(), String> {
    let payload = serde_json::to_string(&body).map_err(|error| error.to_string())?;
    let response = http_response_bytes(
        status_code,
        status_text,
        &[
            ("Content-Type", "application/json; charset=utf-8"),
            ("Access-Control-Allow-Origin", "*"),
            ("Access-Control-Allow-Methods", "GET, POST, OPTIONS"),
            (
                "Access-Control-Allow-Headers",
                "Content-Type, Authorization, X-Auth-Token",
            ),
            ("Access-Control-Max-Age", "600"),
            ("Access-Control-Allow-Private-Network", "true"),
        ],
        Some(payload.as_bytes()),
    );
    stream
        .write_all(&response)
        .map_err(|error| error.to_string())
}

pub(crate) fn http_empty_response(
    stream: &mut TcpStream,
    status_code: u16,
    status_text: &str,
) -> Result<(), String> {
    let response = http_response_bytes(
        status_code,
        status_text,
        &[
            ("Access-Control-Allow-Origin", "*"),
            ("Access-Control-Allow-Methods", "GET, POST, OPTIONS"),
            (
                "Access-Control-Allow-Headers",
                "Content-Type, Authorization, X-Auth-Token",
            ),
            ("Access-Control-Max-Age", "600"),
            ("Access-Control-Allow-Private-Network", "true"),
        ],
        None,
    );
    stream
        .write_all(&response)
        .map_err(|error| error.to_string())
}

pub(crate) fn http_ok_json(stream: &mut TcpStream, body: Value) -> Result<(), String> {
    http_json_response(stream, 200, "OK", body)
}

fn http_response_bytes(
    status_code: u16,
    status_text: &str,
    headers: &[(&str, &str)],
    body: Option<&[u8]>,
) -> Vec<u8> {
    let body = body.unwrap_or(&[]);
    let mut response = format!("HTTP/1.1 {} {}\r\n", status_code, status_text).into_bytes();
    for (key, value) in headers {
        response.extend_from_slice(format!("{key}: {value}\r\n").as_bytes());
    }
    response.extend_from_slice(format!("Content-Length: {}\r\n", body.len()).as_bytes());
    response.extend_from_slice(b"Connection: close\r\n\r\n");
    response.extend_from_slice(body);
    response
}

pub(crate) fn parse_http_request_parts(raw: &str) -> (String, String) {
    let normalized = raw.replace("\r\n", "\n");
    let mut parts = normalized.splitn(2, "\n\n");
    let headers = parts.next().unwrap_or_default().to_string();
    let body = parts.next().unwrap_or_default().to_string();
    (headers, body)
}

pub(crate) fn parse_http_request_meta(
    raw_headers: &str,
) -> (String, String, HashMap<String, String>) {
    let mut lines = raw_headers.lines();
    let request_line = lines.next().unwrap_or_default();
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts.next().unwrap_or("GET").to_string();
    let path = request_parts.next().unwrap_or("/").to_string();
    let mut headers = HashMap::new();
    for line in lines {
        if let Some((key, value)) = line.split_once(':') {
            headers.insert(key.trim().to_lowercase(), value.trim().to_string());
        }
    }
    (method, path, headers)
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
        .or_else(|| {
            buffer
                .windows(2)
                .position(|window| window == b"\n\n")
                .map(|index| index + 2)
        })
}

fn read_http_request(stream: &mut TcpStream, max_body_bytes: usize) -> Result<String, String> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    let mut header_end = None;
    let mut content_length = 0_usize;
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => {
                if let Some(end) = header_end {
                    let body_bytes = buffer.len().saturating_sub(end);
                    if body_bytes < content_length {
                        return Err(format!(
                            "HTTP request body 不完整: expected {} bytes, got {}",
                            content_length, body_bytes
                        ));
                    }
                }
                break;
            }
            Ok(bytes_read) => {
                buffer.extend_from_slice(&chunk[..bytes_read]);
                if header_end.is_none() {
                    header_end = find_header_end(&buffer);
                    if let Some(end) = header_end {
                        let raw_headers = String::from_utf8_lossy(&buffer[..end]).to_string();
                        let (_, _, headers) = parse_http_request_meta(raw_headers.trim_end());
                        content_length = headers
                            .get("content-length")
                            .and_then(|value| value.parse::<usize>().ok())
                            .unwrap_or(0);
                        if content_length > max_body_bytes {
                            return Err(format!(
                                "HTTP body 超过限制: {} > {}",
                                content_length, max_body_bytes
                            ));
                        }
                    }
                }
                if let Some(end) = header_end {
                    if buffer.len() >= end + content_length {
                        break;
                    }
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                if header_end.is_some() {
                    let end = header_end.unwrap_or(0);
                    let body_bytes = buffer.len().saturating_sub(end);
                    if body_bytes >= content_length {
                        break;
                    }
                    return Err(format!(
                        "HTTP request body 读取超时: expected {} bytes, got {}",
                        content_length, body_bytes
                    ));
                }
                return Err("HTTP request 读取超时".to_string());
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    String::from_utf8(buffer).map_err(|error| error.to_string())
}

pub(crate) fn assistant_session_id_for_route(route_kind: &str) -> String {
    format!(
        "assistant-session:{}",
        crate::slug_from_relative_path(route_kind)
    )
}

pub(crate) fn extract_assistant_prompt(
    route_kind: &str,
    body: &str,
) -> Result<Option<String>, String> {
    let parsed = serde_json::from_str::<Value>(body).unwrap_or_else(|_| json!({}));

    if let Some(challenge) = parsed.get("challenge").and_then(|value| value.as_str()) {
        return Ok(Some(challenge.to_string()));
    }

    let text = match route_kind {
        "feishu" => extract_feishu_prompt(&parsed),
        "weixin" => parsed
            .get("text")
            .and_then(|value| value.as_str())
            .or_else(|| parsed.get("content").and_then(|value| value.as_str()))
            .or_else(|| parsed.get("message").and_then(|value| value.as_str()))
            .map(ToString::to_string),
        "relay" => parsed
            .get("text")
            .and_then(|value| value.as_str())
            .or_else(|| parsed.get("message").and_then(|value| value.as_str()))
            .or_else(|| parsed.get("prompt").and_then(|value| value.as_str()))
            .map(ToString::to_string),
        _ => parsed
            .get("text")
            .and_then(|value| value.as_str())
            .or_else(|| parsed.get("message").and_then(|value| value.as_str()))
            .map(ToString::to_string),
    };

    Ok(text
        .map(|value: String| value.trim().to_string())
        .filter(|value: &String| !value.is_empty()))
}

pub(crate) fn validate_assistant_request(
    route_kind: &str,
    headers: &HashMap<String, String>,
    body: &Value,
    assistant_state: &AssistantStateRecord,
) -> Result<Option<Value>, String> {
    match route_kind {
        "feishu" => {
            if body.get("encrypt").is_some() {
                return Err(
                    "Feishu 加密事件当前未实现解密，请先关闭 encrypt 或改走明文校验".to_string(),
                );
            }
            if let Some(expected) = assistant_state
                .feishu
                .get("verificationToken")
                .and_then(|value| value.as_str())
                .filter(|value| !value.trim().is_empty())
            {
                let provided = body
                    .get("token")
                    .and_then(|value| value.as_str())
                    .unwrap_or("");
                if provided != expected {
                    return Err("Feishu verification token mismatch".to_string());
                }
            }
            if body.get("type").and_then(|value| value.as_str()) == Some("url_verification") {
                let challenge = body
                    .get("challenge")
                    .and_then(|value| value.as_str())
                    .unwrap_or("");
                return Ok(Some(json!({ "challenge": challenge })));
            }
        }
        "relay" => {
            if let Some(expected) = assistant_state
                .relay
                .get("authToken")
                .and_then(|value| value.as_str())
                .filter(|value| !value.trim().is_empty())
            {
                if extract_bearer_or_token(headers) != expected {
                    return Err("Relay auth token mismatch".to_string());
                }
            }
        }
        "weixin" => {
            if let Some(expected) = assistant_state
                .weixin
                .get("authToken")
                .and_then(|value| value.as_str())
                .filter(|value| !value.trim().is_empty())
            {
                if extract_bearer_or_token(headers) != expected {
                    return Err("Weixin auth token mismatch".to_string());
                }
            }
        }
        _ => {}
    }
    Ok(None)
}

fn handle_knowledge_http_request(
    app: &AppHandle,
    assistant_state: &AssistantStateRecord,
    method: &str,
    path: &str,
    body: &str,
) -> Result<(u16, &'static str, Value), String> {
    let base_path = knowledge_api_endpoint_path(assistant_state);
    let normalized_path = normalize_request_path(path);
    let subpath = normalized_path
        .strip_prefix(&base_path)
        .unwrap_or("")
        .trim_start_matches('/');
    let state = app.state::<AppState>();

    match (method, subpath) {
        ("OPTIONS", _) => Ok((204, "No Content", json!({}))),
        ("GET", "") | ("GET", "health") => Ok((
            200,
            "OK",
            knowledge::knowledge_http_health(
                &state,
                knowledge::knowledge_http_body_limit(),
                knowledge::knowledge_http_batch_limit(),
            )?,
        )),
        ("POST", "entries") => {
            let request: knowledge::KnowledgeEntryIngestRequest = serde_json::from_str(body)
                .map_err(|error| format!("knowledge entry request 无法解析: {error}"))?;
            let response = knowledge::ingest_entry(Some(app), &state, &request)?;
            Ok((200, "OK", response))
        }
        ("POST", "xhs/v2/entries") => {
            let request: knowledge::XhsKnowledgeEntryImportV2Request =
                match serde_json::from_str(body) {
                    Ok(request) => request,
                    Err(error) => {
                        return Err(format!("xhs knowledge import v2 request 无法解析: {error}"));
                    }
                };
            let response = knowledge::ingest_xhs_entry_v2(Some(app), &state, &request)?;
            Ok((200, "OK", response))
        }
        ("POST", "zhihu/answers") => {
            let request: knowledge::ZhihuAnswerIngestRequest = serde_json::from_str(body)
                .map_err(|error| format!("zhihu answer request 无法解析: {error}"))?;
            let response = knowledge::ingest_zhihu_answer(Some(app), &state, &request)?;
            Ok((200, "OK", response))
        }
        ("POST", "zhihu/articles") => {
            let request: knowledge::ZhihuArticleIngestRequest = serde_json::from_str(body)
                .map_err(|error| format!("zhihu article request 无法解析: {error}"))?;
            let response = knowledge::ingest_zhihu_article(Some(app), &state, &request)?;
            Ok((200, "OK", response))
        }
        ("POST", "document-sources") => {
            let request: knowledge::KnowledgeDocumentSourceIngestRequest =
                serde_json::from_str(body).map_err(|error| {
                    format!("knowledge document source request 无法解析: {error}")
                })?;
            let response = knowledge::ingest_document_source(Some(app), &state, &request)?;
            Ok((200, "OK", response))
        }
        ("POST", "media-assets") => {
            let request: knowledge::KnowledgeMediaAssetIngestRequest =
                serde_json::from_str(body)
                    .map_err(|error| format!("knowledge media assets request 无法解析: {error}"))?;
            let response = knowledge::ingest_media_assets(Some(app), &state, &request)?;
            Ok((200, "OK", response))
        }
        ("POST", "batch-ingest") => {
            let request: knowledge::KnowledgeBatchIngestRequest = serde_json::from_str(body)
                .map_err(|error| format!("knowledge batch request 无法解析: {error}"))?;
            let response = knowledge::batch_ingest(Some(app), &state, &request)?;
            Ok((200, "OK", response))
        }
        _ => Ok((
            404,
            "Not Found",
            json!({
                "success": false,
                "error": "Knowledge API route not found",
                "path": normalized_path,
                "basePath": base_path,
            }),
        )),
    }
}

fn handle_accounts_http_request(
    app: &AppHandle,
    method: &str,
    path: &str,
    body: &str,
) -> Result<(u16, &'static str, Value), String> {
    let state = app.state::<AppState>();
    crate::accounts::handle_accounts_http_request(&state, method, path, body)
}

fn handle_browser_ipc_http_request(
    app: &AppHandle,
    method: &str,
    path: &str,
    headers: &HashMap<String, String>,
    body: &str,
) -> Result<(u16, &'static str, Value), String> {
    let normalized_path = normalize_request_path(path);
    if method == "OPTIONS" {
        return Ok((204, "No Content", json!({})));
    }
    if method == "GET" && normalized_path == "/api/ipc/health" {
        return Ok((200, "OK", json!({ "success": true })));
    }
    if method != "POST"
        || (normalized_path != "/api/ipc/invoke" && normalized_path != "/api/ipc/send")
    {
        return Ok((
            404,
            "Not Found",
            json!({
                "success": false,
                "error": "Browser IPC route not found",
                "path": normalized_path,
            }),
        ));
    }
    if !is_allowed_browser_ipc_origin(headers) {
        return Ok((
            403,
            "Forbidden",
            json!({
                "success": false,
                "error": "Browser IPC only accepts same-device localhost pages",
            }),
        ));
    }

    let request = serde_json::from_str::<Value>(body)
        .map_err(|error| format!("browser IPC request 无法解析: {error}"))?;
    let channel = request
        .get("channel")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "browser IPC request 缺少 channel".to_string())?;
    let payload = request.get("payload").cloned().unwrap_or(Value::Null);
    let state = app.state::<AppState>();

    // 请求级身份：web 网关会注入 X-Beav-User；桌面端没有该头 → 管理员（可见全部）。
    let request_owner = headers
        .get("x-beav-user")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    crate::identity::set_request_owner(request_owner);

    let outcome = if normalized_path == "/api/ipc/send" {
        crate::commands::chat::handle_send_channel(app, channel, payload, &state)
            .map(|_| json!({ "success": true }))
    } else {
        crate::handle_channel(app, channel, payload, &state)
    };
    crate::identity::clear_request_owner();
    let value = outcome?;
    Ok((200, "OK", value))
}

fn is_allowed_browser_ipc_origin(headers: &HashMap<String, String>) -> bool {
    let candidate = headers
        .get("origin")
        .or_else(|| headers.get("referer"))
        .map(String::as_str)
        .unwrap_or("")
        .trim();
    if candidate.is_empty() || candidate.eq_ignore_ascii_case("null") {
        return true;
    }
    let Ok(parsed) = url::Url::parse(candidate) else {
        return false;
    };
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return false;
    }
    matches!(
        parsed
            .host_str()
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str(),
        "localhost" | "127.0.0.1" | "::1"
    )
}

pub(crate) fn execute_assistant_message(
    app: &AppHandle,
    route_kind: &str,
    headers: &HashMap<String, String>,
    body: &str,
) -> Result<Value, String> {
    let state = app.state::<AppState>();
    let assistant_snapshot = with_store(&state, |store| Ok(assistant_store::snapshot(&store)))?;
    let parsed_body = serde_json::from_str::<Value>(body).unwrap_or_else(|_| json!({}));
    if let Some(response) =
        validate_assistant_request(route_kind, headers, &parsed_body, &assistant_snapshot)?
    {
        return Ok(response);
    }
    let prompt = extract_assistant_prompt(route_kind, body)?;
    let Some(prompt) = prompt else {
        return Ok(json!({
            "success": true,
            "message": "No actionable text found in request body.",
            "routeKind": route_kind
        }));
    };

    let session_id = assistant_session_id_for_route(route_kind);
    let turn = PreparedSessionAgentTurn::assistant_daemon(AssistantDaemonTurn::new(
        route_kind,
        session_id.clone(),
        prompt.clone(),
    ));
    let execution = execute_prepared_session_agent_turn(Some(app), &state, &turn)?;
    let delivery = if route_kind == "feishu" {
        Some(send_feishu_text_reply(
            &assistant_snapshot,
            &parsed_body,
            execution.response(),
        )?)
    } else {
        None
    };
    emit_assistant_log(
        app,
        &format!("assistant daemon completed {} request", route_kind),
    );
    Ok(json!({
        "success": true,
        "routeKind": route_kind,
        "reply": execution.response(),
        "sessionId": execution.session_id(),
        "delivery": delivery
    }))
}

fn handle_assistant_connection(app: AppHandle, mut stream: TcpStream, addr: SocketAddr) {
    let request = match read_http_request(&mut stream, knowledge::knowledge_http_body_limit()) {
        Ok(request) => request,
        Err(error) => {
            emit_assistant_log(
                &app,
                &format!(
                    "assistant daemon request read failed from {}: {}",
                    addr, error
                ),
            );
            let _ = http_json_response(
                &mut stream,
                400,
                "Bad Request",
                json!({ "success": false, "error": error }),
            );
            return;
        }
    };
    let first_line = request.lines().next().unwrap_or_default().to_string();
    let path = first_line
        .split_whitespace()
        .nth(1)
        .unwrap_or("/")
        .to_string();
    emit_assistant_log(
        &app,
        &format!("assistant daemon request from {}: {}", addr, first_line),
    );
    let assistant_snapshot = with_store(&app.state::<AppState>(), |store| {
        Ok(assistant_store::snapshot(&store))
    })
    .unwrap_or_else(|_| AssistantStateRecord::default());
    let (raw_headers, body) = parse_http_request_parts(&request);
    let (method, _path, headers) = parse_http_request_meta(&raw_headers);
    if normalize_request_path(&path) == "/api/events" {
        stream_sse(&mut stream);
        return;
    }
    if normalize_request_path(&path) == "/api/file" {
        if method != "GET" {
            let _ = http_json_response(
                &mut stream,
                405,
                "Method Not Allowed",
                json!({ "success": false, "error": "method not allowed" }),
            );
            return;
        }
        if let Err(error) = serve_local_file(
            &app,
            &mut stream,
            &path,
            headers.get("range").map(String::as_str),
        ) {
            let _ = http_json_response(
                &mut stream,
                404,
                "Not Found",
                json!({ "success": false, "error": error }),
            );
        }
        return;
    }
    if crate::is_acp_gateway_path(&path) {
        emit_assistant_log(&app, "assistant daemon matched route kind: acp");
        let acp_local_only = with_store(&app.state::<AppState>(), |store| {
            Ok(store.acp_gateway.local_only)
        })
        .unwrap_or(true);
        let response = if acp_local_only && !addr.ip().is_loopback() {
            http_json_response(
                &mut stream,
                403,
                "Forbidden",
                json!({
                    "success": false,
                    "error": {
                        "code": "acp_local_only",
                        "message": "RedBox ACP gateway only accepts loopback clients."
                    }
                }),
            )
        } else {
            match crate::handle_acp_gateway_http_request(&app, &method, &path, &headers, &body) {
                Ok((status, status_text, body)) => {
                    if status == 204 {
                        http_empty_response(&mut stream, status, status_text)
                    } else {
                        http_json_response(&mut stream, status, status_text, body)
                    }
                }
                Err(error) => http_json_response(
                    &mut stream,
                    400,
                    "Bad Request",
                    json!({ "success": false, "error": error }),
                ),
            }
        };
        if let Err(error) = response {
            emit_assistant_log(
                &app,
                &format!("assistant daemon ACP response failed: {}", error),
            );
        }
        return;
    }
    if is_browser_ipc_path(&path) {
        let response = match handle_browser_ipc_http_request(&app, &method, &path, &headers, &body)
        {
            Ok((status, status_text, body)) => {
                if status == 204 {
                    http_empty_response(&mut stream, status, status_text)
                } else {
                    http_json_response(&mut stream, status, status_text, body)
                }
            }
            Err(error) => http_json_response(
                &mut stream,
                400,
                "Bad Request",
                json!({ "success": false, "error": error }),
            ),
        };
        if let Err(error) = response {
            emit_assistant_log(
                &app,
                &format!("assistant daemon browser IPC response failed: {}", error),
            );
        }
        return;
    }
    if is_accounts_api_path(&path) {
        emit_assistant_log(&app, "assistant daemon matched route kind: accounts-api");
        let response = match handle_accounts_http_request(&app, &method, &path, &body) {
            Ok((status, status_text, body)) => {
                if status == 204 {
                    http_empty_response(&mut stream, status, status_text)
                } else {
                    http_json_response(&mut stream, status, status_text, body)
                }
            }
            Err(error) => http_json_response(
                &mut stream,
                400,
                "Bad Request",
                json!({ "success": false, "error": error }),
            ),
        };
        if let Err(error) = response {
            emit_assistant_log(
                &app,
                &format!("assistant daemon accounts response failed: {}", error),
            );
        }
        return;
    }
    if is_knowledge_api_path(&path, &assistant_snapshot) {
        emit_assistant_log(&app, "assistant daemon matched route kind: knowledge-api");
        let response =
            match handle_knowledge_http_request(&app, &assistant_snapshot, &method, &path, &body) {
                Ok((status, status_text, body)) => {
                    if status == 204 {
                        http_empty_response(&mut stream, status, status_text)
                    } else {
                        http_json_response(&mut stream, status, status_text, body)
                    }
                }
                Err(error) => http_json_response(
                    &mut stream,
                    400,
                    "Bad Request",
                    json!({ "success": false, "error": error }),
                ),
            };
        if let Err(error) = response {
            emit_assistant_log(
                &app,
                &format!(
                    "assistant daemon failed to write knowledge response: {}",
                    error
                ),
            );
        }
        return;
    }
    let route_kind = assistant_route_kind_for_path(&path, &assistant_snapshot);
    emit_assistant_log(
        &app,
        &format!("assistant daemon matched route kind: {}", route_kind),
    );
    let result =
        execute_assistant_message(&app, route_kind, &headers, &body).unwrap_or_else(|error| {
            json!({
                "success": false,
                "routeKind": route_kind,
                "error": error
            })
        });
    let _ = http_ok_json(
        &mut stream,
        json!({
            "endpoint": first_line,
            "path": path,
            "routeKind": route_kind,
            "result": result
        }),
    );
}

// ==== Web 版实时事件（SSE 桥）==============================================
// 桌面端 `emit` 的事件，除了发给 WebView，也广播给通过 HTTP 连上来的浏览器。
// 只新增监听 + 新路由，桌面端原有行为不变。

fn sse_clients() -> &'static Mutex<Vec<Sender<String>>> {
    static CLIENTS: OnceLock<Mutex<Vec<Sender<String>>>> = OnceLock::new();
    CLIENTS.get_or_init(|| Mutex::new(Vec::new()))
}

fn broadcast_sse_event(event: &str, payload: &str) {
    let payload_value: Value = serde_json::from_str(payload).unwrap_or(Value::Null);
    let frame = format!(
        "data: {}\n\n",
        json!({ "event": event, "payload": payload_value })
    );
    let Ok(mut clients) = sse_clients().lock() else {
        return;
    };
    // 接收端已断开（send 失败）时顺手清理。
    clients.retain(|sender| sender.send(frame.clone()).is_ok());
}

/// 前端 `ipcRenderer.on(channel, …)` 会订阅的事件名，逐个注册并广播到 SSE。
const SSE_EVENT_NAMES: &[&str] = &[
    "advisors:changed",
    "advisors:download-progress",
    "app:startup-migration-status",
    "app:update-available",
    "app:update-install-progress",
    "archives:sample-created",
    "assistant:daemon-log",
    "assistant:daemon-status",
    "auth:data-changed",
    "auth:state-changed",
    "background:task-updated",
    "chat:session-title-updated",
    "data:changed",
    "diagnostics:report-pending",
    "generation:job-log",
    "generation:job-updated",
    "knowledge:catalog-updated",
    "knowledge:changed",
    "knowledge:docs-updated",
    "knowledge:file-index-updated",
    "knowledge:new-youtube-video",
    "knowledge:note-updated",
    "knowledge:youtube-video-updated",
    "llm-readiness:state-changed",
    "manuscripts:render-progress",
    "manuscripts:write-proposal",
    "plugins:changed",
    "redclaw:runner-status",
    "redclaw:task-event",
    "runtime:event",
    "settings:updated",
    "space:changed",
    "wander:progress",
    "wander:result",
    "youtube:fetch-info-progress",
];

/// 幂等：daemon 重启时不会重复注册。
pub(crate) fn install_sse_event_bridge(app: &AppHandle) {
    static INSTALLED: AtomicBool = AtomicBool::new(false);
    if INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }
    for name in SSE_EVENT_NAMES {
        let event_name = (*name).to_string();
        let _ = app.listen(event_name.clone(), move |event| {
            broadcast_sse_event(&event_name, event.payload());
        });
    }
    emit_assistant_log(app, "sse event bridge installed");
}

fn mime_for_path(path: &std::path::Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "avif" => "image/avif",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "m4a" => "audio/mp4",
        "json" => "application/json; charset=utf-8",
        "txt" | "srt" => "text/plain; charset=utf-8",
        "md" => "text/markdown; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// 解析 `Range: bytes=start-end`，返回闭区间（含首尾）。
///
/// 支持三种写法：`bytes=0-1023`、`bytes=1000-`（到末尾）、`bytes=-1000`（末尾 1000 字节）。
/// 多段 range 不处理（视频播放器不会用）；无效或不满足时返回 None，由调用方回 200 全量。
fn parse_byte_range(header: &str, total: u64) -> Option<(u64, u64)> {
    if total == 0 {
        return None;
    }
    let value = header.trim();
    let value = value.strip_prefix("bytes=")?;
    let first = value.split(',').next()?.trim();
    let (start_raw, end_raw) = first.split_once('-')?;
    let (start, end) = if start_raw.trim().is_empty() {
        let n: u64 = end_raw.trim().parse().ok()?;
        if n == 0 {
            return None;
        }
        (total.saturating_sub(n), total - 1)
    } else {
        let start: u64 = start_raw.trim().parse().ok()?;
        let end = if end_raw.trim().is_empty() {
            total - 1
        } else {
            end_raw.trim().parse::<u64>().ok()?.min(total - 1)
        };
        (start, end)
    };
    if start > end || start >= total {
        return None;
    }
    Some((start, end))
}

/// Web 版文件代理：`GET /api/file?path=<绝对路径>`，仅允许工作区 / 导出根内的文件。
/// 浏览器读不了 `file://`，Web 端图片/视频/字幕都靠它。
///
/// **必须支持 Range**：浏览器播视频是分段请求（`Range: bytes=...`），
/// 只会 200 全量返回的话，播放器只能播开头几秒就开始无限缓冲
/// （用户实测：视频播几秒一直加载中）。这里同时改成**流式发送**，
/// 不再把整个视频读进内存。
fn serve_local_file(
    app: &AppHandle,
    stream: &mut TcpStream,
    path: &str,
    range_header: Option<&str>,
) -> Result<(), String> {
    let query = path.split_once('?').map(|(_, value)| value).unwrap_or("");
    let mut raw_path = None;
    for pair in query.split('&') {
        if let Some((key, value)) = pair.split_once('=') {
            if key == "path" {
                raw_path = Some(value.to_string());
            }
        }
    }
    let raw_path = raw_path.ok_or_else(|| "缺少 path 参数".to_string())?;
    let decoded = urlencoding::decode(&raw_path)
        .map_err(|error| error.to_string())?
        .into_owned();
    let requested = std::path::PathBuf::from(decoded.trim());
    let canonical = requested
        .canonicalize()
        .map_err(|_| format!("文件不存在: {}", requested.display()))?;

    let state = app.state::<AppState>();
    let mut roots: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(guard) = state.workspace_root_cache.lock() {
        if let Ok(root) = guard.canonicalize() {
            roots.push(root);
        }
    }
    if let Ok(export_root) = with_store(&state, |store| Ok(store.monitor_export_root.clone())) {
        let export_root = export_root.trim();
        if !export_root.is_empty() {
            if let Ok(root) = std::path::PathBuf::from(export_root).canonicalize() {
                roots.push(root);
            }
        }
    }
    if !roots.iter().any(|root| canonical.starts_with(root)) {
        return Err("路径不在允许的目录内".to_string());
    }

    let metadata = std::fs::metadata(&canonical).map_err(|error| error.to_string())?;
    let total = metadata.len();
    let mime = mime_for_path(&canonical);
    let range = range_header.and_then(|value| parse_byte_range(value, total));

    let (status_line, content_length, content_range) = match range {
        Some((start, end)) => (
            "HTTP/1.1 206 Partial Content\r\n".to_string(),
            end - start + 1,
            Some(format!("Content-Range: bytes {start}-{end}/{total}\r\n")),
        ),
        None => (
            "HTTP/1.1 200 OK\r\n".to_string(),
            total,
            None,
        ),
    };

    let mut head = status_line;
    head.push_str(&format!("Content-Type: {mime}\r\n"));
    head.push_str(&format!("Content-Length: {content_length}\r\n"));
    // 让浏览器知道可以按段请求（视频拖动进度条依赖它）
    head.push_str("Accept-Ranges: bytes\r\n");
    if let Some(content_range) = content_range {
        head.push_str(&content_range);
    }
    head.push_str("Access-Control-Allow-Origin: *\r\n");
    head.push_str("Cache-Control: private, max-age=3600\r\n");
    head.push_str("Connection: close\r\n\r\n");
    stream
        .write_all(head.as_bytes())
        .map_err(|error| error.to_string())?;

    // 流式发送：视频可能上百 MB，不能再整进内存
    let mut file = std::fs::File::open(&canonical).map_err(|error| error.to_string())?;
    if let Some((start, end)) = range {
        use std::io::{Seek, SeekFrom};
        file.seek(SeekFrom::Start(start))
            .map_err(|error| error.to_string())?;
        let mut limited = std::io::Read::take(file, end - start + 1);
        std::io::copy(&mut limited, stream).map_err(|error| error.to_string())?;
    } else {
        std::io::copy(&mut file, stream).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn stream_sse(stream: &mut TcpStream) {
    const HEADERS: &str = concat!(
        "HTTP/1.1 200 OK\r\n",
        "Content-Type: text/event-stream\r\n",
        "Cache-Control: no-cache\r\n",
        "Connection: keep-alive\r\n",
        "Access-Control-Allow-Origin: *\r\n",
        "\r\n"
    );
    if stream.write_all(HEADERS.as_bytes()).is_err() {
        return;
    }
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    let (sender, receiver) = mpsc_channel::<String>();
    if sse_clients()
        .lock()
        .map(|mut clients| clients.push(sender))
        .is_err()
    {
        return;
    }
    let _ = stream.write_all(b": connected\n\n");
    let _ = stream.flush();
    loop {
        match receiver.recv_timeout(Duration::from_secs(15)) {
            Ok(frame) => {
                if stream.write_all(frame.as_bytes()).is_err() {
                    break;
                }
                let _ = stream.flush();
            }
            Err(RecvTimeoutError::Timeout) => {
                if stream.write_all(b": ping\n\n").is_err() {
                    break;
                }
                let _ = stream.flush();
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

pub(crate) fn run_assistant_listener(
    app: AppHandle,
    host: String,
    port: i64,
    stop: Arc<AtomicBool>,
) -> Result<JoinHandle<()>, String> {
    install_sse_event_bridge(&app);
    let listener =
        TcpListener::bind(format!("{}:{}", host, port)).map_err(|error| error.to_string())?;
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    emit_assistant_log(
        &app,
        &format!("assistant daemon listening on http://{}:{}", host, port),
    );
    if let Err(error) = crate::refresh_acp_discovery_file(&app, true) {
        emit_assistant_log(
            &app,
            &format!("assistant daemon ACP discovery write failed: {}", error),
        );
    }
    let join = thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((stream, addr)) => {
                    let app_for_request = app.clone();
                    thread::spawn(move || {
                        handle_assistant_connection(app_for_request, stream, addr);
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(200));
                }
                Err(error) => {
                    emit_assistant_log(
                        &app,
                        &format!("assistant daemon listener error: {}", error),
                    );
                    thread::sleep(Duration::from_millis(500));
                }
            }
        }
        if let Err(error) = crate::refresh_acp_discovery_file(&app, false) {
            emit_assistant_log(
                &app,
                &format!(
                    "assistant daemon ACP discovery stop update failed: {}",
                    error
                ),
            );
        }
        emit_assistant_log(&app, "assistant daemon stopped");
    });
    Ok(join)
}

pub(crate) fn spawn_weixin_sidecar(
    weixin: &Value,
) -> Result<Option<AssistantSidecarRuntime>, String> {
    let enabled = weixin
        .get("enabled")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let auto_start = weixin
        .get("autoStartSidecar")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let command = weixin
        .get("sidecarCommand")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if !enabled || !auto_start || command.is_none() {
        return Ok(None);
    }
    let command = command.unwrap();
    let args = weixin
        .get("sidecarArgs")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(ToString::to_string))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut child_command = background_command(command);
    child_command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(cwd) = weixin
        .get("sidecarCwd")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        child_command.current_dir(cwd);
    }
    if let Some(env) = weixin.get("sidecarEnv").and_then(|value| value.as_object()) {
        for (key, value) in env {
            if let Some(value) = value.as_str() {
                child_command.env(key, value);
            }
        }
    }
    let child = child_command.spawn().map_err(|error| error.to_string())?;
    let pid = child.id();
    Ok(Some(AssistantSidecarRuntime { child, pid }))
}

pub(crate) fn stop_assistant_sidecar(state: &State<'_, AppState>) -> Result<Option<u32>, String> {
    let mut guard = state
        .assistant_sidecar
        .lock()
        .map_err(|_| "assistant sidecar lock 已损坏".to_string())?;
    if let Some(mut runtime) = guard.take() {
        let pid = runtime.pid;
        let _ = runtime.child.kill();
        let _ = runtime.child.wait();
        return Ok(Some(pid));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_assistant_state() -> AssistantStateRecord {
        AssistantStateRecord {
            enabled: true,
            auto_start: true,
            keep_alive_when_no_window: true,
            host: "127.0.0.1".to_string(),
            port: 31937,
            listening: true,
            lock_state: "owner".to_string(),
            blocked_by: None,
            last_error: None,
            active_task_count: 0,
            queued_peer_count: 0,
            in_flight_keys: Vec::new(),
            feishu: json!({
                "enabled": true,
                "receiveMode": "webhook",
                "endpointPath": "/custom/feishu",
                "replyUsingChatId": true,
                "webhookUrl": "",
                "websocketRunning": false
            }),
            relay: json!({
                "enabled": true,
                "endpointPath": "hooks/channel/custom-relay",
                "authToken": "",
                "webhookUrl": ""
            }),
            weixin: json!({
                "enabled": true,
                "endpointPath": "/hooks/weixin/custom",
                "authToken": "",
                "accountId": "",
                "autoStartSidecar": false,
                "cursorFile": "",
                "sidecarCommand": "",
                "sidecarArgs": [],
                "sidecarCwd": "",
                "sidecarEnv": {},
                "webhookUrl": "",
                "sidecarRunning": false,
                "connected": false,
                "stateDir": "",
                "availableAccountIds": []
            }),
            knowledge_api: json!({
                "endpointPath": "/api/knowledge/custom",
                "webhookUrl": ""
            }),
        }
    }

    #[test]
    fn assistant_state_value_populates_webhook_urls_from_runtime_host() {
        let value = assistant_state_value(&sample_assistant_state());
        assert_eq!(
            value
                .pointer("/feishu/webhookUrl")
                .and_then(|item| item.as_str()),
            Some("http://127.0.0.1:31937/custom/feishu")
        );
        assert_eq!(
            value
                .pointer("/relay/webhookUrl")
                .and_then(|item| item.as_str()),
            Some("http://127.0.0.1:31937/hooks/channel/custom-relay")
        );
        assert_eq!(
            value
                .pointer("/weixin/webhookUrl")
                .and_then(|item| item.as_str()),
            Some("http://127.0.0.1:31937/hooks/weixin/custom")
        );
        assert_eq!(
            value
                .pointer("/knowledgeApi/webhookUrl")
                .and_then(|item| item.as_str()),
            Some("http://127.0.0.1:31937/api/knowledge")
        );
    }

    #[test]
    fn assistant_route_kind_for_path_uses_configured_endpoint_paths() {
        let state = sample_assistant_state();
        assert_eq!(
            assistant_route_kind_for_path("/custom/feishu?source=test", &state),
            "feishu"
        );
        assert_eq!(
            assistant_route_kind_for_path("/hooks/weixin/custom", &state),
            "weixin"
        );
        assert_eq!(
            assistant_route_kind_for_path("/hooks/channel/custom-relay", &state),
            "relay"
        );
        assert_eq!(
            assistant_route_kind_for_path("/hooks/feishu/events", &state),
            "generic"
        );
        assert!(is_knowledge_api_path("/api/knowledge/entries", &state));
        assert!(!is_knowledge_api_path(
            "/api/knowledge/custom/entries",
            &state
        ));
    }

    #[test]
    fn browser_ipc_origin_allows_localhost_only() {
        let mut headers = HashMap::new();
        assert!(is_allowed_browser_ipc_origin(&headers));

        headers.insert("origin".to_string(), "http://localhost:1420".to_string());
        assert!(is_allowed_browser_ipc_origin(&headers));

        headers.insert("origin".to_string(), "http://127.0.0.1:1420".to_string());
        assert!(is_allowed_browser_ipc_origin(&headers));

        headers.insert("origin".to_string(), "https://example.com".to_string());
        assert!(!is_allowed_browser_ipc_origin(&headers));
    }

    #[test]
    fn extract_assistant_prompt_reads_feishu_message_content_json() {
        let body = json!({
            "event": {
                "message": {
                    "content": "{\"text\":\"你好，助手\"}"
                }
            }
        });
        assert_eq!(
            extract_assistant_prompt("feishu", &body.to_string()).unwrap(),
            Some("你好，助手".to_string())
        );
    }

    #[test]
    fn validate_assistant_request_rejects_encrypted_feishu_events() {
        let headers = HashMap::new();
        let state = sample_assistant_state();
        let error = validate_assistant_request(
            "feishu",
            &headers,
            &json!({ "encrypt": "ciphertext" }),
            &state,
        )
        .unwrap_err();
        assert!(error.contains("Feishu 加密事件"));
    }

    #[test]
    fn parses_byte_range_variants() {
        // 闭区间
        assert_eq!(parse_byte_range("bytes=0-1023", 10000), Some((0, 1023)));
        // 开区间到末尾
        assert_eq!(parse_byte_range("bytes=9000-", 10000), Some((9000, 9999)));
        // 末尾 N 字节
        assert_eq!(parse_byte_range("bytes=-500", 10000), Some((9500, 9999)));
        // 超出末尾 → 截到末尾
        assert_eq!(parse_byte_range("bytes=9000-99999", 10000), Some((9000, 9999)));
        // 多段只取第一段
        assert_eq!(parse_byte_range("bytes=0-99,200-299", 10000), Some((0, 99)));
        // 无效 / 越界 → None（调用方回 200 全量）
        assert_eq!(parse_byte_range("bytes=10000-", 10000), None);
        assert_eq!(parse_byte_range("bytes=abc-def", 10000), None);
        assert_eq!(parse_byte_range("items=0-10", 10000), None);
        assert_eq!(parse_byte_range("bytes=0-10", 0), None);
    }

    #[test]
    fn http_response_bytes_includes_browser_preflight_headers() {
        let response = String::from_utf8(http_response_bytes(
            204,
            "No Content",
            &[
                ("Access-Control-Allow-Origin", "*"),
                ("Access-Control-Allow-Methods", "GET, POST, OPTIONS"),
                (
                    "Access-Control-Allow-Headers",
                    "Content-Type, Authorization, X-Auth-Token",
                ),
                ("Access-Control-Allow-Private-Network", "true"),
            ],
            None,
        ))
        .unwrap();
        assert!(response.contains("HTTP/1.1 204 No Content"));
        assert!(response.contains("Access-Control-Allow-Origin: *"));
        assert!(response.contains("Access-Control-Allow-Methods: GET, POST, OPTIONS"));
        assert!(response
            .contains("Access-Control-Allow-Headers: Content-Type, Authorization, X-Auth-Token"));
        assert!(response.contains("Access-Control-Allow-Private-Network: true"));
        assert!(response.contains("Content-Length: 0"));
    }
}
