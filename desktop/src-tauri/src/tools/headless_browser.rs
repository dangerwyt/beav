//! 最小可用的 Chrome DevTools Protocol (CDP) 客户端。
//!
//! 设计目标：让我们用用户本机 Chrome（通过 `--remote-debugging-port=PORT` 拉起）
//! 抓取需要 JS 渲染 + 登录态的平台（小红书 / 抖音等），不下载 Chromium。
//!
//! 仅实现以下能力，足以驱动监控抓取：
//! - 探测系统已装的 Chrome / Edge / Chromium 可执行文件
//! - 启动 Chrome 进程，分配独立 user-data-dir 持久化 cookie
//! - HTTP GET `http://localhost:<port>/json/version` 拿到 WebSocket URL
//! - 用 `tokio-tungstenite` 直连 CDP，执行 `Page.navigate` / `Runtime.evaluate`
//!
//! 后续若需要更多 CDP 域（Network/Performance/Cookies），按同一模式扩展。

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{mpsc, Mutex, OnceLock};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};
use tokio::net::TcpStream;
use tokio::sync::Mutex as AsyncMutex;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

const CDP_VERSION_PATH: &str = "/json/version";

/// 每个平台独立的 Chrome 会话：拥有自己的 user-data-dir（cookie 隔离）与调试端口。
/// 关键点：**不能用全局单例端口** —— 否则先开小红书再点"打开抖音"会被复用短路，
/// 导致抖音根本没拉起独立 Chrome、cookie 全混进同一个 profile。
/// 这里按 platform 维度各存一份端口 + 子进程句柄（句柄必须持有，否则 Windows 上
/// `Child::drop` 触发 `TerminateProcess` 把 Chrome 杀掉）。
struct ChromeSession {
    port: u16,
    child: Option<Child>,
}

static CHROME_SESSIONS: OnceLock<Mutex<HashMap<String, ChromeSession>>> = OnceLock::new();

/// 登录态探针标签（platform → targetId）。
///
/// 复用同一个标签页看登录态；每次都新建+关闭的话，服务器上会看到
/// "Chrome 反复跳出小红书主页然后关闭"（扫码页是 5 秒轮询一次），
/// 还会打断正在等待扫码的登录页。
static PROBE_TABS: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

/// 登录态缓存（platform → (cookie 指纹, 结果, 检测时刻)）。
///
/// cookie 没变 → 会话没变 → 直接用上次结果，**完全不碰浏览器**。
/// 但**"未登录"结果只缓存 15 秒**：扫码成功后 cookie 可能稍晚才写进来，
/// 若把"过早检测到的未登录"永久缓存，就会一直显示未登录（用户实测踩到过）。
static LOGIN_STATE_CACHE: OnceLock<Mutex<HashMap<String, (String, bool, std::time::Instant)>>> =
    OnceLock::new();

/// 「未登录」结果的缓存有效期
const NOT_LOGGED_IN_CACHE_TTL: Duration = Duration::from_secs(15);

/// 探测系统已装的 Chrome / Edge / Chromium 路径。
///
/// 各平台按自己的安装约定搜索；找不到返回 None，由调用方提示用户。
pub fn detect_chrome_path() -> Option<PathBuf> {
    let candidates: &[&str] = &[
        // Windows：Chrome stable / Chrome x86 / Edge（Win11 预装）/ Brave
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe",
        // macOS：应用包内部的可执行文件（Chrome / Edge / Brave / Chromium）
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
        "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        // macOS：用户级安装目录
        "/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary",
        // Linux：发行版常见路径
        "/usr/bin/google-chrome",
        "/usr/bin/google-chrome-stable",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
        "/usr/bin/microsoft-edge",
        "/usr/bin/brave-browser",
    ];
    for c in candidates {
        let p = PathBuf::from(c);
        if p.exists() {
            return Some(p);
        }
    }
    // 兜底：看 PATH 里有没有
    if let Ok(p) = which_browser("google-chrome") {
        return Some(p);
    }
    if let Ok(p) = which_browser("google-chrome-stable") {
        return Some(p);
    }
    if let Ok(p) = which_browser("chromium") {
        return Some(p);
    }
    if let Ok(p) = which_browser("chromium-browser") {
        return Some(p);
    }
    if let Ok(p) = which_browser("microsoft-edge") {
        return Some(p);
    }
    if let Ok(p) = which_browser("brave-browser") {
        return Some(p);
    }
    if let Ok(p) = which_browser("chrome") {
        return Some(p);
    }
    if let Ok(p) = which_browser("msedge") {
        return Some(p);
    }
    None
}

fn which_browser(name: &str) -> Result<PathBuf, ()> {
    let exts = ["", ".exe"];
    let paths = std::env::var_os("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&paths) {
        for ext in exts {
            let candidate = dir.join(format!("{name}{ext}"));
            if candidate.exists() {
                return Ok(candidate);
            }
        }
    }
    Err(())
}

/// 每个平台独立的 profile 目录，cookie/缓存完全隔离。
///
/// 多实例：优先 `BEAV_CHROME_PROFILE_DIR`；否则跟随实例的 `BEAV_STORE_DIR`
/// （`<store>/browser-profile`），避免所有用户共用同一份平台登录态。
pub fn profile_dir_for(app: &AppHandle, platform: &str) -> Result<PathBuf, String> {
    let base = chrome_profile_root(app)?;
    let dir = base.join(platform);
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("创建 profile 目录失败: {error}"))?;
    Ok(dir)
}

fn chrome_profile_root(app: &AppHandle) -> Result<PathBuf, String> {
    if let Ok(raw) = std::env::var("BEAV_CHROME_PROFILE_DIR") {
        let raw = raw.trim();
        if !raw.is_empty() {
            return Ok(PathBuf::from(raw));
        }
    }
    if let Ok(raw) = std::env::var("BEAV_STORE_DIR") {
        let raw = raw.trim();
        if !raw.is_empty() {
            return Ok(PathBuf::from(raw).join("browser-profile"));
        }
    }
    app.path()
        .app_data_dir()
        .map(|base| base.join("browser-profile"))
        .map_err(|error| format!("app_data_dir 不可用: {error}"))
}

/// 清理使用同一 profile（user-data-dir）的残留浏览器进程。
///
/// 只匹配命令行里带我们 profile 路径的 chrome/msedge/brave 进程，
/// 绝不误伤用户自己正常打开的浏览器（它们的命令行没有这个 user-data-dir）。
/// 典型场景：Beav 重启后上次拉起的 Chrome 变孤儿进程还占着 profile 目录，
/// 不清掉的话新 Chrome 会把手权交给旧进程后立即退出，导致端口探测 8s 超时。
#[cfg(windows)]
fn kill_stale_profile_chrome(profile_dir: &PathBuf) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let dir = profile_dir.display().to_string();
    if dir.is_empty() {
        return;
    }
    // PowerShell 单引号字符串里反斜杠无需转义；仅防御路径出现单引号的极端情况
    let dir_ps = dir.replace('\'', "''");
    let script = format!(
        "Get-CimInstance Win32_Process -Filter \"Name='chrome.exe' OR Name='msedge.exe' OR Name='brave.exe'\" | Where-Object {{ $_.CommandLine -like '*{dir_ps}*' }} | ForEach-Object {{ Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }}"
    );
    let _ = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    // 给 OS 一点时间释放 profile 目录锁（进程退出后句柄释放略滞后）
    std::thread::sleep(Duration::from_millis(400));
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn kill_stale_profile_chrome(_profile_dir: &PathBuf) {}

/// macOS / Linux：按 profile 路径精准清掉上次遗留的浏览器进程。
///
/// 只匹配命令行里带我们 profile 路径的 chrome/chromium/msedge/brave 进程，
/// 绝不误伤用户自己正常打开的浏览器。
///
/// 为什么非 Windows 也需要：Beav 重启后会话表清空，但上次拉起的 Chrome 还活着。
/// 再 spawn 同 user-data-dir 的 Chrome 时，新进程会把请求转交给旧进程后立即退出，
/// stderr 里永远等不到 DevTools listening → 8s 超时，表现为「按钮点了没反应」。
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn kill_stale_profile_chrome(profile_dir: &PathBuf) {
    let dir = profile_dir.display().to_string();
    if dir.is_empty() {
        return;
    }
    let output = Command::new("ps")
        .args(["-axo", "pid=,command="])
        .output();
    let stdout = match output {
        Ok(result) => String::from_utf8_lossy(&result.stdout).to_string(),
        Err(_) => return,
    };
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (pid_part, command) = match line.split_once(char::is_whitespace) {
            Some(parts) => parts,
            None => continue,
        };
        let pid: i32 = match pid_part.trim().parse() {
            Ok(value) => value,
            Err(_) => continue,
        };
        // 只杀浏览器进程，且命令行必须包含我们的 profile 目录
        let is_browser = [
            "Google Chrome",
            "chrome",
            "Chromium",
            "chromium",
            "Microsoft Edge",
            "msedge",
            "Brave Browser",
            "brave",
        ]
        .iter()
        .any(|name| command.contains(name));
        if !is_browser || !command.contains(&dir) {
            continue;
        }
        // 避免误杀自己
        if pid == std::process::id() as i32 {
            continue;
        }
        let _ = Command::new("kill")
            .arg(pid.to_string())
            .output();
    }
    // 给 OS 一点时间释放 profile 目录锁（进程退出后句柄释放略滞后）。
    std::thread::sleep(Duration::from_millis(400));
}

/// 为指定平台启动 Chrome 并拉起登录页（独立 profile）。返回该平台的调试端口。
///
/// 每个平台各自一个 Chrome 实例 + 独立 user-data-dir，cookie 完全隔离：
/// 先开小红书再开抖音，会拉起**两个** Chrome，互不干扰。
/// `-remote-debugging-port=0` 让 OS 分配随机端口，再从 Chrome 的 stderr 行
/// `DevTools listening on ws://127.0.0.1:<port>/...` 解析出真实端口（比扫固定端口范围可靠）。
/// 子进程句柄被存进 `CHROME_SESSIONS` 保活，直到 `shutdown_chrome` 显式结束。
/// 端口是否有进程在监听（识别"陈旧会话"用）。
fn debug_port_alive(port: u16) -> bool {
    std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_millis(600),
    )
    .is_ok()
}

pub fn launch_chrome_for_login(
    chrome_path: &PathBuf,
    profile_dir: &PathBuf,
    platform: &str,
    login_url: &str,
) -> Result<u16, String> {
    // 该平台已有会话 → 复用其端口（窗口仍在，用户可直接在该 Chrome 登录/使用）。
    // 注意：只复用「同一平台」的会话，绝不跨平台复用（否则 cookie 串台）。
    {
        let mut sessions = CHROME_SESSIONS.get_or_init(Default::default).lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(session) = sessions.get(platform) {
            if session.port != 0 {
                // 端口必须还活着，否则是陈旧会话（Chrome 已退出）→ 清掉并重新拉起。
                if debug_port_alive(session.port) {
                    return Ok(session.port);
                }
                sessions.remove(platform);
            }
        }
    }

    // Beav 重启后会话表清空，但上次拉起的 Chrome 可能还活着（孤儿进程）。
    // 此时再 spawn 同 user-data-dir 的 Chrome：新进程会把请求转交给旧进程后**立即退出**，
    // stderr 里永远等不到 DevTools listening → 8s 超时。所以先按 profile 路径精准清残留。
    kill_stale_profile_chrome(profile_dir);

    let mut cmd = Command::new(chrome_path);
    cmd.args([
        "--remote-debugging-port=0",
        &format!("--user-data-dir={}", profile_dir.display()),
        "--no-first-run",
        "--no-default-browser-check",
        "--disable-features=Translate,InfiniteSessionRestore",
        "--window-size=1200,800",
        login_url,
    ]);
    // stdout 丢弃，stderr 管道化 —— Chrome 会在 stderr 打印
    // "DevTools listening on ws://127.0.0.1:<port>/devtools/browser/<uuid>"
    cmd.stdout(Stdio::null()).stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|error| format!("启动 Chrome 失败: {error}"))?;

    // 把 stderr 管道接管出来，单独线程逐行读，解析端口。
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "无法捕获 Chrome stderr".to_string())?;
    let (tx, rx) = mpsc::channel::<u16>();
    std::thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines().map_while(Result::ok) {
            if let Some(pos) = line.find("DevTools listening on") {
                let rest = &line[pos..];
                if let Some(start) = rest.find("127.0.0.1:") {
                    let after = &rest[start + "127.0.0.1:".len()..];
                    if let Some(end) = after.find('/') {
                        if let Ok(port) = after[..end].parse::<u16>() {
                            let _ = tx.send(port);
                            return;
                        }
                    }
                }
            }
        }
    });

    // 最多等 8s 拿到端口
    let port = rx
        .recv_timeout(Duration::from_secs(8))
        .map_err(|_| "等待 Chrome 调试端口超时（8s）：Chrome 可能未正常启动，请重试".to_string())?;

    // 保活子进程 + 登记端口，避免作用域结束被 OS 杀掉。
    // 新拉起 Chrome → 端口可能变化 → 旧端口缓存的 ws_url 已失效，先清掉。
    invalidate_ws_url_cache(port);
    let mut sessions = CHROME_SESSIONS.get_or_init(Default::default).lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    sessions.insert(
        platform.to_string(),
        ChromeSession {
            port,
            child: Some(child),
        },
    );
    Ok(port)
}

/// 某平台当前调试端口（0 表示未拉起）。
pub fn active_chrome_port(platform: &str) -> u16 {
    let sessions = CHROME_SESSIONS.get_or_init(Default::default).lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    sessions
        .get(platform)
        .map(|session| session.port)
        .unwrap_or(0)
}

/// 端口对应�� Chrome 是否**还活着**。
///
/// 会话表里的端口会残留（用户手动关掉 Chrome 后进程没了、端口号还在），
/// 直接用它当"已就绪"会让 UI 一直显示已连接，实际抓取全部失败。
pub fn chrome_session_alive(platform: &str) -> bool {
    let port = active_chrome_port(platform);
    port != 0 && debug_port_alive(port)
}

/// 返回所有已就绪的平台名（供 UI 展示）。
pub fn ready_platforms() -> Vec<String> {
    let sessions = CHROME_SESSIONS.get_or_init(Default::default).lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    sessions
        .iter()
        .filter(|(_, session)| session.port != 0 && debug_port_alive(session.port))
        .map(|(platform, _)| platform.clone())
        .collect()
}

/// 判断错误是否属于「Chrome 连接层失效」（WS 断开 / 连接不上 / 响应超时 / DevTools HTTP 不可达）。
/// 这类错误意味着该平台的 Chrome 很可能已被手动关闭或崩溃，缓存的调试端口已失效。
pub fn is_connection_error(message: &str) -> bool {
    let markers = [
        "GET /json/version",
        "error sending request",
        "IO error",
        "os error 10053",
        "os error 10054",
        "连接已关闭",
        "CDP 流已关闭",
        "CDP WebSocket",
        "等待响应超时",
    ];
    markers
        .iter()
        .any(|marker| message.contains(marker))
}

