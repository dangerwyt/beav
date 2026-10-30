# 竞品监控链路踩坑记录

核对日期：2026-10-30。适用范围：`desktop/src-tauri/src/commands/monitor.rs`、
`desktop/src-tauri/src/commands/media_transcribe.rs`、`desktop/src-tauri/src/desktop_io.rs`、
`desktop/src-tauri/src/cli_runtime/`、`desktop/src-tauri/src/ai_model_manager/`、
`desktop/src/pages/Settings.tsx`、`desktop/src/pages/Monitor.tsx`。

这些坑的共同特征：**症状与根因看起来完全不像**（数据丢了却像"抓取没抓到"、
权限报错却像"文件坏了"、失败却显示"成功"）。全部已有回归守卫，改动相关代码时
请一并跑对应测试。

---

## 1. `make_id` 同毫秒撞号 → 批量落库静默丢数据

**症状**：补历史提示「抓到 47 → 新建 47」，但库里只有 5 条；且**每次剩的条数都不一样**
（6 / 4 / 5 随机）。这是决定性线索——数据在写入时被覆盖，且覆盖与否取决于机器快慢。

**根因**：`host_impl.rs` 的 `make_id` 只用毫秒时间戳：

```rust
format!("{prefix}-{}", now_ms())   // ❌ 同一毫秒内必然撞号
```

而落库代码普遍写成「按 id 找位置 → 找到就覆盖、找不到就 push」：

```rust
if let Some(position) = store.monitored_posts.iter().position(|item| item.id == record.id) {
    store.monitored_posts[position] = record.clone();   // ← 撞号时互相覆盖
} else {
    store.monitored_posts.push(record.clone());
}
```

返回值里的 `created` 数组是**无条件 push** 的，所以照报 47 条 —— 形成
「新建 47 但库里 5 条」的矛盾。

**修法**：`make_id` 补一个进程内单调递增序号（定宽补零以保住 `sort_by(id)` 的时间序）：

```rust
static COUNTER: AtomicU64 = AtomicU64::new(0);
let seq = COUNTER.fetch_add(1, Ordering::Relaxed) % 10_000;
format!("{prefix}-{}-{:04}", now_ms(), seq)
```

**影响面**：`make_id` 全仓 150+ 处在用，凡"循环里批量创建记录"的场景（帖子、
互动快照、子任务/子会话、消息、媒体任务）此前都会撞号，本次一并修复。

**回归守卫**：`host_impl.rs` → `make_id_is_unique_within_a_tight_loop`（500 次紧循环验唯一性）。

---

## 2. 落库去重不带归属 → 帖子被别的 watch / 别的用户抢走

**症状**：某个用户卡片上的「已收录 N 篇」凭空归零；两个用户监控同一博主时互相干扰。

**根因**（两处叠加）：

1. 落库查找只按**裸 URL 全局**匹配（`post_url.split('?')`），完全不看 owner / watch_id；
2. 记录构造时 `watch_id: watch.map(|item| item.id.clone())` **无条件覆盖**。

于是后抓的一方会把先抓的记录整个抢走：先抓的用户 `owner` 还是自己、`watch_id` 却成了别人的
→ 他的列表按 owner 过滤时**看得到、按 watch 过滤时看不到**，卡片数归零。

**修法**：

- 查找按 **(归属用户, 裸URL)** 匹配，归属用户取**监控项的 owner**
  （不是当前请求的 owner —— 定时任务线程没有请求身份，= admin，用它会把各用户数据混成一堆）；
- 新增 `resolve_watch_id()` 决定归属：
  - 已有归属且原监控项还在 → **保留原归属**，不抢；
  - 原监控项**已被删除** → 归给当前监控项（否则帖子永远挂在不存在的 watch 上，卡片恒 0）；
  - 无归属的孤儿记录（临时粘贴单帖抓的）→ 归给当前监控项（顺手收编）。

