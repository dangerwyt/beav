# `scripts/`

本目录放开发和构建辅助脚本。

## Current Scripts

- [extract-ipc-inventory.mjs](/desktop/scripts/extract-ipc-inventory.mjs): 生成 IPC 清单文档
- [sync-version.mjs](/desktop/scripts/sync-version.mjs): 同步根版本号到 Rust 元数据
- [tauri-before-dev.mjs](/desktop/scripts/tauri-before-dev.mjs): 开发期复用或拉起 Vite
- [release-utils.mjs](/desktop/scripts/release-utils.mjs): 发布脚本共用命令、临时配置与产物查找工具
- [build-all-release.mjs](/desktop/scripts/build-all-release.mjs): `release:all` 总控脚本，固定执行 Windows 远程多架构打包、macOS 多架构打包、Linux 远程打包和浏览器插件 zip 打包，并输出统一结果摘要
- [build-linux-release.mjs](/desktop/scripts/build-linux-release.mjs): Linux 桌面端发布脚本；Linux 主机本地打包，非 Linux 主机默认通过 `ssh jamdebian` 远程构建并拉回 `.deb`
- [publish-open-source-release.mjs](/desktop/scripts/publish-open-source-release.mjs): `release:oss` 总控脚本，复用 `release:all` 打包，再推送开源 tag、生成更新日志，并通过 `gh release create` 上传安装包和浏览器插件 zip
- [build-mac-release.mjs](/desktop/scripts/build-mac-release.mjs): 本地发现 `Developer ID Application` 证书，默认连续构建 `arm64 + x64` 两个 macOS 安装包，签名、notarize、staple 并验证
- [setup-mac-notary-profile.mjs](/desktop/scripts/setup-mac-notary-profile.mjs): 用 `xcrun notarytool store-credentials` 保存 Apple notarization profile
- [build-windows-release.mjs](/desktop/scripts/build-windows-release.mjs): 非 Windows 主机默认通过 `ssh jamdebian` 远程连续构建并拉回 `x64 + arm64 + x86` 三种 NSIS 安装包；也支持在本机走原生/本地交叉打包，并支持注入自定义签名命令
- [check-embedded-js-syntax.mjs](/desktop/scripts/check-embedded-js-syntax.mjs): 校验 `src-tauri/src/tools/headless_browser.rs` 里内嵌的 CDP 注入脚本语法。Rust 不校验 raw string 内的 JS，改完注入脚本必须跑一次（`pnpm check:embedded-js`）

## Maintenance Scripts

一次性数据修复脚本，**执行前必须退出应用**（否则内存快照会覆盖写入），默认自动备份 store。