/// 把该平台的会话标记为失效：清端口 + 结束子进程。
/// 之后 `active_chrome_port` 返回 0，UI 显示「未启动」，引导用户重新打开登录。
/// （profile 目录里的 cookie 还在，重新打开无需重新登录，只是重拉一个 Chrome。）
pub fn deactivate_session(platform: &str) {
    let mut sessions = CHROME_SESSIONS.get_or_init(Default::default).lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(mut session) = sessions.remove(platform) {
        if let Some(mut child) = session.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[derive(Debug, Deserialize)]
struct CdpVersionInfo {
    #[serde(rename = "webSocketDebuggerUrl")]
    web_socket_debugger_url: String,
}

/// HTTP GET /json/version 拿 WS URL（异步版本，给 Phase 1 测试用）。
pub async fn fetch_ws_url(port: u16) -> Result<String, String> {
    let url = format!("http://127.0.0.1:{port}{CDP_VERSION_PATH}");
    let response = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .map_err(|error| format!("构建 reqwest 失败: {error}"))?
        .get(&url)
        .send()
        .await
        .map_err(|error| format!("GET /json/version 失败: {error}"))?;
    let info: CdpVersionInfo = response
        .json()
        .await
        .map_err(|error| format!("解析 /json/version 失败: {error}"))?;
    Ok(info.web_socket_debugger_url)
}

/// 按端口缓存 CDP WebSocket URL。
///
/// Chrome 调试端口在一次登录会话内稳定不变（除非用户关掉重开 → 端口会变），
/// 没必要每次抓取详情/列表都再发一次 HTTP `GET /json/version`（最坏 3s 超时）。
/// 之前每次 `scrape_post_detail` / `scrape_profile_posts` 都先 `fetch_ws_url` 一次，
/// 是「抓取时间太长」的主要元凶之一。这里按 port 缓存，命中即省掉一次网络往返。
static WS_URL_CACHE: OnceLock<Mutex<HashMap<u16, String>>> = OnceLock::new();

/// 拿到某端口的 CDP WebSocket URL：命中缓存直接返回，否则 `fetch_ws_url` 并写入缓存。
pub async fn ws_url_for_port(port: u16) -> Result<String, String> {
    if let Some(cached) = WS_URL_CACHE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&port)
    {
        return Ok(cached.clone());
    }
    let url = fetch_ws_url(port).await?;
    WS_URL_CACHE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(port, url.clone());
    Ok(url)
}

/// 某个端口对应的 ws_url 缓存失效（Chrome 重启/重新登录时调用，避免拿到已死的连接）。
pub fn invalidate_ws_url_cache(port: u16) {
    if let Some(cache) = WS_URL_CACHE.get() {
        cache.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).remove(&port);
    }
}

#[derive(Debug, Deserialize)]
struct CdpResponse {
    #[serde(default)]
    id: u64,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<CdpError>,
}

#[derive(Debug, Deserialize)]
struct CdpError {
    message: String,
}

/// 单个 WS 连接的客户端（不持有 state，复用 tokio-tungstenite）。
pub struct CdpClient {
    stream: AsyncMutex<WebSocketStream<MaybeTlsStream<TcpStream>>>,
    next_id: AtomicU32,
}

impl CdpClient {
    pub async fn connect(ws_url: &str) -> Result<Self, String> {
        let request = ws_url
            .into_client_request()
            .map_err(|error| format!("构造 WS 请求失败: {error}"))?;
        let (stream, _response) = timeout(
            Duration::from_secs(5),
            tokio_tungstenite::connect_async(request),
        )
        .await
        .map_err(|_| "连接 CDP WebSocket 超时（5s）".to_string())?
        .map_err(|error| format!("连接 CDP WebSocket 失败: {error}"))?;
        Ok(Self {
            stream: AsyncMutex::new(stream),
            next_id: AtomicU32::new(1),
        })
    }

    /// 优雅关闭 WebSocket（发 Close 帧）。
    ///
    /// 直接 drop 只会关 TCP，Chrome 侧可能把连接当「仍在」保留一会儿；
    /// 显式 Close 让 DevTools 服务端立刻清理，避免下次连接被旧连接状态干扰。
    pub async fn close(&self) {
        let mut stream = self.stream.lock().await;
        let _ = stream.send(Message::Close(None)).await;
        let _ = stream.flush().await;
    }

    /// 发送一条 CDP 命令并等待匹配 id 的响应。
    ///
    /// `session_id` 必须放在**消息顶层**（CDP flatten 模式协议要求），而不是 params 里 ——
    /// 塞进 params 会让命令落在浏览器级会话上，页面域命令（Page.navigate 等）会报
    /// "wasn't found"。
    async fn send_request(
        &self,
        method: &str,
        params: Option<Value>,
        session_id: Option<&str>,
    ) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) as u64;
        let mut message = json!({
            "id": id,
            "method": method,
        });
        if let Some(sid) = session_id {
            message["sessionId"] = json!(sid);
        }
        if let Some(p) = params {
            message["params"] = p;
        }
        let payload = serde_json::to_string(&message)
            .map_err(|error| format!("序列化 CDP 请求失败: {error}"))?;
        let mut stream = self.stream.lock().await;
        stream
            .send(Message::Text(payload))
            .await
            .map_err(|error| format!("发送 CDP 消息失败: {error}"))?;
        // 简单协议：等下一个 id 匹配的消息（不带并发安全，生产可加 broadcast 通道）
        loop {
            let msg = timeout(Duration::from_secs(30), stream.next())
                .await
                .map_err(|_| format!("CDP {method} 等待响应超时（30s）"))?
                .ok_or_else(|| "CDP 流已关闭".to_string())?
                .map_err(|error| format!("读取 CDP 消息失败: {error}"))?;
            match msg {
                Message::Text(text) => {
                    let resp: CdpResponse = serde_json::from_str(&text)
                        .map_err(|error| format!("解析 CDP 响应失败: {error}"))?;
                    if resp.id == id {
                        if let Some(err) = resp.error {
                            return Err(format!("CDP {method} 错误: {}", err.message));
                        }
                        let result = resp.result.unwrap_or(Value::Null);
                        // JS 异常不是协议级 error，藏在 exceptionDetails 里，这里显式抛出
                        if let Some(exception) = result.get("exceptionDetails") {
                            let detail = exception
                                .get("exception")
                                .and_then(|e| e.get("description").or_else(|| e.get("value")))
                                .and_then(Value::as_str)
                                .unwrap_or("未知 JS 异常");
                            return Err(format!("CDP {method} JS 异常: {detail}"));
                        }
                        return Ok(result);
                    }
                    // 不是我们的 id，丢弃继续等
                }
                Message::Binary(_) | Message::Ping(_) | Message::Pong(_) => continue,
                Message::Close(_) => return Err("CDP 连接已关闭".to_string()),
                _ => continue,
            }
        }
    }

    /// 创建一个新 target（页面）并返回其 targetId。（浏览器级命令，无 session）
    pub async fn create_target(&self, url: &str) -> Result<String, String> {
        let result = self
            .send_request(
                "Target.createTarget",
                Some(json!({ "url": url })),
                None,
            )
            .await?;
        result
            .get("targetId")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| "createTarget 返回无 targetId".to_string())
    }

    /// 创建一个**后台**标签页（不抢前台焦点）。
    ///
    /// 探针标签用它：登录态轮询时新建前台标签会让服务器上的 Chrome 窗口
    /// 反复切到小红书主页（用户看到"窗口自己跳走"）。
    pub async fn create_target_background(&self, url: &str) -> Result<String, String> {
        let result = self
            .send_request(
                "Target.createTarget",
                Some(json!({ "url": url, "background": true })),
                None,
            )
            .await?;
        result
            .get("targetId")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| "createTarget 返回无 targetId".to_string())
    }

    /// 列出所有标签页（targetId + url），用于清理遗留页面。
    pub async fn list_targets(&self) -> Result<Vec<(String, String)>, String> {
        let result = self
            .send_request("Target.getTargets", None, None)
            .await?;
        let mut out = Vec::new();
        if let Some(items) = result.get("targetInfos").and_then(Value::as_array) {
            for item in items {
                let id = item.get("targetId").and_then(Value::as_str);
                let url = item.get("url").and_then(Value::as_str);
                let kind = item.get("type").and_then(Value::as_str).unwrap_or("");
                if let (Some(id), Some(url)) = (id, url) {
                    if kind == "page" {
                        out.push((id.to_string(), url.to_string()));
                    }
                }
            }
        }
        Ok(out)
    }

    /// 关闭指定标签页（失败不致命）。
    pub async fn close_target(&self, target_id: &str) {
        let _ = self
            .send_request(
                "Target.closeTarget",
                Some(json!({ "targetId": target_id })),
                None,
            )
            .await;
    }

    /// 附加到指定 target，建立该 target 的 sessionId。（浏览器级命令，无 session）
    pub async fn attach_to_target(&self, target_id: &str) -> Result<String, String> {
        let result = self
            .send_request(
                "Target.attachToTarget",
                Some(json!({
                    "targetId": target_id,
                    "flatten": true,
                })),
                None,
            )
            .await?;
        result
            .get("sessionId")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| "attachToTarget 返回无 sessionId".to_string())
    }

    /// 在已 attach 的 session 中执行 JS 表达式，返回 JS 求值后的真实值
    /// （已解开 CDP `Runtime.evaluate` 的 `result.value` 包裹）。
    pub async fn evaluate(
        &self,
        session_id: &str,
        expression: &str,
        await_promise: bool,
    ) -> Result<Value, String> {
        let mut params = json!({
            "expression": expression,
            "returnByValue": true,
        });
        if await_promise {
            params["awaitPromise"] = json!(true);
        }
        let resp = self
            .send_request("Runtime.evaluate", Some(params), Some(session_id))
            .await?;
        // resp 是 CDP 的 result 对象：{ "result": RemoteObject, "exceptionDetails": ... }
        // 我们只要 RemoteObject 的 value 字段（by-value 求值结果）。
        Ok(resp
            .get("result")
            .and_then(|remote| remote.get("value"))
            .cloned()
            .unwrap_or(Value::Null))
    }

    /// 等到某个 selector 在 DOM 中出现（轮询 evaluate），最多 timeout_secs。
    pub async fn wait_for_selector(
        &self,
        session_id: &str,
        selector: &str,
        timeout_secs: u64,
    ) -> Result<bool, String> {
        let js = format!(
            r#"(() => {{
                const el = document.querySelector({sel});
                return el ? true : false;
            }})()"#,
            sel = serde_json::to_string(selector).unwrap_or_else(|_| format!("\"{selector}\""))
        );
        let deadline = std::time::Instant::now() + Duration::from_secs(timeout_secs);
        while std::time::Instant::now() < deadline {
            let found = self.evaluate(session_id, &js, false).await?;
            let v = found
                .get("result")
                .and_then(|r| r.get("value"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if v {
                return Ok(true);
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        Ok(false)
    }
}

/// 一次性便捷：连 CDP → 建 target → 导航 → 评估 JS。Phase 1 测试用。
pub async fn navigate_and_eval(
    ws_url: &str,
    target_url: &str,
    expression: &str,
) -> Result<Value, String> {
    navigate_and_eval_settle(ws_url, target_url, expression, 2_000, false).await
}

/// 同 `navigate_and_eval`，但可自定义「页面加载后、JS 求值前」的静置毫秒数。
///
/// SPA（抖音 / 小红书）在 body 出现后还要异步拉 XHR；静置太短会在数据到达前就求值，
/// 表现为「明明页面有数据，却一个都没抓到」。抽视频直链这类依赖运行时请求的场景必须放宽。
///
/// `await_promise=true` 用于「表达式本身是 async IIFE」的场景（B 站要先 fetch playurl
/// 才能拿到 DASH 直链，是异步的）；其余平台保持 false，求值结果直接是对象。
pub async fn navigate_and_eval_settle(
    ws_url: &str,
    target_url: &str,
    expression: &str,
    settle_ms: u64,
    await_promise: bool,
) -> Result<Value, String> {
    let client = CdpClient::connect(ws_url).await?;
    let target_id = client.create_target("about:blank").await?;
    // 导航/求值全放进 inner async 块：无论成功还是中途 `?` 返回，
    // 都会走到下面的 Target.closeTarget —— 否则每抓一条就留一个标签页。
    let outcome: Result<Value, String> = async {
        let session_id = client.attach_to_target(&target_id).await?;
        // 导航到目标 URL（sessionId 消息顶层）
        let nav_params = json!({ "url": target_url });
        let _ = client
            .send_request("Page.navigate", Some(nav_params), Some(&session_id))
            .await?;
        // 等真实文档加载完成。
        //
        // ⚠️ 不能用 `wait_for_selector("body")`：新 target 是 about:blank，**它本来就有 body**，
        // 轮询会在导航真正开始前就判定"已就绪"，于是只剩固定 sleep 在兜。抖音/小红书这类 SPA
        // 的状态对象（`_ROUTER_DATA` / `__INITIAL_STATE__`）是异步注入的，等不够就会读了个空页，
        // 表现为"明明页面有视频，却一个直链都抽不到"。所以这里轮询 `readyState === 'complete'`。
        let nav_deadline = std::time::Instant::now() + Duration::from_secs(20);
        loop {
            let state = client
                .evaluate(
                    &session_id,
                    "(() => { try { return document.readyState; } catch (e) { return ''; } })()",
                    false,
                )
                .await;
            if matches!(state.as_ref().ok().and_then(Value::as_str), Some("complete")) {
                break;
            }
            if std::time::Instant::now() >= nav_deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        // readyState 到位后再给 SPA 异步渲染/注流留静置时间
        tokio::time::sleep(Duration::from_millis(settle_ms)).await;
        client.evaluate(&session_id, expression, await_promise).await
    }
    .await;
    // 抓完即关临时标签页（浏览器级命令，无 session），再优雅关闭 WS。
    let _ = client
        .send_request(
            "Target.closeTarget",
            Some(json!({ "targetId": target_id })),
            None,
        )
        .await;
    client.close().await;
    outcome
}

/// 各平台扫码登录配置。
///
/// - `login_url`：登录页
/// - `prepare_js`：进入登录页后要先点的按钮（抖音要先点「登录」才弹二维码）
/// - `cookie_name`：登录成功后该平台必然有的 cookie（游客不会有）
/// - `home_url`：登录态检查用的首页
pub struct QrPlatformSpec {
    pub login_url: &'static str,
    pub prepare_js: &'static str,
    pub cookie_name: &'static str,
    pub home_url: &'static str,
}

const XHS_QR: QrPlatformSpec = QrPlatformSpec {
    login_url: "https://www.xiaohongshu.com/login",
    prepare_js: "",
    cookie_name: "web_session",
    home_url: "https://www.xiaohongshu.com/explore",
};

const DOUYIN_QR: QrPlatformSpec = QrPlatformSpec {
    login_url: "https://www.douyin.com/",
    // 抖音首页右上角「登录」按钮 → 弹出二维码
    prepare_js: r#"(() => {
        const hit = [...document.querySelectorAll('button,div,span,a')]
            .find((el) => (el.textContent || '').trim() === '登录');
        if (hit) { hit.click(); return true; }
        const fallback = document.querySelector('.login-btn,[data-e2e="login-button"],#douyin-header-menuCt button');
        if (fallback) { fallback.click(); return true; }
        return false;
    })()"#,
    cookie_name: "sessionid",
    home_url: "https://www.douyin.com/",
};

const BILIBILI_QR: QrPlatformSpec = QrPlatformSpec {
    login_url: "https://account.bilibili.com/login",
    // 登录页默认展示二维码；存在「扫码登录」页签时点一下
    prepare_js: r#"(() => {
        const hit = [...document.querySelectorAll('div,span,a,button')]
            .find((el) => (el.textContent || '').trim().startsWith('扫码登录'));
        if (hit) { hit.click(); return true; }
        return false;
    })()"#,
    cookie_name: "SESSDATA",
    home_url: "https://www.bilibili.com/",
};