**回归守卫**：`commands/monitor.rs` → `resolve_watch_id_keeps_existing_owner`、
`resolve_watch_id_reclaims_orphaned_watch`。

**配套**：`watch_add` 现在按 `(owner, 平台, 归一化主页URL)` 判重，重复添加直接报错提示，
而不是默默建第二条（两条监控项会轮流抢同一批帖子）。

---

## 3. 补历史的视频下载时序颠倒 → 补齐了却一个视频没下

**症状**：补历史「已翻到主页底部，共抓到 47 篇」，视频一条都没下载。

**根因**：视频与否由 `media_type` 决定，而 `media_type` **只有抓详情页时才会被修正**
（`post_detail` 里 `detail.mediaType` 落库）。列表抓取拿不到媒体类型，新帖默认值是 `"image"`。

原流程顺序反了：

```
① 筛视频帖   ← 此时 media_type 全是 "image" → 一个都选不出来
② 补正文     ← 这时才把 media_type 修正成 "video"，但视频任务早已筛完退出
```

小红书 URL 是 `/explore/xxx`，兜底条件 `post_url.contains("/video/")` 也匹配不上
（抖音的 `/video/` 能匹配，所以**只有抖音不受影响**）。

**修法**：把视频筛选挪到 `enrich_detail` 循环**之后**，与每日增量路径
（`run_daily_prefetch`）保持同一顺序。

**注意**：每日增量本来就是对的（先 `enrich_detail` 再判视频）——所以「每日增量能下、
补历史下不了」是这个 bug 的特征。

**回归守卫**：无（靠 `videoDiags` 诊断字段观察：`视频 N/M 篇识别为视频，已启动 X 个下载`）。

---

## 4. macOS 沙箱 EPERM：白名单不含应用工作区

**症状**：转写抽音频失败，ffmpeg 报

```
Error opening input files: Operation not permitted
[in#0 @ ...] Error opening input: Operation not permitted
```

**关键判读**：`Operation not permitted` = **EPERM**（不是 EACCES / "Permission denied"）。
在 macOS 上 EPERM 是 **`sandbox-exec` 拒绝访问**的特征，**不是文件损坏、也不是权限位问题**。

**根因**：CLI 沙箱的白名单按 **CLI environment** 推导：

```
允许读 = environment.root_path（哈希后的运行时目录）+ cwd + 工具自身 + 系统路径
```

**不含应用的工作区根**（`~/.beav`）——`cli_runtime/sandbox.rs` 里从头到尾没用过
`environment.workspace_root`。于是沙箱内的 ffmpeg 打不开 `~/.beav/monitor-videos/*.mp4`。

旁证：`monitor.rs` 里其它 5 处 ffmpeg 调用（下载/抽帧/转码/合并音轨/探测）**全部用
`Command::new` 直连**，不受沙箱约束 → 所以下载、抽帧都正常，**只有转写这一处挂**。

**修法**（两层）：

1. `build_cli_sandbox_spec` 把 `environment.workspace_root` 加进允许读列表；
2. 源文件在**应用工作区内**时直接 `Command::new` 调 ffmpeg，不走沙箱
   （与其它 5 处一致）；工作区**之外**的路径（LLM 工具 `media.transcribe` 传入的任意路径）
   仍走沙箱，边界不变。

第 2 层是保险：环境解析可能复用 `workspace_root: None` 的旧记录。

**回归守卫**：`cli_runtime/sandbox.rs` → `build_cli_sandbox_spec_allows_reading_workspace_root`。

---

## 5. 转写配置被「回退的默认源」覆盖（地址 / Key / 模型名三处）

**症状**：401 `Invalid access token`，请求却打到了用户没打算用的服务。

**根因（前端）**：`Settings.tsx` 保存设置时写的是

```js
transcription_endpoint: String(resolvedTranscriptionSource?.baseURL || formData.transcription_endpoint || …)
transcription_key:     String(resolvedTranscriptionSource?.apiKey  || formData.transcription_key  || …)
```

