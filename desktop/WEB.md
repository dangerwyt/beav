# Beav Web 版

同一套前端，两种服务方式：

| 模式 | 脚本 | 适用 | 账号 | 数据 |
|---|---|---|---|---|
| **单用户** | `scripts/web-server.mjs` (:8080) | 自己/内网一台机器 | 可选 Basic Auth | 与桌面端**共用**同一份 store |
| **多用户（方案 B）** | `scripts/web-instances.mjs` (:8090) | 多人使用、要隔离 | 本地账号 + 登录 | **每人一个隐藏实例**，数据/工作区/导出/Chrome 全隔离 |

> 早期版本是"零 Rust 改动"的桥接；现在 daemon 已内置 `/api/events`(SSE)、`/api/file`(文件代理)
> 与实例级环境变量，桌面端行为不变（不设环境变量时完全一致）。

---

## 一、单用户模式

```
浏览器 ──同源──► web-server.mjs (:8080) ──反代──► assistant daemon (127.0.0.1:31937) ──► 桌面端能力
                 ├ 静态托管 dist
                 ├ /api/ipc/*  → daemon（改写 Origin 为 loopback）
                 ├ /api/events → daemon SSE（实时事件，原样透传）
                 └ /api/file   → daemon 读本地文件（白名单=工作区+导出根）
```

**前提：桌面端 Beav 必须运行**（它监听 31937、提供全部能力）。

```bash
cd desktop
pnpm web:build
pnpm web:serve          # http://0.0.0.0:8080
```

局域网放行（一次性，需管理员）：

```powershell
netsh advfirewall firewall add rule name="Beav Web 8080" dir=in action=allow protocol=TCP localport=8080
```

| 变量 | 默认 | 说明 |
|---|---|---|
| `WEB_HOST` | `0.0.0.0` | 监听地址 |
| `WEB_PORT` | `8080` | 端口 |
| `BEAV_DAEMON` | `http://127.0.0.1:31937` | daemon 地址 |
| `WEB_DIST` | `../dist` | 静态目录 |
| `WEB_BASIC_USER` / `WEB_BASIC_PASS` | 空 | 可选 Basic Auth（单账号） |

---

## 二、单实例多租户（推荐：服务器只开桌面端）

适合"服务器只跑一个桌面 App，局域网用户用 web"：

```
局域网用户 ──► web 网关(8090) ──登录 + X-Beav-User──► 桌面端 daemon(31937)
                                                        │
                                            一份 store，watch/post 带 owner
                                            ├ 桌面端（admin）：看全部、跑全部
                                            └ web 用户：只看/只写自己的
```

```bash
# 桌面端（服务器，可省略扫码）
BEAV_SKIP_OFFICIAL_LOGIN=1 pnpm tauri:dev:beav     # 或运行打包好的 App

# web 网关：单实例模式，不拉起隐藏实例
cd desktop
BEAV_SINGLE_DAEMON=1 pnpm web:instances            # 8090
```

- 账号仍用 `users.json`（`--create` / 注册页）；请求由网关注入 `X-Beav-User`；
- daemon 按 `owner` 过滤：桌面端（无头）看全部，web 用户只看自己；
- 夜间由**桌面端的调度器**遍历所有 watch（无需多实例/预热）；
- 旧数据 `owner` 为空 → 归 admin；新数据写入当前用户；
- **已知限制**：图片/视频/导出仍共用一个工作区，未按 owner 分目录（适合 2–3 个可信用户）。

### macOS 常驻（launchd，开机/登录自启）

```bash
cd desktop
bash deploy/install-macos-services.sh              # 安装/更新
bash deploy/install-macos-services.sh --uninstall  # 卸载
```

- `com.beav.desktop`：桌面端（`BEAV_SKIP_OFFICIAL_LOGIN=1`），登录自启、**崩溃重启**（手动退出不重启）；
- `com.beav.web`：web 网关（`BEAV_SINGLE_DAEMON=1`，8090），常驻；
- 日志：`~/Library/Logs/beav/{desktop,web}.log`；
- 需要**图形登录会话**（`LimitLoadToSessionType=Aqua`），纯 SSH 不会启动；
- 改了二进制/前端后：重启服务即可
  ```bash
  launchctl kickstart -k gui/$(id -u)/com.beav.desktop
  launchctl kickstart -k gui/$(id -u)/com.beav.web
  ```

## 三、多用户多实例（方案 B，每用户一个隐藏实例）