pub fn qr_platform_spec(platform: &str) -> Option<QrPlatformSpec> {
    match platform {
        "xiaohongshu" => Some(XHS_QR),
        "douyin" => Some(DOUYIN_QR),
        "bilibili" => Some(BILIBILI_QR),
        _ => None,
    }
}

/// 找页面上的二维码区域。
///
/// 兼容三种形态：`<img>` / `<canvas>` / 带 background-image 的 div（B 站就是这种）。
const FIND_QR_JS: &str = r#"(() => {
    const cands = [];
    const consider = (el) => {
        const r = el.getBoundingClientRect();
        if (r.width < 110 || r.width > 420) return;
        if (Math.abs(r.width - r.height) > 36) return;
        if (r.x < 0 || r.y < 0) return;
        cands.push({ x: r.x, y: r.y, width: r.width, height: r.height });
    };
    document.querySelectorAll('img,canvas').forEach(consider);
    document.querySelectorAll('div,span').forEach((el) => {
        const bg = getComputedStyle(el).backgroundImage || '';
        if (bg !== 'none' && (bg.includes('data:image') || bg.includes('base64'))) consider(el);
    });
    if (!cands.length) return null;
    cands.sort((a, b) => b.width - a.width);
    return cands[0];
})()"#;

/// 打开登录页并截取二维码（返回 data URL）。
///
/// 用于 Web 端扫码登录：管理员在浏览器里看到平台自己的登录二维码，
/// 用手机 App 扫，登录态落在该平台 profile 里，之后抓取复用。
/// 从 DOM 里**直接取出二维码图像数据**（而不是截图裁剪）。
///
/// 截图裁剪有两个硬伤：
/// 1. 页面上常同时存在多个二维码（登录码、"扫码下载 App"码），按"最大近正方形"选容易选错；
/// 2. 截到的是失效占位图时，用户扫码永远提示过期 —— 看起来"刷新了还是过期"。
///
/// 这里改为：canvas 用 `toDataURL()`，img 用 data URL 或 `fetch`（带 cookie）转 base64，
/// 并对"已失效/点击刷新"文案先点一次刷新。
const EXTRACT_QR_JS: &str = r#"(async () => {
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  const bodyText = () => ((document.body && document.body.innerText) || '');
  const classOf = (el) => {
    const c = el.className;
    if (!c) return '';
    return (typeof c === 'string') ? c : (c.baseVal || '');
  };
  // 小红书的**过期占位图本身就是一张图**（图上写着"二维码已过期 点击刷新"），
  // 它同样满足"近正方形、尺寸合适" → 会被误当成二维码截出来，用户扫了永远提示过期。
  // 所以：先认过期态 → 点"点击刷新" → 等新码 → 再提取；仍过期就明确报错。
  const EXPIRED_RE = /二维码已过期|二维码已失效|二维码过期|二维码失效/;
  const clickRefresh = () => {
    const cands = [...document.querySelectorAll('span,a,div,i,button,em')]
      .filter((el) => {
        const t = (el.textContent || '').trim();
        // 只认明确的"刷新二维码"语义，不认"重新加载"这类泛化文案（会误点页面别处）
        if (t !== '点击刷新' && t !== '刷新二维码' && t !== '重新获取二维码' && t !== '点击刷新二维码') return false;
        const r = el.getBoundingClientRect();
        return r.width > 0 && r.height > 0 && r.width < 160;
      });
    if (!cands.length) return false;
    // 面积最小的 = 真正的链接/按钮（外层容器文本也含它）
    cands.sort((a, b) => {
      const ra = a.getBoundingClientRect();
      const rb = b.getBoundingClientRect();
      return ra.width * ra.height - rb.width * rb.height;
    });
    cands[0].click();
    return true;
  };
  let refreshed = false;
  if (EXPIRED_RE.test(bodyText())) {
    if (clickRefresh()) {
      refreshed = true;
      await sleep(2500);
    }
  }
  const stillExpired = EXPIRED_RE.test(bodyText());
  const isQrBox = (el) => {
    const r = el.getBoundingClientRect();
    return r.width >= 110 && r.width <= 460
      && Math.abs(r.width - r.height) <= 36 && r.x >= 0 && r.y >= 0;
  };
  const score = (el) => {
    let s = 0;
    const src = (el.currentSrc || el.src || '');
    if (/qrcode|qr-code|qr_|login-?qr|scan/i.test(src + ' ' + classOf(el))) s += 100;
    if (src.startsWith('data:image')) s += 60;
    const near = el.parentElement ? (el.parentElement.innerText || '') : '';
    if (/扫码登录|手机号登录|登录/.test(near)) s += 30;
    if (/下载.*APP|下载客户端|扫码下载/i.test(near)) s -= 120;
    // 过期占位图：所在容器里会出现"二维码已过期 / 点击刷新" → 强烈排除
    if (EXPIRED_RE.test(near)) s -= 200;
    return s;
  };
  // 2) canvas 直接导出
  for (const c of [...document.querySelectorAll('canvas')].filter(isQrBox)) {
    try {
      const d = c.toDataURL('image/png');
      if (d && d.length > 2000) return { dataUrl: d, kind: 'canvas', refreshed, expired: stillExpired };
    } catch (e) {}
  }
  // 3) img：data URL 直接用；普通 URL 带 cookie 取回转 base64
  const imgs = [...document.querySelectorAll('img')].filter(isQrBox);
  imgs.sort((a, b) => score(b) - score(a));
  for (const img of imgs) {
    const src = img.currentSrc || img.src || '';
    if (!src) continue;
    if (src.startsWith('data:image')) {
      if (src.length > 2000) return { dataUrl: src, kind: 'img-data', refreshed, expired: stillExpired };
      continue;
    }
    try {
      const resp = await fetch(src, { credentials: 'include' });
      const blob = await resp.blob();
      const d = await new Promise((res) => {
        const fr = new FileReader();
        fr.onload = () => res(fr.result);
        fr.onerror = () => res(null);
        fr.readAsDataURL(blob);
      });
      if (d && d.length > 2000) return { dataUrl: d, kind: 'img-fetch', refreshed, expired: stillExpired };
    } catch (e) {}
  }
  const text = bodyText();
  return {
    dataUrl: null,
    kind: 'none',
    refreshed,
    expired: stillExpired || EXPIRED_RE.test(text),
    snippet: text.replace(/\s+/g, ' ').slice(0, 240),
  };
})()"#;

/// 扫码结果：图像 + 诊断信息。
pub struct QrCapture {
    pub data_url: Option<String>,
    pub kind: String,
    pub refreshed: bool,
    pub expired: bool,
    pub snippet: String,
}