- [maintenance/clean-douyin-titles.mjs](/desktop/scripts/maintenance/clean-douyin-titles.mjs): 回洗存量抖音标题（去前导「置顶」/点赞数、去重复文案）。`--dry-run` 只报告；`--self-test` 跑 10 条回归用例；`--store <path>` 指定文件
- [maintenance/fix-video-audio-flag.mjs](/desktop/scripts/maintenance/fix-video-audio-flag.mjs): 用 ffprobe 探测本地视频是否含音轨，回填 `videoHasAudio` 并把无音轨的分析记录标为 `no-audio`。`--dry-run` / `--store` / `--ffprobe` 可用
- [maintenance/dedupe-watch-list.mjs](/desktop/scripts/maintenance/dedupe-watch-list.mjs): 检测 / 合并「同一用户下重复的监控项」（同一主页被建了多条）。重复项会让两个 watch 轮流抓同一批帖子，而落库按 (owner, 裸URL) 去重 → 只有一个 watch 能拿到帖子，另一个卡片恒显示「已收录 0 篇」。默认 `--dry-run` 只报告，`--merge` 才实际合并（帖子迁移到保留的主项，同一条帖子不重复入库）
- [maintenance/switch-routes-to-relayrouter.mjs](/desktop/scripts/maintenance/switch-routes-to-relayrouter.mjs): 把 `ai_model_routes_json` 里的 scope 切到 RelayRouter —— chat / knowledge / redclaw / team / wander（`deepseek-v4-flash-0731`）与 embedding / image / videoAnalysis / voiceTts / visualIndex；`transcription` 不动，`voiceClone` 因 RelayRouter 无克隆类模型而**保持官方**。默认 `--dry-run` 只报告，`--apply` 才写入（先自动备份 store）。加 `--ensure-source` 且设置 `RELAYROUTER_API_KEY=<key>` 可在源不存在时**自动创建** RelayRouter 源（Key 走环境变量，不进 shell history）。**必须先退出应用**，否则内存快照会覆盖写入；只切路由不改作者/目录的脚本不受此限
- [maintenance/fix-post-authors.mjs](/desktop/scripts/maintenance/fix-post-authors.mjs): 修复被详情页误抓污染的帖子作者名。旧版 `post_detail` 会无条件用详情页作者覆盖记录，而详情页兜底链会把正文里的 `@提及`（如「@抖音科技」）或推荐位作者当成作者 → 同一博主在导出目录里裂成多个作者文件夹。规则：有 `watch_id` 且 watch 仍存在时，作者应为 `watch.name`。`--dry-run` 只报告；`--apply` 写入；`--merge-dirs` 按 `.beav-post-id` 标记把裂开的导出目录挪回正确作者下并清理空目录（同名目录按 `{标题}_{postId}` 避让，不覆盖）；`--rename-dirs` 按目录内 `帖子信息.json` 的 `title` 重命名产物目录（取前 40 字符，同名追加序号），若配了 `monitorObsidianVault` 会同步改 hub 目录名并替换笔记内的库内路径。**必须先退出应用**
- [maintenance/probe-douyin-date.mjs](/desktop/scripts/maintenance/probe-douyin-date.mjs): 抖音发布时间探针（只读）。复用应用已拉起的 Chrome（读 profile 里的 `DevToolsActivePort` 拿实际调试端口，不另起浏览器），打开一条抖音视频页并打印四条提取来源的原始值：DOM 选择器 / `__INITIAL_STATE__.itemModule[vid].createTime` / `_ROUTER_DATA` 的 `item_list[*].create_time` / 页面文本里的「发布时间：YYYY-MM-DD」。另会列出所有含日期的文本节点及其 `data-e2e`、class 与祖先链，用来在抖音改版后重新定位选择器。默认从 store 里挑一条无 `publishedAt` 的抖音帖，可用 `--url` / `--store` 覆盖。**前提**：应用在运行且抖音已登录。
- [maintenance/backfill-publish-time.mjs](/desktop/scripts/maintenance/backfill-publish-time.mjs): 批量补「发布时间」。对 `publishedAt` 为空的帖子逐条触发 `monitor:post-detail`（等价于界面点「重新抓正文」），走 daemon 的 `/api/ipc/invoke`，有进度、失败原因汇总、可中断续跑（已补上的下次跳过）。`--limit` 限条数（默认 20）、`--concurrency` 并发（默认 2）、`--reason content` 改为筛「正文等于标题 / 比标题短」（洗平台改版导致的标题级正文），成功判据同步改成「变长了」；`--platform` 换平台、`--dry-run` 只看清单。**前提**：桌面端在运行、平台 Chrome 已登录；且 `post_detail` 已有「新正文不短于旧正文才覆盖」保护，否则会把完整正文冲成标题级。
- [maintenance/retry-transcribe.mjs](/desktop/scripts/maintenance/retry-transcribe.mjs): 批量重跑失败的语音转写。按 `postVideoAnalyses[].transcriptStatus` 分类处理：`failed` / `pending` 直接重试转写；`no-audio` 先走 `monitor:download-video {patchAudio:true}` 补音轨再转写；`not-configured` 跳过（没配 ASR 重试无意义）。调 `monitor:transcribe-video`（只补转写，不重抽帧）。`--status` 指定要处理的状态（默认 `failed,pending`）、`--limit`、`--concurrency`、`--platform`、`--watch-id`、`--dry-run`。结束时报「仍待重跑 N 条」，可中断续做。**前提**：桌面端在运行、平台 Chrome 已登录、已配好 ASR 服务。
- [sync-wb-memory.mjs](/desktop/scripts/sync-wb-memory.mjs): 同步 WorkBuddy 的**工作区记忆**到另一台机器。WorkBuddy 的云端 profile、用户级 MEMORY.md、对话历史都跟账号走（换机自动有），**只有工作区记忆**（`<workspace>/.workbuddy/memory/`）是纯本地文件，需要手动同步。`status` 看状态 / `push` 导出到私有 git 仓库 / `pull` 合并回来（同名文件按 mtime 新者胜，冲突会列出来）。仓库路径用 `WB_MEMORY_REPO` 指定（默认 `~/wb-memory`），工作区用 `WB_WORKSPACE` 指定（默认按记忆文件数自动选）

## Artifact Paths

- 最终 macOS 安装包复制到 `artifacts/installers/macos/`
- 最终 Windows 安装包复制到 `artifacts/installers/windows/`
- 最终 Linux 安装包复制到 `artifacts/installers/linux/`
- 最新浏览器插件压缩包复制到 `artifacts/installers/browser-plugin/`
- 构建摘要写到 `artifacts/release/`
- 开源发布更新日志写到 `artifacts/release/vX.Y.Z-release-notes.md`
- `artifacts/` 应保持为本地构建输出目录，不进入 Git

## Rules

- 脚本输出路径必须明确、稳定。
- 报错信息要足够定位问题，不要吞异常。
- 脚本变更后至少手动运行一次。

## Related Docs

- [本地构建与源码范围](../README.md)