```
浏览器 ──► web-instances.mjs (:8090)
            ├ /login /register /account   本地账号（scrypt + HMAC cookie）
            ├ /admin                      管理员页（状态/内存/停止）
            ├ 静态 dist                    共享（不含用户数据）
            └ /api/*  ──按登录用户反代──► 每用户隐藏实例
                                          beav.exe (BEAV_HEADLESS=1, :32001/:32002/…)
                                          独立 APPDATA / store / workspace / exports
                                          / chrome-profile / webview
```

- 首次访问某用户的 `/api/*` 才拉起他的实例（冷启动约 5–40s，之后常驻秒开）；
- 闲置 `BEAV_IDLE_MINUTES` 分钟自动回收（下次访问再拉起）；
- 静态资源与登录页共享，**数据只在各实例内**。

### 快速开始

```bash
cd desktop
pnpm web:build
pnpm web:instances        # http://0.0.0.0:8090
```

> **跨平台**
> - Windows 的实例二进制是 `beav.exe`；macOS/Linux 是 `beav`（无扩展名），也会识别 `.app` 包；
> - 服务器上必须**先构建**（`pnpm tauri:build:beav`）或用 `BEAV_EXE` 指向可执行文件；
> - 默认用户目录：Windows `D:\beav_workspace\users`，macOS/Linux `~/.beav-workspace/users`
>   （可用 `BEAV_USERS_ROOT` 覆盖）；
> - macOS 需要有**图形会话**（Tauri/WebView 依赖 WindowServer），纯 SSH 无登录会话跑不起来。

首次启动若没有 `users.json`，会创建管理员并**打印随机密码**：

```
[multi] 默认账号 admin / xxxxxxxx  ← 请尽快用 --set-password 修改
```

浏览器打开 `http://<服务器IP>:8090` → 登录 / 注册。

### 用户管理（命令行）

```bash
node scripts/web-instances.mjs --list
node scripts/web-instances.mjs --create alice '强密码' 32002
node scripts/web-instances.mjs --create-admin boss '强密码'
node scripts/web-instances.mjs --set-password alice '新密码'
node scripts/web-instances.mjs --delete alice
```

- 端口必须唯一；不传端口时自动取 `32001+` 的空闲值；
- 用户表 `desktop/users.json`、会话密钥 `desktop/.beav-session-secret` 均已 gitignore。

### 夜间增量（本地 00:00）

由**管理器集中触发**：到点对每个用户实例调用 `monitor:run-daily`。
受管实例内部自带的每日定时器已通过 `BEAV_DISABLE_DAILY_SCHEDULER=1` 关闭，不会重复跑。

| 变量 | 默认 | 说明 |
|---|---|---|
| `BEAV_DAILY_AT` | `00:00` | 集中触发时间；置空关闭 |
| `BEAV_PREWARM_AT` | `23:55` | 提前拉起所有用户实例；置空关闭 |
| `BEAV_KEEP_UNTIL` | `03:00` | 预热实例保活到几点 |
| `BEAV_IDLE_MINUTES` | `30` | 闲置回收分钟；`0` = 不回收 |

管理器会先按该用户的 watch 平台调 `monitor:chrome-launch` 拉起 Chrome，再跑预取；
任务开始时自动保活 4 小时。

> Chrome 必须能拉起（服务器有图形会话）。若 Chrome 被关掉，详情/视频抓取仍会失败。
> 桌面端同理：App 必须开着。

### 页面

| 路径 | 权限 | 内容 |
|---|---|---|
| `/login` | 公开 | 登录（含注册入口） |
| `/register` | 公开（可关） | 注册并自动登录 |
| `/account` | 登录用户 | 自己的端口/状态/PID/内存/数据目录 |
| `/admin` | `role=admin` | 所有用户实例状态、内存、启动时间、最近活跃、停止按钮（5s 刷新） |
| `/admin/user/<uid>` | admin | 该用户的**只读**数据：监控博主 + **视频下载异常** + 最近动态 +「立即跑每日任务」 |
| `/admin/api/instances` | admin | 上述数据 JSON |
| `/admin/api/user/<uid>` | admin | 该用户的 watches + posts + `issues`（失败/卡住）+ `pendingVideos` |
| `/admin/api/retry-video` | admin | POST `{username, postId, platform?}` 重新下载单个视频（先拉起对应平台 Chrome） |
| `/admin/api/retry-videos` | admin | POST `{username, ids}` 批量重试 |
| `/admin/api/run-daily` | admin | POST `{username}` 触发该用户的每日预取 |
| `/admin/api/stop` | admin | POST `{username}` 停止实例 |

注册开关：`BEAV_ALLOW_REGISTER=0` 关闭自助注册。