pub async fn capture_login_qr(ws_url: &str, spec: &QrPlatformSpec) -> Result<QrCapture, String> {
    let client = CdpClient::connect(ws_url).await?;

    // 登录标签页的生命周期很关键：
    // 小红书扫码登录靠**这个标签页**轮询二维码状态、拿到会话后写 cookie。
    // 如果关掉它（或另开新标签），手机端会显示"登录成功"，但网页端什么也没拿到
    // —— 用户实测就是这样：手机成功、页面仍显示未登录。
    //
    // 所以：同一个标签页**只创建一次并保留**；要换新码就在同一标签内
    // 清 sessionStorage（二维码缓存在这里）后再导航。
    let existing = {
        let tabs = PROBE_TABS
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        tabs.get(spec.login_url).cloned()
    };

    let target_id = match existing {
        Some(target) => {
            // 校验它是否还活着；死了就重建
            let alive = client
                .list_targets()
                .await
                .map(|targets| targets.iter().any(|(id, _)| *id == target))
                .unwrap_or(false);
            if alive {
                target
            } else {
                let created = client.create_target("about:blank").await?;
                let mut tabs = PROBE_TABS
                    .get_or_init(Default::default)
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                tabs.insert(spec.login_url.to_string(), created.clone());
                created
            }
        }
        None => {
            let created = client.create_target("about:blank").await?;
            let mut tabs = PROBE_TABS
                .get_or_init(Default::default)
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            tabs.insert(spec.login_url.to_string(), created.clone());
            created
        }
    };

    let session_id = client.attach_to_target(&target_id).await?;

    // 清掉二维码缓存（sessionStorage）→ 否则重新导航还是那张过期图
    let _ = client
        .evaluate(
            &session_id,
            "(() => { try { sessionStorage.clear(); } catch (e) {} return true; })()",
            false,
        )
        .await;
    let _ = client
        .send_request(
            "Page.navigate",
            Some(json!({ "url": spec.login_url })),
            Some(&session_id),
        )
        .await?;

    // 等页面加载完成（最多 15s）
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        let state = client
            .evaluate(
                &session_id,
                "(() => { try { return document.readyState; } catch (e) { return ''; } })()",
                false,
            )
            .await;
        if matches!(state.as_ref().ok().and_then(Value::as_str), Some("complete")) {
            break;
        }
        if std::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    // 平台特有的前置操作（抖音要点「登录」弹窗）
    if !spec.prepare_js.is_empty() {
        let _ = client.evaluate(&session_id, spec.prepare_js, false).await;
    }
    // 给登录弹窗/二维码渲染留时间
    tokio::time::sleep(Duration::from_secs(3)).await;

    // 直接从 DOM 取图像数据（轮询几次，等二维码渲染完）
    let mut extracted = None;
    for _ in 0..3 {
        let value = client.evaluate(&session_id, EXTRACT_QR_JS, true).await.ok();
        let has_image = value
            .as_ref()
            .and_then(|v| v.get("dataUrl"))
            .and_then(Value::as_str)
            .is_some();
        extracted = value;
        if has_image {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1500)).await;
    }

    // 没取到 → 可能是登录弹窗没打开（小红书首页要先点「登录」）→ 点一下再取
    if extracted
        .as_ref()
        .and_then(|value| value.get("dataUrl"))
        .and_then(Value::as_str)
        .is_none()
    {
        const OPEN_LOGIN_JS: &str = r#"(async () => {
            const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
            // 必须点**最内层**的那个"登录"：外层容器（整个 header div）的文本也包含"登录"，
            // 点它不会打开登录弹窗 —— 这是"点了没反应"的根因。
            const cands = [...document.querySelectorAll('button,a,span,div,i')]
                .filter((el) => {
                    const t = (el.textContent || '').trim();
                    if (t !== '登录' && !t.startsWith('登录')) return false;
                    if (el.querySelector('button,a,span,div,i')) {
                        // 含子元素 → 可能是容器；只有当子元素文本不同时才算候选
                        const kids = [...el.children].some((c) => {
                            const ct = (c.textContent || '').trim();
                            return ct === '登录' || ct.startsWith('登录');
                        });
                        if (kids) return false;
                    }
                    const r = el.getBoundingClientRect();
                    return r.width > 0 && r.height > 0;
                });
            if (!cands.length) return false;
            // 面积最小的 = 最内层 = 真正的按钮
            cands.sort((a, b) => {
                const ra = a.getBoundingClientRect();
                const rb = b.getBoundingClientRect();
                return ra.width * ra.height - rb.width * rb.height;
            });
            cands[0].click();
            await sleep(2500);
            return true;
        })()"#;
        let _ = client.evaluate(&session_id, OPEN_LOGIN_JS, true).await;
        for _ in 0..3 {
            let value = client.evaluate(&session_id, EXTRACT_QR_JS, true).await.ok();
            let has_image = value
                .as_ref()
                .and_then(|v| v.get("dataUrl"))
                .and_then(Value::as_str)
                .is_some();
            extracted = value;
            if has_image {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1500)).await;
        }
    }
    let kind = extracted
        .as_ref()
        .and_then(|value| value.get("kind"))
        .and_then(Value::as_str)
        .unwrap_or("none")
        .to_string();
    let refreshed = extracted
        .as_ref()
        .and_then(|value| value.get("refreshed"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let expired = extracted
        .as_ref()
        .and_then(|value| value.get("expired"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let snippet = extracted
        .as_ref()
        .and_then(|value| value.get("snippet"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let data_url = extracted
        .as_ref()
        .and_then(|value| value.get("dataUrl"))
        .and_then(Value::as_str)
        .map(|value| value.to_string());

    // 提取到的若是**过期占位图**（小红书把"二维码已过期 点击刷新"画成一张图），
    // 绝不能返回给用户 —— 他扫了只会再看到"已过期"，还以为是自己手慢。
    // 此时明确报错，让前端提示"二维码已过期，请重试"。
    if expired {
        client.close().await;
        return Err("二维码已过期，请重试（点「刷新二维码」重新获取）".to_string());
    }

    // 兜底：DOM 取不到就退回"找元素 + 截图裁剪"。
    // 注意：**没有找到元素就不要截图** —— 整页截图会得到一张白图（用户扫不了，
    // 还以为是"二维码过期"）。此时直接报错并带上诊断信息。
    let (data_url, kind) = match data_url {
        Some(url) => (url, kind),
        None => {
            let rect = client
                .evaluate(&session_id, FIND_QR_JS, false)
                .await
                .ok()
                .filter(|value| value.is_object());
            let Some(rect) = rect else {
                client.close().await;
                return Err(format!(
                    "未在登录页找到二维码（诊断：{}）",
                    if snippet.is_empty() {
                        "页面无可见文本".to_string()
                    } else {
                        format!("页面文本「{}」", snippet)
                    }
                ));
            };
            let params = json!({
                "format": "png",
                "captureBeyondViewport": true,
                "clip": {
                    "x": rect.get("x").and_then(Value::as_f64).unwrap_or(0.0),
                    "y": rect.get("y").and_then(Value::as_f64).unwrap_or(0.0),
                    "width": rect.get("width").and_then(Value::as_f64).unwrap_or(320.0),
                    "height": rect.get("height").and_then(Value::as_f64).unwrap_or(320.0),
                    "scale": 1
                }
            });
            let shot = client
                .send_request("Page.captureScreenshot", Some(params), Some(&session_id))
                .await?;
            let data = shot
                .get("data")
                .and_then(Value::as_str)
                .ok_or_else(|| "截图未返回数据".to_string())?;
            (
                format!("data:image/png;base64,{data}"),
                format!("{kind}-screenshot"),
            )
        }
    };

    client.close().await;
    Ok(QrCapture {
        data_url: Some(data_url),
        kind,
        refreshed,
        expired,
        snippet,
    })
}

/// 检查某平台是否**真实登录**。
///
/// 各平台判定方式不同：
/// - 小红书：游客也会拿到 `web_session`，只能看页面状态（explore 是否仍弹登录框）；
/// - 抖音：登录后必有 `sessionid`；
/// - B 站：登录后必有 `SESSDATA`。
pub async fn login_logged_in(ws_url: &str, platform: &str) -> Result<bool, String> {
    let spec = qr_platform_spec(platform).ok_or_else(|| format!("不支持的平台：{platform}"))?;
    let client = CdpClient::connect(ws_url).await?;

    // 抖音 / B 站：cookie 判定足够可靠且便宜。
    if platform != "xiaohongshu" {
        let matched = has_login_cookie(&client, spec.cookie_name, platform).await;
        client.close().await;
        return Ok(matched);
    }

    // 小红书：游客也会拿到 web_session，只能看页面状态。
    //
    // 但**不要每次都开一个标签页**：扫码页轮询是 5 秒一次，每次都开小红书首页再关掉，
    // 服务器上会看到"窗口反复跳出小红书主页然后关闭"，还会打断正在等待扫码的登录页。
    // 做法：先用 cookie 签名判断会话有没有变；没变就直接用上次的结果，**完全不动浏览器**。
    let signature = cookie_signature(&client, platform).await;
    {
        let cache = LOGIN_STATE_CACHE
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some((cached_signature, cached_result, checked_at)) = cache.get(platform) {
            let fresh_enough =
                *cached_result || checked_at.elapsed() < NOT_LOGGED_IN_CACHE_TTL;
            if *cached_signature == signature && fresh_enough {
                client.close().await;
                return Ok(*cached_result);
            }
        }
    }

    // 会话变了（扫码成功/被登出）→ 才去看页面。
    // 复用同一个探针标签页，不再每次新建+关闭。
    let target_id = {
        let mut tabs = PROBE_TABS
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match tabs.get(platform).cloned() {
            Some(existing) => existing,
            None => {
                let created = client.create_target_background(spec.home_url).await?;
                tabs.insert(platform.to_string(), created.clone());
                created
            }
        }
    };
    let session_id = client.attach_to_target(&target_id).await?;
    // 每次检查都重新导航一次，保证 SSR 状态是最新的（登录后需要重载才看得到）
    let _ = client
        .send_request(
            "Page.navigate",
            Some(json!({ "url": spec.home_url })),
            Some(&session_id),
        )
        .await;

    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        let state = client
            .evaluate(
                &session_id,
                "(() => { try { return document.readyState; } catch (e) { return ''; } })()",
                false,
            )
            .await;
        if matches!(state.as_ref().ok().and_then(Value::as_str), Some("complete")) {
            break;
        }
        if std::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    // 给弹窗渲染留时间
    tokio::time::sleep(Duration::from_millis(1500)).await;

    let logged = client
        .evaluate(
            &session_id,
            r#"(() => {
                // 判据要稳：不能"页面上出现扫码登录字样"就判未登录 ——
                // 小红书登录后页面上仍可能残留登录弹窗相关文案，导致误判。
                // 主判据：页头是否还有「登录」按钮（未登录才显示）。
                const visible = (el) => {
                    const r = el.getBoundingClientRect();
                    return r.width > 0 && r.height > 0 && r.width < 160;
                };
                const loginButtons = [...document.querySelectorAll('button,a,span,div')]
                    .filter((el) => {
                        const t = (el.textContent || '').trim();
                        if (t !== '登录' && t !== '登录/注册') return false;
                        if (!visible(el)) return false;
                        // 最内层才算（外层容器文本也可能等于"登录"）
                        return ![...el.children].some((c) => {
                            const ct = (c.textContent || '').trim();
                            return ct === '登录' || ct === '登录/注册';
                        });
                    });
                // INITIAL_STATE 有明确布尔值时以它为准
                try {
                    const s = window.__INITIAL_STATE__;
                    const u = s && s.user;
                    if (u && typeof u.loggedIn === 'boolean') return u.loggedIn;
                    if (u && u.userInfo && (u.userInfo.nickname || u.userInfo.userId)) return true;
                } catch (e) {}
                const hasSession = (document.cookie || '').includes('web_session=');
                // 页头还有「登录」按钮 → 未登录
                if (loginButtons.length > 0) return false;
                return hasSession;
            })()"#,
            false,
        )
        .await
        .ok()
        .and_then(|value| value.as_bool())
        .unwrap_or(false);

    {
        let mut cache = LOGIN_STATE_CACHE
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        cache.insert(
            platform.to_string(),
            (signature, logged, std::time::Instant::now()),
        );
    }

    // 登录成功后，那个登录页标签就没用了 → 关掉，别在 Chrome 里留着
    if logged {
        let login_target = {
            let mut tabs = PROBE_TABS
                .get_or_init(Default::default)
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            tabs.remove(spec.login_url)
        };
        if let Some(login_target) = login_target {
            client.close_target(&login_target).await;
        }
    }

    client.close().await;
    Ok(logged)
}

/// 该平台 cookie 的指纹（只看该域名的 cookie 名+值）。
///
/// 用于判断"会话有没有变"：变了才需要重新加载页面看登录态。
async fn cookie_signature(client: &CdpClient, platform: &str) -> String {
    let Ok(result) = client.send_request("Storage.getCookies", None, None).await else {
        return String::new();
    };
    let domain = platform_domain(platform);
    let mut parts: Vec<String> = result
        .get("cookies")
        .and_then(Value::as_array)
        .map(|cookies| {
            cookies
                .iter()
                .filter_map(|cookie| {
                    let cookie_domain = cookie.get("domain").and_then(Value::as_str)?;
                    if !cookie_domain.contains(domain) {
                        return None;
                    }
                    let name = cookie.get("name").and_then(Value::as_str)?;
                    let value = cookie.get("value").and_then(Value::as_str)?;
                    Some(format!("{name}={value}"))
                })
                .collect()
        })
        .unwrap_or_default();
    parts.sort();
    parts.join("|")
}

/// 该平台是否存在登录态 cookie。
async fn has_login_cookie(client: &CdpClient, name: &str, platform: &str) -> bool {
    let result = match client.send_request("Storage.getCookies", None, None).await {
        Ok(value) => value,
        Err(_) => return false,
    };
    let domain = platform_domain(platform);
    result
        .get("cookies")
        .and_then(Value::as_array)
        .map(|cookies| {
            cookies.iter().any(|cookie| {
                let cookie_name = cookie.get("name").and_then(Value::as_str).unwrap_or("");
                let cookie_domain = cookie.get("domain").and_then(Value::as_str).unwrap_or("");
                let cookie_value = cookie.get("value").and_then(Value::as_str).unwrap_or("");
                cookie_name == name && !cookie_value.is_empty() && cookie_domain.contains(domain)
            })
        })
        .unwrap_or(false)
}

fn platform_domain(platform: &str) -> &'static str {
    match platform {
        "douyin" => "douyin.com",
        "bilibili" => "bilibili.com",
        _ => "xiaohongshu.com",
    }
}

/// 剥掉 URL 里的 `modal_id` 查询参数（抖音视频弹窗）。
///
/// 用户从浏览器复制抖音主页链接时若开着某个视频弹窗，链接会带上 `modal_id`；
/// 打开这种链接，页面会在作品列表之外挂出「相关推荐」里**其他博主**的 `/video/`
/// 链接 → 被抽取器误收成该博主的作品（表现为「混了几个不是这个博主的视频」，
/// 且抓取数 > 主页作品总数）。导航前剥掉，保证落在干净的主页。
/// 剥掉主页 URL 上不该带走的参数。
///
/// - `modal_id`：抖音弹窗参数。
/// - `xsec_token` / `xsec_source`：小红书防盗链令牌，**与浏览器会话绑定**。
///   用户在自己电脑上复制的主页链接带的是他本机的 token，搬到服务器上无效
///   （甚至会被判无效）。服务器抓取时用的是服务器自己的登录态，主页里的笔记链接
///   会由登录态 SSR 重新带上服务器的有效 token，所以这里统一剥掉。
///
/// 注意：仅用于**主页**URL；笔记详情 URL 的 token 必须保留（否则 404）。
fn strip_modal_id_param(url: &str) -> String {
    let Some((base, query)) = url.split_once('?') else {
        return url.to_string();
    };
    let kept: Vec<&str> = query
        .split('&')
        .filter(|p| !p.is_empty())
        .filter(|p| {
            !matches!(
                p.split('=').next(),
                Some("modal_id") | Some("xsec_token") | Some("xsec_source")
            )
        })
        .collect();
    if kept.is_empty() {
        base.to_string()
    } else {
        format!("{base}?{}", kept.join("&"))
    }
}

/// 公开版：给 watch 存储用（与导航时保持一致，避免库里留着别人的 token）。
pub fn normalize_profile_url(url: &str) -> String {
    strip_modal_id_param(url)
}

/// 通过已登录的 Chrome 抓取博主主页帖子（小红书 / 抖音等 JS 渲染站点）。
///
/// 需要 `monitor:chrome-launch` 已拉起 Chrome 并完成登录。`wait_selector` 是平台特有
/// 的帖子锚点（小红书 `a[href*="/explore/"]`、抖音 `a[href*="/video/"]`）。
/// 返回帖子数组：`[{ url, title, likes, platform }]`。
pub async fn scrape_profile_posts(
    ws_url: &str,
    profile_url: &str,
    wait_selector: &str,
    limit: usize,
) -> Result<Value, String> {
    let client = CdpClient::connect(ws_url).await?;
    let target_id = client.create_target("about:blank").await?;
    let session_id = client.attach_to_target(&target_id).await?;
    let profile_url = strip_modal_id_param(profile_url);
    let nav_params = json!({ "url": profile_url });
    let _ = client
        .send_request("Page.navigate", Some(nav_params), Some(&session_id))
        .await?;
    // 等帖子锚点出现（最多 20s），再给动态渲染/图片懒加载 3s 缓冲。
    let _ = client.wait_for_selector(&session_id, wait_selector, 20).await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    // 通用抽取：同时兼容小红书 /explore/ 与抖音 /video/ 两种链接形态。
    //
    // 关键点（踩过的坑）：
    // 1. 小红书卡片里，封面 <a> 只包图片；标题是**同级**的 a.title，点赞数在卡片层
    //    的 .like-wrapper .count —— 所以必须按 note id 分组后，回到**卡片容器**里找标题/互动。
    // 2. URL 必须保留完整 href（含 ?xsec_token=...），小红书现在没 token 打不开笔记详情。
    // 3. 计数文本是 "1.3万" / "872" 这类格式，要解析中文单位。
    let extractor = r#"
        (async () => {
            const LIMIT = __LIMIT__;
            const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
            // 抖音主页若带视频弹窗（URL modal_id），页面会把「相关推荐」里其他博主的
            // /video/ 链接挂进 DOM；锚点收集限定到作品容器（存在才限定，缺失/改版回退全文档，
            // 不影响小红书——它没有该属性）。
            const worksRoot = () => document.querySelector('[data-e2e="user-post-list"]') || document;
            // —— 滚动加载（关键）——
            // 博主主页是**无限滚动网格**：不滚只能拿到首屏（实测小红书首屏约 30 条）。
            // 「增量」窗口若被首屏卡死，博主单日爆更超过窗口大小时，
            // 中间那批帖既不在今天的首屏、昨天也没抓到 → 永久漏抓。
            // 所以这里向下滚到收集够 LIMIT、或连续若干屏无新增为止。
            const seenNotes = new Set();
            const countNotes = () => {
                for (const a of worksRoot().querySelectorAll('a[href*="/explore/"], a[href*="/video/"]')) {
                    const href = a.getAttribute('href') || '';
                    const me = href.match(/\/explore\/([A-Za-z0-9]+)/);
                    const mv = href.match(/\/video\/(\d+)/);
                    const nid = me ? me[1] : (mv ? mv[1] : null);
                    if (nid) seenNotes.add(nid);
                }
                return seenNotes.size;
            };
            // 诊断信息：真机跑一次就能看出「是没滚到 / 滚错了容器 / 页面就是不给更多」。
            const diag = { rounds: 0, counts: [], scrolled: false, scroller: '', maxScrollTop: 0, scrollHeight: 0, clientHeight: 0 };
            // 小红书/抖音用户主页的帖子列表容器 class/id 经常变，多试几个候选。
            const CANDIDATE_SELECTORS = [
                '#userPostedFeeds', '.feeds-container', '#noteContainer', '.note-list',
                '[class*="feeds"]', '[class*="note-list"]', '[class*="works"]',
                '[class*="post-list"]', '[class*="user-post"]', '[class*="scroll"]'
            ];
            // 动态找滚动容器：从小红书笔记卡片向上找第一个 overflow-y 可滚动、内容超高的祖先，
            // 比硬编码 class 列表更稳（XHS class 常变，硬编码容易选错容器 → 页面不动）。
            const findScrollable = (start) => {
                let node = start;
                while (node && node !== document.body && node !== document.documentElement) {
                    const style = getComputedStyle(node);
                    const oy = style.overflowY;
                    if ((oy === 'auto' || oy === 'scroll' || oy === 'overlay') && node.scrollHeight > node.clientHeight + 5) {
                        return node;
                    }
                    node = node.parentElement;
                }
                return null;
            };
            const scrollTargets = () => {
                const els = [];
                // 0) 先按笔记卡片动态定位真正的滚动容器（优先，最稳）
                const anchor = worksRoot().querySelector('a[href*="/explore/"], a[href*="/video/"]');
                const sc = anchor ? findScrollable(anchor) : null;
                if (sc) {
                    els.push(sc);
                    if (!diag.scroller) { diag.scroller = sc.id ? '#' + sc.id : (typeof sc.className === 'string' && sc.className ? '.' + sc.className.split(' ')[0] : sc.tagName); }
                }
                // 1) 文档滚动容器（多数站点是 window 滚动）
                if (document.scrollingElement) els.push(document.scrollingElement);
                if (document.body) els.push(document.body);
                if (document.documentElement) els.push(document.documentElement);
                // 2) 已知容器选择器
                for (const sel of CANDIDATE_SELECTORS) {
                    for (const el of Array.from(document.querySelectorAll(sel))) els.push(el);
                }
                // 3) 任何明显可滚的容器
                for (const el of Array.from(document.querySelectorAll('div,main,section'))) {
                    if (el.scrollHeight > el.clientHeight + 200 && el.clientHeight > 300) els.push(el);
                }
                const seen = new Set();
                return els.filter((el) => {
                    if (!el || seen.has(el)) return false;
                    seen.add(el);
                    return true;
                }).slice(0, 12);
            };
            const scrollAll = () => {
                let moved = false;
                for (const el of scrollTargets()) {
                    try {
                        const before = el.scrollTop;
                        el.scrollTop = el.scrollHeight;
                        diag.maxScrollTop = Math.max(diag.maxScrollTop, el.scrollTop);
                        diag.scrollHeight = Math.max(diag.scrollHeight, el.scrollHeight);
                        diag.clientHeight = Math.max(diag.clientHeight, el.clientHeight);
                        if (el.scrollTop > before + 10) {
                            moved = true;
                            if (!diag.scroller) {
                                const cls = (typeof el.className === 'string' && el.className) ? el.className.split(' ')[0] : '';
                                diag.scroller = el.id ? '#' + el.id : (cls ? '.' + cls : el.tagName);
                            }
                        }
                    } catch (e) {}
                }
                // 兜底：真实 window 滚动（触发 scroll 事件 + IntersectionObserver 懒加载）
                try {
                    window.scrollTo(0, document.body ? document.body.scrollHeight : 0);
                    if (window.scrollY > 0) moved = true;
                } catch (e) {}
                if (moved) diag.scrolled = true;
                return moved;
            };
            let lastCount = 0;
            let stagnant = 0;
            // exhausted = 真的翻到主页底部了（连续 4 屏无新增）。
            // 这是「历史已抓完」的判据 —— 只有翻到底了才能说"这个博主的历史补齐了"；
            // 若是因为触到 limit 上限而停，说明历史可能还没抓完，不能算补齐。
            let exhausted = false;
            // 滚动轮数按 limit 自适应：每滚一屏大约能多加载几条笔记，
            // 固定轮数会让大 limit（补历史用 200）根本翻不到底。
            const MAX_ROUNDS = 60;
            const ROUNDS = Math.max(15, Math.min(MAX_ROUNDS, Math.ceil(LIMIT / 4)));
            for (let i = 0; i < ROUNDS; i++) {
                const n = countNotes();
                diag.rounds = i + 1;
                diag.counts.push(n);
                if (n >= LIMIT) break;
                if (n === lastCount) {
                    stagnant += 1;
                    if (stagnant >= 4) { exhausted = true; break; }
                } else {
                    stagnant = 0;
                    lastCount = n;
                }
                scrollAll();
                // 小红书懒加载要时间去请求下一页，700ms 太短容易误判成"翻不动了"
                await sleep(1200);
            }
            const parseCount = (text) => {
                if (!text) return 0;
                const t = String(text).trim().replace(/,/g, '');
                const m = t.match(/^([\d.]+)\s*(万|w|W|k|K)?$/);
                if (!m) return 0;
                const n = parseFloat(m[1]);
                if (!isFinite(n)) return 0;
                const unit = m[2] || '';
                if (unit === '万' || unit === 'w' || unit === 'W') return Math.round(n * 10000);
                if (unit === 'k' || unit === 'K') return Math.round(n * 1000);
                return Math.round(n);
            };
            // 抖音卡片标题清洗（与分页版 `scrape_profile_posts_paged` 同一套规则）：
            // textContent 会拼成「置顶3.7万<正文><正文>」——开头是置顶徽标 + 独立子元素里的点赞数，
            // 末尾重复一遍同一段文案。数字后紧跟量词时（「10个技巧」）不剥，避免误伤正文。
            const UNIT_HEAD = /^[个种条步招篇天年月日岁次位项秒分]/;
            const dedupeTail = (s) => {
                for (let i = Math.ceil(s.length / 2); i < s.length; i++) {
                    const tail = s.slice(i);
                    if (tail.length >= 8 && s.startsWith(tail)) return s.slice(0, i).trim();
                }
                return s;
            };
            const cleanTitle = (raw) => {
                let t = String(raw || '').replace(/\s+/g, ' ').trim();
                if (!t) return '';
                t = t.replace(/^(?:置顶|置頂)\s*/, '');
                // 顺序要求：**先剥前导点赞数再做重复检测** —— 否则前缀带着「667」，
                // 与尾部重复段对不上，永远查不到重复。
                const m = t.match(/^[\d.]+\s*(?:万|w|W|k|K)?\s*/);
                if (m) {
                    const rest = t.slice(m[0].length).trim();
                    if (rest.length >= 6 && !UNIT_HEAD.test(rest)) t = rest;
                }
                return dedupeTail(t).trim();
            };
            const idOf = (href) => {
                const me = href.match(/\/explore\/([A-Za-z0-9]+)/);
                // B 站视频是 /video/BV1xx…（字母开头），抖音是 /video/<纯数字>，先判 BV。
                const mb = href.match(/\/video\/(BV[0-9A-Za-z]+)/);
                const mv = href.match(/\/video\/(\d+)/);
                if (me) return { id: me[1], kind: 'xiaohongshu' };
                if (mb) return { id: mb[1], kind: 'bilibili' };
                if (mv) return { id: mv[1], kind: 'douyin' };
                return null;
            };
            const anchors = Array.from(worksRoot().querySelectorAll('a[href*="/explore/"], a[href*="/video/"]'));
            // 按 note id 分组（同一笔记有封面链接 + 标题链接两个 <a>）
            const groups = new Map();
            for (const a of anchors) {
                const info = idOf(a.getAttribute('href') || '');
                if (!info) continue;
                if (!groups.has(info.id)) groups.set(info.id, { kind: info.kind, anchors: [] });
                groups.get(info.id).anchors.push(a);
            }
            const results = [];
            for (const [id, group] of groups) {
                const first = group.anchors[0];
                // 卡片容器：往上找包含标题/互动数的最近祖先
                let card = first.closest('section.note-item, [class*="note-item"], [class*="note-card"], li, article') || first.parentElement;
                if (group.kind === 'bilibili') {
                    // 坑：B 站封面 <a> 自身 class 是 `bili-video-card__image--link`（含 bili-video-card 子串），
                    // 用 `[class*="bili-video-card"]` 会命中锚点本身 → 卡内找不到标题，
                    // 标题退化成锚点的 textContent =「播放量+弹幕+时长」（如 251.2万574224:50）。
                    // 必须向上找到**真正含有标题节点**的祖先。
                    let node = first;
                    for (let i = 0; i < 6 && node; i++, node = node.parentElement) {
                        if (node.querySelector && node.querySelector('[class*="info--tit"], h3')) { card = node; break; }
                    }
                }
                let title = '';
                // 1) 同组锚点里 class 含 title 的（xhs 标题本身就是 a.title）
                for (const a of group.anchors) {
                    if (/title/i.test(a.className || '')) {
                        title = (a.getAttribute('title') || a.textContent || '').trim();
                        break;
                    }
                }
                // 2) 卡片内任意 [class*=title]
                if (!title && card) {
                    const t = card.querySelector('[class*="title"]');
                    if (t) title = (t.getAttribute('title') || t.textContent || '').trim();
                }
                // 3) 锚点自身文字兜底
                if (!title) title = (first.textContent || '').trim();
                // 抖音：优先结构化描述节点 + 清洗（去置顶徽标 / 点赞数 / 重复文案）
                if (group.kind === 'douyin') {
                    const scope = first.closest('[data-e2e="user-post-item"], li, section, article') || first.parentElement;
                    const descEl = scope && scope.querySelector('[data-e2e="user-post-item-desc"], [data-e2e="video-desc"]');
                    const desc = descEl ? (descEl.textContent || '').trim() : '';
                    title = cleanTitle(desc || title);
                }
                // B 站：改版可能换 class，用多候选兜底（标题节点 / 视频链接 title / 封面 img alt / 锚点 title），
                // 并把「播放量+弹幕+时长」这类纯数字脏值判为无效，避免写进卡片标题（如 6.8万208052）。
                if (group.kind === 'bilibili') {
                    const pickAttr = (el, name) => (el && el.getAttribute && el.getAttribute(name) ? String(el.getAttribute(name)).trim() : '');
                    const titleEl = card && card.querySelector('.bili-video-card__info--tit, [class*="info--tit"], [class*="video-name"], h3');
                    const titleLink = card && card.querySelector('a[href*="/video/"][title]');
                    const coverImg = card && card.querySelector('img[alt]');
                    const candidates = [
                        pickAttr(titleEl, 'title') || (titleEl ? (titleEl.textContent || '').trim() : ''),
                        pickAttr(titleLink, 'title'),
                        pickAttr(coverImg, 'alt'),
                        pickAttr(first.querySelector ? first.querySelector('img[alt]') : null, 'alt'),
                        pickAttr(first, 'title'),
                        title,
                    ];
                    const isStatsLike = (s) => {
                        const trimmed = String(s || '').trim();
                        // 以 mm:ss / h:mm:ss 结尾 = 时长（如 6.8万20108:52、充电专属3.2万9917:21）
                        if (/\d{1,2}:\d{2}(?::\d{2})?$/.test(trimmed)) return true;
                        const stripped = trimmed.replace(/[万亿]/g, '');
                        return /\d/.test(trimmed) && !/[\u4e00-\u9fa5]/.test(stripped);
                    };
                    title = candidates.find((v) => v && v.length > 1 && !isStatsLike(v)) || '';
                }
                let likes = 0;
                if (card) {
                    const likeEl = card.querySelector('.like-wrapper .count, [class*="like"] [class*="count"], [class*="count"]');
                    if (likeEl) likes = parseCount(likeEl.textContent);
                }
                // 小红书把防盗链 token 放在 href 的 query（xsec_token=...）或某个
                // data-xsec-token / data-xsec_token 属性里，且**只在登录态 SSR 渲染时存在**。
                // 必须补到 URL 上，否则打开详情页会落到「你访问的页面不见了」（404）。
                // 用 outerHTML 正则扫描最稳：不管 token 在 href 还是任意属性名里都能抓到。
                const extractXsec = (anchors, card) => {
                    let html = (card ? card.outerHTML || '' : '');
                    for (const a of anchors) {
                        html += a.outerHTML || '';
                    }
                    const m = html.match(/xsec[-_]token["'\s:=]+([^"'&\s>]+)/i);
                    const s = html.match(/xsec[-_]source["'\s:=]+([^"'&\s>]+)/i);
                    return { token: m ? m[1] : null, source: s ? s[1] : '' };
                };
                const xsec = extractXsec(group.anchors, card);
                let url = first.href || first.getAttribute('href') || '';
                if (xsec.token && !url.includes('xsec_token')) {
                    const sep = url.includes('?') ? '&' : '?';
                    url = url + sep + 'xsec_token=' + encodeURIComponent(xsec.token)
                        + (xsec.source ? '&xsec_source=' + encodeURIComponent(xsec.source) : '');
                }
                results.push({ url: url, title: title, likes: likes, platform: group.kind });
                if (results.length >= LIMIT) break;
            }
            return { posts: results, exhausted: exhausted, diag: diag };
        })()
    "#;
    // 把 limit 注入抽取器（用占位符替换而非 format!，避免 JS 的大括号被当成格式串）
    let extractor = extractor.replace("__LIMIT__", &limit.max(1).to_string());
    // await_promise=true：抽取器是 async IIFE（内部要 await 滚动加载）
    let extracted = client.evaluate(&session_id, &extractor, true).await;
    // 抓完关临时 tab + 优雅关 WS：抓取会被反复点，不清理会积累标签页和半开连接，
    // 最终把 Chrome 的 DevTools 服务拖到不稳定（os error 10053 的诱因之一）。
    let _ = client
        .send_request(
            "Target.closeTarget",
            Some(json!({ "targetId": target_id })),
            None,
        )
        .await;
    client.close().await;
    extracted
}

/// B 站补全抽取器：在页面上下文调公开 API，把「合集 / 收藏夹 / 空间全部合集」拉齐。
///
/// 为什么不滚 DOM：合集页是虚拟列表 + 分页，滚动既慢又容易漏；
/// `seasons_archives_list`（100 条/页）、`fav/resource/list`（20 条/页）翻页即可拿全。
/// 返回 `{ posts: [{url,title,likes,platform,publishedAt}], debug }`；
/// 接口失败/未登录时返回空数组，由调用方回退到 DOM 抓取。
const BILI_EXTRAS_EXPRESSION: &str = r#"
(async () => {
    const out = { posts: [], debug: '' };
    // likes 留 0：合集/收藏夹接口只给播放量，点赞要另走 archive/stat 补（见 BILI_STATS_EXPRESSION）。
    const push = (bv, title, view, pubdate) => {
        if (!bv) return;
        out.posts.push({
            url: 'https://www.bilibili.com/video/' + bv,
            title: String(title || '').trim(),
            likes: 0,
            platform: 'bilibili',
            publishedAt: pubdate ? new Date(pubdate * 1000).toISOString().slice(0, 10) : '',
        });
    };
    const getJson = async (p) => {
        try {
            const r = await fetch(p, { credentials: 'include' });
            return await r.json();
        } catch (e) { return null; }
    };
    const u = new URL(location.href);
    const seg = u.pathname.split('/').filter(Boolean);
    const mid = /^\d+$/.test(seg[0] || '') ? seg[0] : '';
    const sid = u.searchParams.get('sid') || (seg[1] === 'lists' ? (seg[2] || '') : '');
    const fid = u.searchParams.get('fid') || u.searchParams.get('media_id') || '';

    // 单个合集：page_size=100，翻到空页或超上限为止
    const fetchSeason = async (seasonId) => {
        for (let pn = 1; pn <= 40; pn++) {
            const r = await getJson('https://api.bilibili.com/x/polymer/web-space/seasons_archives_list?mid=' + mid + '&season_id=' + seasonId + '&page_num=' + pn + '&page_size=100');
            const arr = r && r.code === 0 && r.data && r.data.archives;
            if (!arr || !arr.length) break;
            for (const a of arr) push(a.bvid, a.title, a.stat && a.stat.view, a.pubdate);
            const total = (r.data.page && r.data.page.total) || 0;
            if (arr.length < 100 || pn * 100 >= total) break;
        }
    };
    // 收藏夹：20 条/页
    const fetchFav = async (mediaId) => {
        for (let pn = 1; pn <= 50; pn++) {
            const r = await getJson('https://api.bilibili.com/x/v3/fav/resource/list?media_id=' + mediaId + '&pn=' + pn + '&ps=20&platform=web');
            const arr = r && r.code === 0 && r.data && r.data.medias;
            if (!arr || !arr.length) break;
            for (const m of arr) {
                if (!m || m.type !== 2) continue; // 2 = 视频
                push(m.bvid, m.title, m.cnt_info && m.cnt_info.play, m.pubtime);
            }
            if (arr.length < 20) break;
        }
    };
    // 空间页：枚举该 UP 的全部合集，再逐个拉全
    const fetchAllSeasons = async () => {
        const sids = [];
        for (let pn = 1; pn <= 10; pn++) {
            const r = await getJson('https://api.bilibili.com/x/polymer/web-space/seasons_series_list?mid=' + mid + '&page_num=' + pn + '&page_size=20');
            const box = r && r.code === 0 && r.data && r.data.items_lists;
            const list = box && box.seasons_list;
            if (!list || !list.length) break;
            for (const s of list) {
                const meta = s && s.meta;
                if (meta && meta.season_id) sids.push(meta.season_id);
            }
            const total = (box.page && box.page.total) || 0;
            if (pn * 20 >= total) break;
        }
        for (const s of sids) await fetchSeason(s);
    };

    if (sid) await fetchSeason(sid);
    else if (fid) await fetchFav(fid);
    else if (mid) await fetchAllSeasons();
    out.debug = JSON.stringify({ mid: mid, sid: sid, fid: fid, found: out.posts.length });
    return out;
})()
"#;

/// 抓 B 站合集 / 收藏夹 / 空间全部合集（页面内调公开 API，异步）。
pub async fn scrape_bilibili_extras(ws_url: &str, page_url: &str) -> Result<Value, String> {
    navigate_and_eval_settle(ws_url, page_url, BILI_EXTRAS_EXPRESSION, 2_500, true).await
}

/// B 站互动数补全：批量查 `archive/stat`（列表卡片只有播放量，没有点赞）。
///
/// 在页面上下文里跑（带 cookie），返回 `{ "<BV>": { like, reply, favorite, view } }`。
/// 一次 CDP 调用内部并发 6 路，避免把 B 站风控惹毛（-412）。
const BILI_STATS_EXPRESSION: &str = r#"
(async () => {
    const ids = __BVIDS__;
    const out = {};
    const getJson = async (u) => {
        try {
            const r = await fetch(u, { credentials: 'include' });
            return await r.json();
        } catch (e) { return null; }
    };
    for (let i = 0; i < ids.length; i += 6) {
        const batch = ids.slice(i, i + 6);
        await Promise.all(batch.map(async (bv) => {
            const r = await getJson('https://api.bilibili.com/x/web-interface/archive/stat?bvid=' + bv);
            if (r && r.code === 0 && r.data) {
                out[bv] = {
                    like: r.data.like || 0,
                    reply: r.data.reply || 0,
                    favorite: r.data.favorite || 0,
                    view: r.data.view || 0,
                };
            }
        }));
        await new Promise((res) => setTimeout(res, 120));
    }
    return out;
})()
"#;

/// 批量补 B 站互动数（点赞/评论/收藏）。
pub async fn fetch_bilibili_stats(
    ws_url: &str,
    page_url: &str,
    bvids: &[String],
) -> Result<Value, String> {
    if bvids.is_empty() {
        return Ok(json!({}));
    }
    let ids = serde_json::to_string(bvids).unwrap_or_else(|_| "[]".to_string());
    let expression = BILI_STATS_EXPRESSION.replace("__BVIDS__", &ids);
    navigate_and_eval_settle(ws_url, page_url, &expression, 1_200, true).await
}

/// 补历史专用「深翻」抓取：滚动由 **Rust 侧循环**驱动（而非一次性 JS IIFE），
/// 因此可以每轮上报进度（`monitor:backfill-progress`）。进度分母先用主页声明的作品总数
/// `profileTotal`，翻到底后收尾为**实际可抓总量** `effective_total`
/// （声明数常含私密/已删/审核中而不渲染的条目，直接用声明数会导致进度永远停在 73/87）。
///
/// 与 `scrape_profile_posts` 的差异：
/// - 滚动在 Rust 里每轮 `evaluate` 一小步 —— 中途能发进度事件，前端进度条才会动；
/// - 每轮用「小幅多次滚动 + 滚轮事件 + window 滚动」组合拳，提高触发小红书/抖音
///   懒加载的概率（纯一次性 `scrollTop = scrollHeight` 在部分 CDP 视口下不触发）；
/// - 进页面先 `Emulation.setDeviceMetricsOverride` 给一个较高的视口，让更多卡片进入
///   可视区，IntersectionObserver 懒加载更容易命中；
/// - `limit` 用大值（补历史 200），循环直到翻到底（连续 4 轮无新增）/ 达到主页总数 /
///   或超过上限才停。
pub async fn scrape_profile_posts_paged(
    ws_url: &str,
    profile_url: &str,
    wait_selector: &str,
    limit: usize,
    app: Option<AppHandle>,
    watch_id: &str,
    emit_progress: bool,
) -> Result<Value, String> {
    let client = CdpClient::connect(ws_url).await?;
    let target_id = client.create_target("about:blank").await?;
    let session_id = client.attach_to_target(&target_id).await?;
    // 给临时 tab 一个较高的视口，让更多卡片进入可视区、懒加载更容易触发
    let _ = client
        .send_request(
            "Emulation.setDeviceMetricsOverride",
            Some(json!({
                "width": 1280,
                "height": 1400,
                "deviceScaleFactor": 1,
                "mobile": false
            })),
            Some(&session_id),
        )
        .await;
    let profile_url = strip_modal_id_param(profile_url);
    let nav_params = json!({ "url": profile_url });
    let _ = client
        .send_request("Page.navigate", Some(nav_params), Some(&session_id))
        .await?;
    let _ = client.wait_for_selector(&session_id, wait_selector, 20).await;
    // 等帖子锚点出现后再给动态渲染/图片懒加载 3s 缓冲
    tokio::time::sleep(Duration::from_secs(3)).await;

    // 主页声明的作品总数（小红书「笔记 N」），作为进度条分母；抖音/未知为 0
    let profile_total_js = r#"
        (() => {
            try {
                const text = (document.body && document.body.innerText) || '';
                let m = text.match(/笔记\s*([\d,]+)/);
                if (!m) m = text.match(/作品\s*([\d,]+)/);
                // B 站：视频 tab 的「视频 84」（投稿 105 可能含图文，优先取视频数）
                if (!m) m = text.match(/视频\s*([\d,]+)/);
                if (!m) m = text.match(/投稿\s*([\d,]+)/);
                if (m) return parseInt(String(m[1]).replace(/,/g, ''), 10) || 0;
                // 兜底：XHS SPA 全局状态里的作品数（他人主页也可能藏在这里）
                const st = window.__INITIAL_STATE__;
                if (st && st.user && st.user.userPageData) {
                    const pd = st.user.userPageData;
                    const n = pd.notesCount || pd.noteCount || (pd.user && (pd.user.notesCount || pd.user.noteCount));
                    if (n) return parseInt(String(n), 10) || 0;
                }
            } catch (e) {}
            return 0;
        })()
    "#;
    let profile_total = client
        .evaluate(&session_id, profile_total_js, false)
        .await
        .ok()
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    // 注：每轮实时计数改用「按 id 累计去重集合 seen.len()」（见下方滚动循环），
    // 不再用一次性 count_js，避免虚拟列表下计数失真（30→16 回退）。

    // 滚动一屏：多种手段并用，触发小红书/抖音的懒加载
    // （容器 scrollTop 小幅多次 + window 滚动 + 滚轮事件，模拟真实滚动让 IntersectionObserver 命中更多卡片）
    let scroll_js = r#"
        (() => {
            const moved = { v: false, scroller: '', maxScrollTop: 0, scrollHeight: 0, clientHeight: 0 };
            const CANDIDATE = ['#userPostedFeeds', '.feeds-container', '#noteContainer', '.note-list', '[class*="feeds"]', '[class*="note-list"]', '[class*="works"]', '[class*="post-list"]', '[class*="user-post"]', '[class*="scroll"]'];
            // 锚点限定到抖音作品容器，避免视频弹窗的「相关推荐」干扰滚动容器定位
            const worksRoot = () => document.querySelector('[data-e2e="user-post-list"]') || document;
            // 动态找滚动容器：从小红书笔记卡片向上找第一个 overflow-y 可滚动、内容超高的祖先，
            // 比硬编码 class 列表更稳（XHS class 常变，硬编码容易选错容器 → 页面不动）。
            const findScrollable = (start) => {
                let node = start;
                while (node && node !== document.body && node !== document.documentElement) {
                    const style = getComputedStyle(node);
                    const oy = style.overflowY;
                    if ((oy === 'auto' || oy === 'scroll' || oy === 'overlay') && node.scrollHeight > node.clientHeight + 5) {
                        return node;
                    }
                    node = node.parentElement;
                }
                return null;
            };
            const targets = () => {
                const els = [];
                const anchor = worksRoot().querySelector('a[href*="/explore/"], a[href*="/video/"]');
                const sc = anchor ? findScrollable(anchor) : null;
                if (sc) {
                    els.push(sc);
                    if (!moved.scroller) { moved.scroller = sc.id ? '#' + sc.id : (typeof sc.className === 'string' && sc.className ? '.' + sc.className.split(' ')[0] : sc.tagName); }
                }
                if (document.scrollingElement) els.push(document.scrollingElement);
                if (document.body) els.push(document.body);
                if (document.documentElement) els.push(document.documentElement);
                for (const sel of CANDIDATE) for (const el of Array.from(document.querySelectorAll(sel))) els.push(el);
                const seen = new Set();
                return els.filter((el) => { if (!el || seen.has(el)) return false; seen.add(el); return true; }).slice(0, 12);
            };
            const step = () => {
                for (const el of targets()) {
                    try {
                        const before = el.scrollTop;
                        // 渐进式滚动一屏（而非跳到底）：小红书是虚拟列表，顶部卡片下滑时会被卸载，
                        // 跳到底会跳过中间整段 chunk，导致最终一次性抽取只拿到底部 ~16 条（30→16 回退）。
                        // 每轮只滚约一屏，让下一屏卡片渲染并进入抽取窗口、被 Rust 侧按 id 累计。
                        const inc = Math.max((el.clientHeight || 800), 700);
                        el.scrollTop = before + inc;
                        moved.maxScrollTop = Math.max(moved.maxScrollTop, el.scrollTop);
                        moved.scrollHeight = Math.max(moved.scrollHeight, el.scrollHeight);
                        moved.clientHeight = Math.max(moved.clientHeight, el.clientHeight);
                        if (el.scrollTop > before + 5) {
                            moved.v = true;
                            if (!moved.scroller) { const c = (typeof el.className === 'string' && el.className) ? el.className.split(' ')[0] : ''; moved.scroller = el.id ? '#' + el.id : (c ? '.' + c : el.tagName); }
                        }
                    } catch (e) {}
                }
                try { window.scrollTo(0, (document.scrollingElement ? document.scrollingElement.scrollTop : 0) + 700); } catch (e) {}
                // 真实滚轮事件打在滚动容器上：触发依赖 wheel/scroll 事件的懒加载
                try { const ev = new WheelEvent('wheel', { deltaY: 1200, bubbles: true, cancelable: true }); (targets()[0] || document.scrollingElement || document.body || document.documentElement).dispatchEvent(ev); } catch (e) {}
            };
            // 每轮滚动一屏（渐进式），留时间让懒加载请求发出
            step();
            return new Promise((res) => setTimeout(() => res(moved), 400));
        })()
    "#;

    // 最终抽取（只读当前 DOM，按 note id 分组解析标题/点赞/xsec，与 scrape_profile_posts 一致）
    let extract_js = r#"
        (() => {
            const parseCount = (text) => {
                if (!text) return 0;
                const t = String(text).trim().replace(/,/g, '');
                const m = t.match(/^([\d.]+)\s*(万|w|W|k|K)?$/);
                if (!m) return 0;
                const n = parseFloat(m[1]); if (!isFinite(n)) return 0;
                const u = m[2] || '';
                if (u === '万' || u === 'w' || u === 'W') return Math.round(n * 10000);
                if (u === 'k' || u === 'K') return Math.round(n * 1000);
                return Math.round(n);
            };
            const idOf = (href) => {
                const me = href.match(/\/explore\/([A-Za-z0-9]+)/);
                // B 站视频是 /video/BV1xx…（字母开头），抖音是 /video/<纯数字>，先判 BV。
                const mb = href.match(/\/video\/(BV[0-9A-Za-z]+)/);
                const mv = href.match(/\/video\/(\d+)/);
                if (me) return { id: me[1], kind: 'xiaohongshu' };
                if (mb) return { id: mb[1], kind: 'bilibili' };
                if (mv) return { id: mv[1], kind: 'douyin' };
                return null;
            };
            const LIMIT = __LIMIT__;
            // 标题清洗：抖音卡片的 textContent 常拼成「置顶3.7万<正文><正文>」——
            // 开头是「置顶」徽标 + 点赞数（点赞数是独立子元素，取 textContent 时被粘到最前），
            // 末尾还会把同一段文案重复一遍（aria-label 与视觉文本各一份）。
            // 「数字后紧跟这些量词」时数字多半属于正文（如「10个技巧」），不剥。
            const UNIT_HEAD = /^[个种条步招篇天年月日岁次位项秒分]/;
            const dedupeTail = (s) => {
                for (let i = Math.ceil(s.length / 2); i < s.length; i++) {
                    const tail = s.slice(i);
                    if (tail.length >= 8 && s.startsWith(tail)) return s.slice(0, i).trim();
                }
                return s;
            };
            const cleanTitle = (raw) => {
                let t = String(raw || '').replace(/\s+/g, ' ').trim();
                if (!t) return '';
                t = t.replace(/^(?:置顶|置頂)\s*/, '');
                // 顺序要求：**先剥前导点赞数再做重复检测** —— 否则前缀带着「667」，
                // 与尾部重复段对不上，永远查不到重复。
                const m = t.match(/^[\d.]+\s*(?:万|w|W|k|K)?\s*/);
                if (m) {
                    const rest = t.slice(m[0].length).trim();
                    if (rest.length >= 6 && !UNIT_HEAD.test(rest)) t = rest;
                }
                return dedupeTail(t).trim();
            };
            // 锚点限定到抖音作品容器（存在才限定），避免弹窗「相关推荐」混入其他博主视频
            const worksRoot = () => document.querySelector('[data-e2e="user-post-list"]') || document;
            const anchors = Array.from(worksRoot().querySelectorAll('a[href*="/explore/"], a[href*="/video/"]'));
            const groups = new Map();
            for (const a of anchors) {
                const info = idOf(a.getAttribute('href') || '');
                if (!info) continue;
                if (!groups.has(info.id)) groups.set(info.id, { kind: info.kind, anchors: [] });
                groups.get(info.id).anchors.push(a);
            }
            const results = [];
            for (const [id, group] of groups) {
                const first = group.anchors[0];
                let card = first.closest('section.note-item, [class*="note-item"], [class*="note-card"], li, article') || first.parentElement;
                if (group.kind === 'bilibili') {
                    // 同 scrape_profile_posts：封面 <a> 自身 class 含 bili-video-card 子串，
                    // 必须向上找真正含标题节点的祖先，否则标题会变成「播放量+弹幕+时长」。
                    let node = first;
                    for (let i = 0; i < 6 && node; i++, node = node.parentElement) {
                        if (node.querySelector && node.querySelector('[class*="info--tit"], h3')) { card = node; break; }
                    }
                }
                let title = '';
                for (const a of group.anchors) { if (/title/i.test(a.className || '')) { title = (a.getAttribute('title') || a.textContent || '').trim(); break; } }
                if (!title && card) { const t = card.querySelector('[class*="title"]'); if (t) title = (t.getAttribute('title') || t.textContent || '').trim(); }
                if (!title) title = (first.textContent || '').trim();
                // 抖音：优先取结构化描述节点（不含置顶徽标/点赞数），再按清洗规则兜底
                if (group.kind === 'douyin') {
                    const scope = first.closest('[data-e2e="user-post-item"], li, section, article') || first.parentElement;
                    const descEl = scope && scope.querySelector('[data-e2e="user-post-item-desc"], [data-e2e="video-desc"]');
                    const desc = descEl ? (descEl.textContent || '').trim() : '';
                    title = cleanTitle(desc || title);
                }
                // B 站：多候选 + 脏值过滤（同 scrape_profile_posts，改版后 title 节点可能换 class）
                if (group.kind === 'bilibili') {
                    const pickAttr = (el, name) => (el && el.getAttribute && el.getAttribute(name) ? String(el.getAttribute(name)).trim() : '');
                    const titleEl = card && card.querySelector('.bili-video-card__info--tit, [class*="info--tit"], [class*="video-name"], h3');
                    const titleLink = card && card.querySelector('a[href*="/video/"][title]');
                    const coverImg = card && card.querySelector('img[alt]');
                    const candidates = [
                        pickAttr(titleEl, 'title') || (titleEl ? (titleEl.textContent || '').trim() : ''),
                        pickAttr(titleLink, 'title'),
                        pickAttr(coverImg, 'alt'),
                        pickAttr(first.querySelector ? first.querySelector('img[alt]') : null, 'alt'),
                        pickAttr(first, 'title'),
                        title,
                    ];
                    const isStatsLike = (s) => {
                        const trimmed = String(s || '').trim();
                        // 以 mm:ss / h:mm:ss 结尾 = 时长（如 6.8万20108:52、充电专属3.2万9917:21）
                        if (/\d{1,2}:\d{2}(?::\d{2})?$/.test(trimmed)) return true;
                        const stripped = trimmed.replace(/[万亿]/g, '');
                        return /\d/.test(trimmed) && !/[\u4e00-\u9fa5]/.test(stripped);
                    };
                    title = candidates.find((v) => v && v.length > 1 && !isStatsLike(v)) || '';
                }
                let likes = 0;
                if (card) { const likeEl = card.querySelector('.like-wrapper .count, [class*="like"] [class*="count"], [class*="count"]'); if (likeEl) likes = parseCount(likeEl.textContent); }
                const extractXsec = (anchors, card) => {
                    let html = (card ? card.outerHTML || '' : '');
                    for (const a of anchors) html += a.outerHTML || '';
                    const m = html.match(/xsec[-_]token["'\s:=]+([^"'&\s>]+)/i);
                    const s = html.match(/xsec[-_]source["'\s:=]+([^"'&\s>]+)/i);
                    return { token: m ? m[1] : null, source: s ? s[1] : '' };
                };
                const xsec = extractXsec(group.anchors, card);
                let url = first.href || first.getAttribute('href') || '';
                if (xsec.token && !url.includes('xsec_token')) {
                    const sep = url.includes('?') ? '&' : '?';
                    url = url + sep + 'xsec_token=' + encodeURIComponent(xsec.token) + (xsec.source ? '&xsec_source=' + encodeURIComponent(xsec.source) : '');
                }
                results.push({ id: id, url: url, title: title, likes: likes, platform: group.kind });
                if (results.length >= LIMIT) break;
            }
            return results;
        })()
    "#;
    let extract_js = extract_js.replace("__LIMIT__", "600"); // 每轮窗口安全上限；真实去重在 Rust 侧累计

    // —— Rust 驱动的滚动循环：每轮「抽取当前窗口并并入累计集合 → 滚动一屏 → 静置」——
    // 核心修复：不再依赖「最后一次性抽取」。小红书是虚拟列表，顶部卡片下滑时会被卸载，
    // 一次性抽取只看到底部 ~16 条（30→16 回退）。改为把每一轮 DOM 窗口解析到的笔记按 id
    // 去重累计到 seen；配合渐进式滚动，每张卡片都会在其还挂在 DOM 的轮次里被捕获。
    // 最终返回 seen（去重并集），前端进度条用 seen.len() 逼近 profileTotal。
    let mut seen: HashMap<String, Value> = HashMap::new();
    let mut last_captured = 0i64;
    let mut stagnant = 0i64;
    let mut exhausted = false;
    let mut hit_limit = false; // 是否因触到 limit 兜底上限而停（而非真的翻到底）
    let mut max_window = 0i64; // 单轮窗口峰值（诊断用）
    let rounds = std::cmp::max(
        18usize,
        std::cmp::min(70usize, ((limit.max(1) + 3) / 3) as usize),
    );
    for _ in 0..rounds {
        // 1) 抽取当前 DOM 窗口，按 id 并入累计集合（跨轮累计，避免 30→16 回退）
        if let Ok(val) = client.evaluate(&session_id, &extract_js, false).await {
            if let Some(arr) = val.as_array() {
                for item in arr {
                    if let Some(id) = item.get("id").and_then(|v| v.as_str()) {
                        seen.insert(id.to_string(), item.clone());
                    }
                }
            }
        }
        let captured = seen.len() as i64;
        max_window = max_window.max(captured);
        // 2) 实时进度：用累计去重数量（逼近真实总量），前端据此刷新「已抓 X / 共 Y」
        if emit_progress {
            if let Some(app_handle) = &app {
                let _ = app_handle.emit(
                    "monitor:backfill-progress",
                    json!({
                        "watchId": watch_id,
                        "scanned": captured,
                        "total": profile_total,
                        "phase": "scrolling"
                    }),
                );
            }
        }
        // 3) 早停：达到 limit（兜底上限）或已知主页总数
        if captured >= limit as i64 {
            exhausted = true;
            hit_limit = true;
            break;
        }
        if profile_total > 0 && captured >= profile_total {
            exhausted = true;
            break;
        }
        // 4) 累计去重数量连续 stagnant 轮无新增 → 已翻到底
        if captured == last_captured {
            stagnant += 1;
            if stagnant >= 5 {
                exhausted = true;
                break;
            }
        } else {
            stagnant = 0;
            last_captured = captured;
        }
        // 5) 渐进式滚动一屏
        let _ = client.evaluate(&session_id, scroll_js, true).await;
        tokio::time::sleep(Duration::from_millis(1300)).await;
    }

    let posts: Vec<Value> = seen.values().cloned().collect();
    let captured = posts.len() as i64;
    // 进度分母修正：主页声明的「作品 N」常含私密/已删/审核中而不渲染的条目（实测 87 声明只抓到 73），
    // 若一直用 profile_total 当分母，进度会永远停在 73/87。这里按实际可抓量收尾：
    // - 真的翻到底（stagnant / 达到声明数）→ 分母 = captured（进度 100%）
    // - 只是撞到 limit 兜底上限 → 分母 = max(声明数, captured)（说明还有更多没翻）
    let effective_total = if exhausted && !hit_limit {
        captured
    } else {
        std::cmp::max(profile_total, captured)
    };
    if emit_progress {
        if let Some(app_handle) = &app {
            let _ = app_handle.emit(
                "monitor:backfill-progress",
                json!({
                    "watchId": watch_id,
                    "scanned": captured,
                    "total": effective_total,
                    "phase": "scrolling-done"
                }),
            );
        }
    }
    let diag = json!({
        "rounds": rounds,
        "captured": posts.len(),
        "maxWindow": max_window,
        "profileTotal": profile_total,
        "scrolled": max_window > (limit.min(30) as i64)
    });
    let _ = client
        .send_request(
            "Target.closeTarget",
            Some(json!({ "targetId": target_id })),
            None,
        )
        .await;
    client.close().await;
    Ok(json!({
        "posts": posts,
        "exhausted": exhausted,
        "diag": diag,
        "profileTotal": profile_total
    }))
}

/// 通过已登录的 Chrome（CDP）抓取**单篇帖子详情**（正文/图片/互动数据），供本地详情弹窗显示。
///
/// 与 `scrape_profile_posts` 的区别：目标是帖子详情页（小红书 `/explore/<id>`、抖音 `/video/<id>`），
/// 抽取的是正文全文 + 图片列表，而不是帖子列表。抓完即用 `Target.closeTarget` 关掉临时标签页，
/// 避免用户每点一次详情就攒一个 tab。
/// 判定一次详情抓取结果是否「没抓到正文」：ok:false（整页没解析出来），
/// 或 ok:true 但 content 为空（页面被**半渲染**捕获——标题元素已在、正文 desc 还没挂载）。
fn detail_missing_content(value: &Value) -> bool {
    let ok = value.get("ok").and_then(Value::as_bool).unwrap_or(false);
    if !ok {
        return true;
    }
    value
        .get("content")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("")
        .is_empty()
}

pub async fn scrape_post_detail(
    ws_url: &str,
    post_url: &str,
    platform: &str,
) -> Result<Value, String> {
    match scrape_post_detail_once(ws_url, post_url, platform).await {
        Ok(value) => {
            if !detail_missing_content(&value) {
                return Ok(value);
            }
            // 半渲染/空页兜底：等 1.2s 重开 tab 重抓一次。命中时用户省掉一次
            // 手动「重新抓正文」；没命中（如真 404）就退回第一次结果（带诊断信息）。
            tokio::time::sleep(Duration::from_millis(1200)).await;
            match scrape_post_detail_once(ws_url, post_url, platform).await {
                Ok(retry) if !detail_missing_content(&retry) => Ok(retry),
                _ => Ok(value),
            }
        }
        Err(first_error) => {
            // Windows 上 Chrome 偶发掐断 DevTools 连接（os error 10053/10054）：
            // 等 600ms 用全新连接重试一次，大多数瞬时断连都能恢复。
            tokio::time::sleep(Duration::from_millis(600)).await;
            scrape_post_detail_once(ws_url, post_url, platform)
                .await
                .map_err(|second_error| format!("{first_error}；重试仍失败: {second_error}"))
        }
    }
}

async fn scrape_post_detail_once(
    ws_url: &str,
    post_url: &str,
    platform: &str,
) -> Result<Value, String> {
    let client = CdpClient::connect(ws_url).await?;
    let target_id = client.create_target("about:blank").await?;
    let result = async {
        let session_id = client.attach_to_target(&target_id).await?;
        let nav_params = json!({ "url": post_url });
        let _ = client
            .send_request("Page.navigate", Some(nav_params), Some(&session_id))
            .await?;
        // 等详情锚点出现（xhs: #detail-title/#detail-desc；dy: 视频文案区）。多 selector 用逗号列表。
        let wait_selector = crate::scraper::adapter_for_str(platform)
            .map(|adapter| adapter.detail_wait_selector())
            .unwrap_or("#detail-title, #detail-desc, .note-content");
        let _ = client
            .wait_for_selector(&session_id, wait_selector, 10)
            .await;
        // 多图笔记（如 18 张）图片是懒加载的：先分步滚到底部触发所有图片进入 DOM/发起加载，
        // 否则只抓到首屏若干张。窗口与常见滚动容器都滚一遍。
        let _ = client
            .evaluate(
                &session_id,
                r#"(() => {
                    const cb = () => {
                        window.scrollBy(0, 800);
                        document.querySelectorAll('.note-content, .detail-container, .side-container, [class*="scroll"], [class*="noteSlide"]').forEach((c) => { try { c.scrollTop = c.scrollHeight; } catch (e) {} });
                    };
                    for (let i = 0; i < 16; i++) cb();
                    window.scrollTo(0, 0);
                    return true;
                })()"#,
                false,
            )
            .await;
        tokio::time::sleep(Duration::from_millis(800)).await;
        // 等正文文本真正填进容器（xhs 客户端渲染有延迟：标题先出、正文后填），
        // 避免抓到「元素在但 textContent 空」的半渲染页 → 触发「页面未解析到正文」。
        let content_wait = crate::scraper::adapter_for_str(platform)
            .map(|adapter| adapter.content_wait_selector())
            .unwrap_or("#detail-desc, .note-text, .desc");
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let has = client
                .evaluate(
                    &session_id,
                    &format!(
                        r#"(() => {{ const el = document.querySelector({sel}); return el ? (el.textContent || '').trim().length > 0 : false; }})()"#,
                        sel = serde_json::to_string(content_wait).unwrap_or_default()
                    ),
                    false,
                )
                .await?;
            let ready = has
                .get("result")
                .and_then(|r| r.get("value"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if ready || std::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;

        // 通用详情抽取：先判平台（xhs 详情页有 #detail-title / .note-content），
        // 再按各自的 DOM 取标题/正文/作者/时间/互动数/图片。计数解析复用中文单位逻辑。
        let extractor = r#"
            (() => {
                const txt = (sel) => {
                    const el = document.querySelector(sel);
                    return el ? (el.textContent || '').trim() : '';
                };
                const num = (t) => {
                    if (!t) return 0;
                    const s = String(t).trim().replace(/,/g, '');
                    const m = s.match(/^([\d.]+)\s*(万|w|W|k|K)?$/);
                    if (!m) return 0;
                    const n = parseFloat(m[1]);
                    if (!isFinite(n)) return 0;
                    const u = m[2] || '';
                    if (u === '万' || u === 'w' || u === 'W') return Math.round(n * 10000);
                    if (u === 'k' || u === 'K') return Math.round(n * 1000);
                    return Math.round(n);
                };
                const isBili = /bilibili\.com/i.test(location.hostname);
                const isXhs = !isBili && !!document.querySelector('#detail-title, .note-content, #detail-desc');
                let title = '', content = '', author = '', date = '';
                let likes = 0, collects = 0, comments = 0;
                let mediaType = 'image', cover = '';
                const images = [];
                if (isBili) {
                    mediaType = 'video';
                    // B 站视频页自带 SSR 状态：__INITIAL_STATE__.videoData 里有标题/简介/UP 主/发布时间/统计。
                    let vd = null;
                    try { vd = window.__INITIAL_STATE__ && window.__INITIAL_STATE__.videoData; } catch (e) {}
                    if (vd) {
                        title = String(vd.title || '').trim();
                        content = String(vd.desc || '').trim();
                        author = String((vd.owner && vd.owner.name) || '').trim();
                        const st = vd.stat || {};
                        // 与其它平台统一口径：likes = 点赞数（不是播放量）。
                        likes = Number(st.like || 0);
                        comments = Number(st.reply || 0);
                        collects = Number(st.favorite || 0);
                        if (vd.pubdate) {
                            const d = new Date(vd.pubdate * 1000);
                            if (!isNaN(d.getTime())) date = d.toISOString().slice(0, 10);
                        }
                        cover = String(vd.pic || '');
                        // B 站封面常是协议相对地址（//i0.hdslb.com/...），补成 https 再落库
                        if (cover.indexOf('//') === 0) cover = 'https:' + cover;
                        if (cover) images.push(cover);
                    }
                    if (!title) {
                        title = txt('h1.video-title') || txt('.video-title');
                        if (!title) {
                            const m = document.querySelector('meta[property="og:title"]');
                            if (m) title = (m.getAttribute('content') || '').trim();
                        }
                    }
                    if (!content) content = txt('.desc-info-text');
                    if (!author) author = txt('.up-name') || txt('.up-info--name a');
                    if (!date) date = txt('[class*="pubdate"]');
                } else if (isXhs) {
                    title = txt('#detail-title') || txt('.title');
                    content = txt('#detail-desc') || txt('.note-text') || txt('.desc');
                    author = txt('.author .name') || txt('.username');
                    date = txt('.date');
                    likes = num(txt('.like-wrapper .count'));
                    collects = num(txt('.collect-wrapper .count'));
                    comments = num(txt('.chat-wrapper .count'));
                    // SSR 数据源兜底：XHS 详情页是 SSR，window.__INITIAL_STATE__.note.noteDetailMap
                    // 挂着完整笔记（desc/title/type/imageList），不受客户端渲染时序影响——
                    // DOM 半渲染（标题在、正文还没填）时从这里补齐，正文不再为空，
                    // 缓存（以正文为钥匙）才能生效，否则每次打开弹窗都会重抓。
                    let ssrNote = null;
                    try {
                        const st = window.__INITIAL_STATE__;
                        const map = st && st.note && st.note.noteDetailMap;
                        if (map) { for (const k in map) { const n = map[k] && map[k].note; if (n && (n.desc || n.title)) { ssrNote = n; break; } } }
                    } catch (e) {}
                    if (ssrNote) {
                        if (!title) title = String(ssrNote.title || '').trim();
                        if (!content) content = String(ssrNote.desc || '').trim();
                        mediaType = String(ssrNote.type || '') === 'video' ? 'video' : 'image';
                    } else if (document.querySelector('.media-container video, .swiper-slide video, .note-slider video, .player-container video')) {
                        // 无 SSR 时的保守视频判定：只认主媒体容器里的 <video>，
                        // 不再全页扫（推荐卡片里的播放图标/视频会误标「视频」徽标）
                        mediaType = 'video';
                    }
                    const seen = new Set();
                    // 去重按「去 query 的 path」：同一张图不同尺寸参数（?imageView2/w=..）算同一张
                    const keyOf = (u) => u.replace(/^https?:/, '').split('?')[0];
                    const pushImg = (u) => {
                        if (!u) return;
                        if (u.startsWith('//')) u = 'https:' + u;
                        if (!u.startsWith('http')) return;
                        if (/avatar|header|emoji|icon|zoom|search|magnif|sprite|logo|badge|\.svg/i.test(u)) return;
                        const key = keyOf(u);
                        if (seen.has(key)) return;
                        seen.add(key);
                        if (!cover) cover = u;
                        images.push(u);
                    };
                    document.querySelectorAll('.media-container img, .swiper-slide img, .note-slider img, .note-content img, #detail-desc img, .carousel img, [class*="noteSlide"] img, [class*="image"] img').forEach((img) => {
                        // UI 图标护栏（放大镜/以图搜图等悬浮图标）：
                        // 1) 在悬浮工具类容器里的一律跳过；2) 已加载但固有尺寸过小的跳过。
                        if (img.closest('[class*="zoom"], [class*="search"], [class*="operate"], [class*="toolbar"], [class*="interaction"]')) return;
                        const w = img.naturalWidth || img.width || 0;
                        if (w > 0 && w < 120) return;
                        pushImg(img.getAttribute('src') || img.getAttribute('data-src') || img.getAttribute('data-original') || img.getAttribute('data-actualsrc') || '');
                    });
                    // SSR imageList 补齐：DOM 懒加载没唤出的图从这里补全
                    try {
                        if (ssrNote && Array.isArray(ssrNote.imageList)) {
                            ssrNote.imageList.forEach((it) => {
                                pushImg((it && (it.urlDefault || it.url || it.original)) || '');
                            });
                        }
                    } catch (e) {}
                } else {
                    mediaType = 'video'; // 抖音详情页是视频平台
                    title = txt('[data-e2e="video-desc"]') || txt('.video-info-detail') || txt('h1');
                    content = title;
                    // 作者昵称：**优先级** SSR(author.nickname) → 详情页限定作用域的 DOM → 裸选择器。
                    // 之前把 `a[href*="/user/"] span` 放进裸候选，会命中外层导航「我的」，导致作者变成「我的」。
                    // 最后再加导航词护栏，任何「我的/首页/朋友…」一律丢弃。
                    author = '';
                    const pickAuthor = (o) => {
                        if (!o || typeof o !== 'object') return '';
                        const a = o.author || o.authorInfo || null;
                        return (a && (a.nickname || a.name)) || o.nickname || o.userName || '';
                    };
                    try {
                        const im = window.__INITIAL_STATE__ && window.__INITIAL_STATE__.itemModule;
                        if (im) {
                            const vid = (location.pathname.match(/\/video\/(\d+)/) || [])[1];
                            // 只按当前视频 id 取；不再 Object.values(im)[0]（会取到推荐位的其他作者）
                            const it = (vid && im[vid]) || null;
                            if (it) author = String(pickAuthor(it) || '').trim();
                        }
                    } catch (e) {}
                    if (!author) {
                        try {
                            const rd = window._ROUTER_DATA;
                            const ld = rd && (rd.loaderData || rd.loader_data);
                            if (ld) {
                                for (const k in ld) {
                                    const arr = ld[k] && ld[k].videoInfoRes && ld[k].videoInfoRes.item_list;
                                    const first = arr && arr[0];
                                    if (first) { author = String(pickAuthor(first) || '').trim(); if (author) break; }
                                }
                            }
                        } catch (e) {}
                    }
                    if (!author) {
                        author = txt('[data-e2e="author-name"]')
                            || txt('[data-e2e="video-author-name"]')
                            || txt('.author-name')
                            || txt('[data-e2e="user-name"]')
                            || txt('.account-name');
                    }
                    if (!author) {
                        // 只在「视频信息区」内找作者主页链接，避开侧栏/顶栏导航
                        const scoped = document.querySelectorAll(
                            '[data-e2e="video-detail"] a[href*="/user/"] span, '
                            + '.video-info-detail a[href*="/user/"] span, '
                            + '[data-e2e="video-info"] a[href*="/user/"] span'
                        );
                        for (const el of scoped) {
                            const t = (el.textContent || '').trim();
                            if (t) { author = t; break; }
                        }
                    }
                    // 去掉昵称尾部「· 关注」等装饰
                    if (author) author = author.replace(/[·\s]+$/g, '').trim();
                    // 导航词护栏：命中即判为误抓，置空（交给上层回退平台名，不落库成垃圾作者）
                    if (author && /^(我的|我的主页|首页|朋友|关注|推荐|搜索|消息|我|通知|私信|发布|登录|抖音|精选|商城|团购|直播)$/.test(author)) {
                        author = '';
                    }
                    // 发布时间：现版抖音 DOM 已不再用 `video-create-time`/`.create-time`（恒空），
                    // 改为多候选 + SSR `createTime` 兜底（见下方 formatDate）。
                    date = txt('[data-e2e="video-publish-time"]') || txt('.video-publish-time')
                        || txt('[data-e2e="video-create-time"]') || txt('.create-time');
                    // 互动数：旧 `.count` / `.digg-count` 选择器在现版抖音 DOM 已失效（常全为 0）。
                    // 三道兜底：① 动作按钮包裹节点文本（图标是 svg，text≈计数）；
                    // ② 更宽的 data-e2e 候选（feed-* 命名）；③ __INITIAL_STATE__ / _ROUTER_DATA。
                    // 解析容忍「1.2万」等单位与前后杂文本（取第一个数字）。
                    const numLoose = (t) => {
                        if (t == null) return 0;
                        const m = String(t).replace(/,/g, '').match(/([\d.]+)\s*(万|w|W|k|K)?/);
                        if (!m) return 0;
                        const n = parseFloat(m[1]);
                        if (!isFinite(n)) return 0;
                        const u = m[2] || '';
                        if (u === '万' || u === 'w' || u === 'W') return Math.round(n * 10000);
                        if (u === 'k' || u === 'K') return Math.round(n * 1000);
                        return Math.round(n);
                    };
                    likes = num(txt('[data-e2e="video-player-digg"] .count') || txt('.digg-count'))
                        || numLoose(txt('[data-e2e="video-player-digg"]'))
                        || numLoose(txt('[data-e2e="video-player-digg-count"]'))
                        || numLoose(txt('[data-e2e="feed-digg-icon"]'));
                    collects = num(txt('[data-e2e="video-player-collect"] .count') || txt('.collect-count'))
                        || numLoose(txt('[data-e2e="video-player-collect"]'))
                        || numLoose(txt('[data-e2e="video-player-collect-count"]'))
                        || numLoose(txt('[data-e2e="feed-collect-icon"]'));
                    // 评论数此前恒 0 的根因：守卫写成「三者全为 0」才走 SSR 兜底，而点赞几乎总能
                    // 从按钮文本取到 → 整块跳过 → 评论永远是 0。改成「任一为 0」就补。
                    comments = num(txt('[data-e2e="video-player-comment"] .count') || txt('.comment-count'))
                        || numLoose(txt('[data-e2e="video-player-comment"]'))
                        || numLoose(txt('[data-e2e="video-player-comment-count"]'))
                        || numLoose(txt('[data-e2e="feed-comment-icon"]'))
                        || numLoose(txt('[data-e2e="comment-icon"]'));
                    if (!likes || !collects || !comments) {
                        let stats = null;
                        const deepFind = (obj, depth, budget) => {
                            if (!obj || depth > 6 || (budget.n--) <= 0 || typeof obj !== 'object') return null;
                            if (('digg_count' in obj) || ('diggCount' in obj)) return obj;
                            for (const k in obj) {
                                try { const r = deepFind(obj[k], depth + 1, budget); if (r) return r; } catch (e) {}
                            }
                            return null;
                        };
                        try {
                            const im = window.__INITIAL_STATE__ && window.__INITIAL_STATE__.itemModule;
                            if (im) {
                                const vid = (location.pathname.match(/\/video\/(\d+)/) || [])[1];
                                const it = im[vid] || Object.values(im)[0];
                                if (it) stats = it.stats || it.statistics || null;
                            }
                        } catch (e) {}
                        if (!stats) stats = deepFind(window.__INITIAL_STATE__, 0, { n: 20000 });
                        if (!stats) stats = deepFind(window._ROUTER_DATA, 0, { n: 20000 });
                        const pickStat = (o, keys) => {
                            if (!o) return 0;
                            for (const k of keys) {
                                const v = o[k];
                                if (v != null && v !== '') { const n = Number(v); if (isFinite(n)) return Math.round(n); }
                            }
                            return 0;
                        };
                        if (stats) {
                            likes = likes || pickStat(stats, ['digg_count', 'diggCount']);
                            collects = collects || pickStat(stats, ['collect_count', 'collectCount']);
                            comments = comments || pickStat(stats, ['comment_count', 'commentCount']);
                        }
                    }
                    // 发布时间 SSR 兜底：DOM 选择器全失效时用 create_time（unix 秒）转 YYYY-MM-DD。
                    if (!date) {
                        const formatDate = (ts) => {
                            const n = Number(ts);
                            if (!isFinite(n) || n <= 0) return '';
                            const d = new Date(n > 1e12 ? n : n * 1000);
                            if (isNaN(d.getTime())) return '';
                            const p = (x) => String(x).padStart(2, '0');
                            return d.getFullYear() + '-' + p(d.getMonth() + 1) + '-' + p(d.getDate());
                        };
                        let createTs = 0;
                        try {
                            const im = window.__INITIAL_STATE__ && window.__INITIAL_STATE__.itemModule;
                            if (im) {
                                const vid = (location.pathname.match(/\/video\/(\d+)/) || [])[1];
                                const it = im[vid] || Object.values(im)[0];
                                if (it) createTs = Number(it.createTime || it.create_time || 0) || 0;
                            }
                        } catch (e) {}
                        if (!createTs) {
                            try {
                                const rd = window._ROUTER_DATA;
                                const ld = rd && (rd.loaderData || rd.loader_data);
                                if (ld) {
                                    for (const k in ld) {
                                        const arr = ld[k] && ld[k].videoInfoRes && ld[k].videoInfoRes.item_list;
                                        const first = arr && arr[0];
                                        if (first) {
                                            createTs = Number(first.create_time || first.createTime || 0) || 0;
                                            if (createTs) break;
                                        }
                                    }
                                }
                            } catch (e) {}
                        }
                        if (createTs) date = formatDate(createTs);
                    }
                    // 抖音封面：video 的 poster、播放器占位图、og:image 兜底
                    const v = document.querySelector('video[poster]');
                    if (v && v.getAttribute('poster')) cover = v.getAttribute('poster');
                    if (!cover) { const p = document.querySelector('.xgplayer-poster img'); if (p) cover = p.getAttribute('src') || ''; }
                    if (!cover) { const og = document.querySelector('meta[property="og:image"]'); if (og) cover = og.getAttribute('content') || ''; }
                    if (cover && cover.startsWith('//')) cover = 'https:' + cover;
                }
                if (!title && !content) {
                    // 兜底 1：xhs 详情页是 SSR，即使客户端渲染被拦/选择器改版，
                    // <meta property="og:title"> 与 og:description 里通常仍有标题+正文。
                    const ogT = document.querySelector('meta[property="og:title"]');
                    const ogD = document.querySelector('meta[property="og:description"]')
                        || document.querySelector('meta[name="description"]');
                    let ot = ogT ? (ogT.getAttribute('content') || '').trim() : '';
                    let oc = ogD ? (ogD.getAttribute('content') || '').trim() : '';
                    if (oc && ot && oc.startsWith(ot)) oc = oc.slice(ot.length).replace(/^[\s,，、:：]*/, '').trim();
                    if (ot || oc) {
                        return { ok: true, title: ot, content: oc || ot, author: '', date: '',
                                 likes: 0, collects: 0, comments: 0,
                                 mediaType: document.querySelector('video') ? 'video' : 'image',
                                 cover: (document.querySelector('meta[property="og:image"]') || { getAttribute: () => '' }).getAttribute('content') || '',
                                 images: [], viaMeta: true };
                    }
                    // 兜底 2：彻底解析失败 → 回传落地页信息，便于诊断（跳到了哪个页/被什么拦住）。
                    return { ok: false, pageUrl: location.href, pageTitle: document.title,
                             snippet: document.body ? (document.body.innerText || '').replace(/\s+/g, ' ').slice(0, 120) : '' };
                }
                // 诊断：标题抓到但正文仍空 → 记录各候选元素是否存在/文本长度，随 note 透出，
                // 下次再遇到「页面未解析到正文」能直接看出是选择器失效还是渲染问题。
                let contentDebug = '';
                if (title && !content) {
                    contentDebug = ['#detail-desc', '.note-text', '.desc', '[class*="desc"]'].map((sel) => {
                        const el = document.querySelector(sel);
                        return sel + '=' + (el ? (el.textContent || '').trim().length : 'missing');
                    }).join(', ');
                }
                return { ok: true, title: title, content: content, author: author, date: date,
                         likes: likes, collects: collects, comments: comments,
                         mediaType: mediaType, cover: cover, images: images, contentDebug: contentDebug };
            })()
        "#;
        client.evaluate(&session_id, extractor, false).await
    }
    .await;
    // 无论成败都关临时 tab（浏览器级命令，无 session），再优雅关闭 WS。
    // 不关的话：每点一次详情攒一个 tab + 一条半开连接，Chrome 会越来越不稳定。
    let _ = client
        .send_request(
            "Target.closeTarget",
            Some(json!({ "targetId": target_id })),
            None,
        )
        .await;
    client.close().await;
    result
}

/// 主动停止所有平台的 Chrome 进程 + 清理会话记录。供 UI「退出登录」使用。
pub fn shutdown_chrome() {
    // ws_url 缓存与端口强绑定，关掉 Chrome 后整体失效，整表清空最稳。
    if let Some(cache) = WS_URL_CACHE.get() {
        cache.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clear();
    }
    let mut sessions = CHROME_SESSIONS.get_or_init(Default::default).lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    for (_, session) in sessions.drain() {
        if let Some(mut child) = session.child {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// detect_chrome_path 必须能在当前平台找到已安装的浏览器。
    ///
    /// 回归背景：原实现只列 Windows 路径 + PATH 里找 "chrome"/"msedge"，
    /// macOS 上 Chrome 装在 /Applications/*.app/Contents/MacOS/ 且不在 PATH，
    /// 于是恒返回 None → chromeInstalled=false → 前端「打开 Chrome 登录小红书」
    /// 按钮永久 disabled（点了没反应）。
    #[test]
    fn detect_chrome_path_finds_browser_on_current_platform() {
        // 只在确实装了浏览器的机器上断言；CI 或干净容器里跳过。
        let has_any = [
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
            "/usr/bin/google-chrome",
            "/usr/bin/google-chrome-stable",
            "/usr/bin/chromium",
            "/usr/bin/chromium-browser",
            "/usr/bin/microsoft-edge",
            "/usr/bin/brave-browser",
        ]
        .iter()
        .any(|p| PathBuf::from(p).exists())
            || which_browser("google-chrome").is_ok()
            || which_browser("chromium").is_ok()
            || which_browser("chromium-browser").is_ok()
            || which_browser("brave-browser").is_ok()
            || which_browser("microsoft-edge").is_ok()
            || which_browser("chrome").is_ok()
            || which_browser("msedge").is_ok();

        if !has_any {
            eprintln!("跳过：本机未安装任何受支持的浏览器");
            return;
        }
        let found = detect_chrome_path();
        assert!(
            found.is_some(),
            "本机存在浏览器但 detect_chrome_path() 返回 None —— 平台路径列表缺失"
        );
        let path = found.unwrap();
        assert!(path.exists(), "detect_chrome_path() 返回了不存在的路径: {}", path.display());
        eprintln!("detect_chrome_path() -> {}", path.display());
    }

    /// 返回的路径必须真的可执行，否则 spawn 会失败。
    #[test]
    fn detected_browser_is_executable() {
        if let Some(path) = detect_chrome_path() {
            let ok = path.exists();
            assert!(ok, "路径不存在: {}", path.display());
        }
    }

    /// 主页 URL 必须剥掉会话绑定的 xsec_token / xsec_source（用户本机的 token 对服务器无效）。
    #[test]
    fn profile_url_strips_session_bound_xsec_params() {
        let raw = "https://www.xiaohongshu.com/user/profile/5c6d8d59000000001202e285\
                   ?xsec_token=ABSdL4W6abZI4RlqJqbTGcZGdw7DHvEyi_g14nnHl1q1c%3D\
                   &xsec_source=pc_search";
        assert_eq!(
            strip_modal_id_param(raw),
            "https://www.xiaohongshu.com/user/profile/5c6d8d59000000001202e285"
        );
        // 无 query / 其他参数不受影响
        assert_eq!(
            strip_modal_id_param("https://www.xiaohongshu.com/user/profile/abc"),
            "https://www.xiaohongshu.com/user/profile/abc"
        );
        assert_eq!(
            strip_modal_id_param("https://www.douyin.com/user/MS4wLjABAAAA?modal_id=1&foo=2"),
            "https://www.douyin.com/user/MS4wLjABAAAA?foo=2"
        );
    }
}