而 `resolvedTranscriptionSource = getAiSourceById(transcriptionSourceId) || fallbackRouteSource`，
**`fallbackRouteSource = officialAiSource`** —— 用户没为转写选源时**回退到官方源**，
于是官方源的 baseURL/apiKey **永远赢**，用户手填的地址与 key 被整个覆盖。

`transcriptionSourceId` 又是**自动解析**的（`resolveLinkedSourceIdFromList`：按已存的
endpoint+key 去匹配 AI 源，匹配不上就回退默认源），于是形成闭环：

```
手填独立 STT 地址 → 匹配不上任何源 → 回退官方源 → 保存时被官方源覆盖
→ 请求打到官方网关 + 官方 access token → token 过期 → 401
```

**用户记忆没错，是保存设置时被改的。**

**修法**：

1. 前端保存以 `formData` 为准（它已是最终生效值：选源时会被同步成源的值，手填时就是手填值）；
2. 前端转写**不再自动回退官方源**（`resolveLinkedSourceIdFromList` 加 `allowFallback`，
   转写传 false；`resolvedTranscriptionSource` 不再 `|| fallbackRouteSource`）；
3. 后端 `resolve_transcription_settings` 优先级改为 **专属配置 > 显式路由 > 默认源回退**，
   且「默认源回退」**跳过官方源** → 没配就明确报「未配置音频转写服务」，
   而不是悄悄用官方 access token。

**关键认知**：`transcription_endpoint`/`transcription_key` 存的是**最终生效值**；
`ai_model_routes_json.transcription.sourceId` 只是**来源指针**（且会被前端自动回退填脏，不可靠）。

**回归守卫**：`desktop_io.rs` → `transcription_settings_prefer_scope_config_over_default_source`、
`transcription_settings_fall_back_to_route_source`、
`transcription_route_without_source_is_not_explicit`、
`transcription_settings_never_fall_back_to_official_source`。

---

## 6. 「官方 shim」按来源指针分流 → 模型名被强行替换

**症状**：地址已经指向正确服务、认证也过了，却报

```
400 unsupported_model: 'qwen3-asr-flash-filetrans' is not a valid model_id.
Available models: 'scribe_v1', 'scribe_v1_experimental', 'scribe_v2', 'scribe_v2_medical'
```

**根因**：`resolve_agent_srt_transcription_model` 判「是不是官方服务」：

```rust
is_redbox_official_endpoint(endpoint)                       // endpoint 是 ElevenLabs → false ✓
    || transcription_route_uses_official_source(settings)    // 路由指针 → 官方源 → true ✗
```

第二个条件走的就是被第 5 条污染的**路由指针** —— 即便 endpoint 明明是 ElevenLabs，
也被判成官方 → 模型被强制成 `fun-asr` → ElevenLabs 直接 400。

同源的第三处：前端 `routeModel` 在 `mode === 'official'` 时会用
`pickBestModelForSource` **覆盖显式模型名**（`scribe_v2` 不在官方源模型列表里 → 被换成
官方那个）。

**修法**：

- endpoint 来自用户专属配置时，「是不是官方」**只看 endpoint**（endpoint 是实际请求的地址，
  最终值）；来自 AI 路由时才兼看路由指针；
- 前端转写这一路改为**用户填的模型名优先**，不走 picker 覆盖。

**回归守卫**：`commands/media_transcribe.rs` → `scope_config_endpoint_keeps_configured_model`、
`official_endpoint_still_forces_fun_asr`。

---

## 7. `window.confirm` 在 Tauri macOS WebView 里静默失效

**症状**：桌面端点「清空动态」**完全没反应，连确认框都不弹**；web 端（浏览器）正常。