### 每用户目录

```
D:\beav_workspace\users\<uid>\
├─ appdata\            # APPDATA（auth-state.json / acp-gateway.json 等）
├─ state\beav-state.json
├─ workspace\          # monitor-images / monitor-videos / spaces
├─ exports\            # <平台>\<导出日期>\<作者>\<标题>\
├─ chrome-profile\     # xiaohongshu / douyin / bilibili 登录态
└─ webview\            # WebView2 user-data
```

### 实例级环境变量（由管理器注入，无需手填）

| 变量 | 作用 |
|---|---|
| `APPDATA` | 隔离 Tauri 的 app_data（官方登录态、acp 配置等） |
| `BEAV_STORE_DIR` | 独立 `beav-state.json` |
| `BEAV_WORKSPACE_ROOT` | 独立工作区（图片/视频/笔记/记忆） |
| `BEAV_EXPORT_ROOT` | 强制导出根（**优先级高于 store 设置**） |
| `BEAV_CHROME_PROFILE_DIR` | 独立 Chrome profile（不设则跟随 `BEAV_STORE_DIR`） |
| `BEAV_WEBVIEW_DATA_DIR` | 独立 WebView2 user-data |
| `BEAV_DAEMON_PORT` | 实例 daemon 端口 |
| `BEAV_HEADLESS=1` | 不显示窗口 |
| `BEAV_MANAGED_DIRS=1` | 拒绝客户端修改导出目录/工作区 |

### 受管模式（Web 用户不可选目录）

- 后端：`BEAV_MANAGED_DIRS=1` 时 `monitor:set-export-root`、`monitor:pick-export-root`、
  `settings:pick-workspace-dir` 直接返回错误；`BEAV_EXPORT_ROOT` 覆盖 store 值。
- 前端：Web 模式下「监控导出目标」「工作区」显示为只读提示，隐藏选择按钮。
- 桌面端（默认实例）不受影响，仍可自由设置。

---

## 三、安全（**暴露到局域网前必读**）

- `/api/ipc/invoke` 能调用几乎所有能力（抓取、下载、执行命令、读写知识库）：
  - **单用户模式**没有账号体系，务必设 `WEB_BASIC_USER/PASS` 或只绑 `127.0.0.1`；
  - **多用户模式**有登录与隔离，但**服务器管理员在 OS 层仍能看到所有用户目录**；
- 文件代理只读，白名单=工作区+导出根，越界 403；
- 生产建议 Caddy/Nginx 反代加 HTTPS（麦克风/剪贴板等需要安全上下文）。

## 四、已知限制

| 限制 | 说明 |
|---|---|
| **实时事件** | daemon 提供 `/api/events` SSE；前端 `on()` 用 `EventSource` 订阅（覆盖前端订阅的事件通道）。 |
| **系统对话框** | 原生「选择文件夹」在 Web 受管模式下被拒绝（本就不该让远端选服务器路径）。 |
| **麦克风/剪贴板** | 需要 `localhost` 或 `https`；局域网纯 `http` 不可用。 |
| **官方登录/扫码** | 依赖官方后端配置；每个实例独立登录态。 |
| **实例资源** | 每实例约 300–500MB（隐藏 Tauri + WebView2），`BEAV_IDLE_MINUTES` 可调。 |
| **冷启动** | 首次访问某用户需 5–40s 启动实例。 |
| **单进程模型** | 同一用户并发访问共享一个实例；多标签页无冲突。 |

## 五、HTTPS 反代（Caddy）

示例见 `deploy/Caddyfile`。要点：

- **SSE 必须 `flush_interval -1`**（`/api/events` 要即时 flush，否则实时进度会被缓冲）；
- 公网域名：Caddy 自动申请证书；内网 IP 用 `tls internal`（自签，需在客户端信任 Caddy 根证书）；
- 不要对 `text/event-stream` 开压缩；
- **扫码登录不依赖 HTTPS**：二维码只是 URL/图片，扫码后的跳转由官方后端处理；HTTPS 只影响
  安全上下文（麦克风/剪贴板）与传输安全。

```caddyfile
beav.example.com {
	reverse_proxy 127.0.0.1:8090 {
		flush_interval -1
	}
}
```

## 六、与桌面端的关系

- 桌面端默认实例（`%APPDATA%\Beav`）不受影响；
- 单用户模式共用桌面端 store；
- 多用户模式的每个实例都是**独立 APPDATA + 独立数据目录**的隐藏进程；
- 所有隔离都靠环境变量开关，**不设环境变量时行为与原来一致**。