**关键判读**：**完全静默**说明代码根本没走到 IPC（后端出错至少会有个 toast）。
根因是 Tauri 的 macOS 端是 **WKWebView，不实现原生 confirm 面板**，
`window.confirm()` 静默返回 false → `if (!ok) return` 直接退出。

**修法**：改用项目里已有的应用内对话框 `appConfirm`（`src/utils/appDialogs.ts` +
`ConfirmDialog`，`AppDialogsHost` 已挂在 `App.tsx`）。`main.tsx` 早就把 `window.alert`
换成了 `appAlert` —— **这个坑踩过 alert 但漏了 confirm**。

受影响处：`Monitor.tsx` 的「清空全部/清空动态」、`Publish.tsx` 的「删除发布记录」。

---

## 8. IPC 错误被桥接层吞成 fallback → 失败显示成「成功但 0 条」

**症状**：web 端点「清空动态」→ toast「已清空 **0** 条动态」，实际什么都没清；
后端明明返回了 `Err("该操作仅管理员可用")`。

**根因**：`invokeChannel` 会 **catch 掉后端返回的 Err 并转成 fallback 对象**，不抛异常。
monitor 通道的 fallback 是 `{ success: false, removedPosts: 0 }`，
而前端 `handleClearPosts` **没检查 `result.success`**，直接显示
`已清空 ${removedPosts ?? 0} 条` —— **失败被渲染成了「成功清了 0 条」**。

**修法**：

1. 前端所有 `invokeChannelGuarded` 调用**必须检查 `result.success`**；
2. monitor 的写操作 fallback 带上明确原因；
3. `fallbacks.ts` 给 monitor 的**写操作**通道加特例，错误原样透传
   （只列写操作 —— 读操作失败时返回空值才是对的，不能整个 `monitor:` 前缀都放行）；
4. `clear_posts` 返回值带上 `storePath` 与实际条数，便于区分
   「数据在别的进程/store 里」与「本进程本来就没有」。

**回归守卫**：`cli_runtime/executor.rs` → `execution_failure_summary_includes_stderr_when_verification_fails`、
`execution_failure_summary_falls_back_to_verification_summary_without_stderr`。

---

## 9. CLI 执行的错误摘要吞掉命令自己的 stderr

**症状**：ffmpeg 抽音频失败只显示「退出码校验失败：期望 0，实际 255」，
看不到 ffmpeg 真正报的那行（`Invalid data found when processing input` 等），
无从判断是文件损坏、没音轨还是参数问题。

**根因**：`execution_failure_summary` 在**验证失败**时直接 return 验证摘要，
而 stderr 分支在它**之后**，永远走不到。

**修法**：验证失败时把 stderr 末尾几行一起带上（ffmpeg 把错误堆在末尾）。
这条摘要会一路冒泡到最终的用户可见错误文案里。

**通用教训**：CLI 执行的错误摘要**不能只报「退出码不对」**，必须带命令自己的输出。

---

## 附：排查方法论

- **数字自相矛盾时先查计数口径**，别猜流程：确认两个数字是不是同源
  （本例中 `created` 数组与 store 的 `id` 匹配逻辑不同源，才对不上）。
- **git 考古比猜快**：`git log -S"关键符号" -- <路径>` 看某段逻辑到底改没改过。
  本例中「转写 401」先怀疑代码回归，`-S` 一查发现相关逻辑从 10-20 起没动过 →
  不是回归，而是「配置一直被遮蔽 + 这条链路第一次在服务器上真跑」。
- **区分「代码回归」与「配置一直被遮蔽 / 从未在目标环境验证」**。
- **改「回退链」这类语义前先找有没有对应的既有测试** —— 本例中把 Transcription 加进
  `scope_allows_source_model_fallback` 被
  `specialized_scopes_do_not_fallback_to_provider_chat_model` 直接拦下（专用 scope 不该回退到
  provider 的 chat 模型）。
- **配置类问题要看「最终生效值」而不是「来源指针」** —— 指针可能被自动回退填脏。
