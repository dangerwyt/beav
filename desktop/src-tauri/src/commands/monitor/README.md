# 监控命令模块

`../monitor.rs` 保留 IPC 路由、管理员操作白名单和统一对象访问校验。通道名称与响应结构保持兼容；外部调度入口仍从 `commands::monitor` 导出。

| 文件 | 职责 |
| --- | --- |
| `access.rs` | 对象归属、共享可见性、别名与批量标识检查 |
| `chrome.rs` | Chrome 状态、登录和 Cookie 操作 |
| `watch.rs` | 监控项管理、共享分配和清理 |
| `capture.rs` | 采集、正文补全、去重及来源归属 |
| `posts.rs` | 帖子列表、删除、选题注入和监控参数 |
| `video.rs` | 下载、抽帧、视频分析和语音转写 |
| `creation.rs` | 拆解、二创、历史列表和素材包 |
| `export.rs` | 产物目录、文件复制、Markdown 与 Obsidian 导出 |
| `schedule.rs` | 定时采集、每日预取、历史补采和启动恢复 |
| `tests.rs` | 权限、转写保留、导出错误和采集纯逻辑回归测试 |

新增涉及对象的 IPC 时，先更新 `guard_owned_target`；批量请求应在产生副作用前校验全部标识。删除仅允许对象所有者，其他操作保留共享监控项的访问规则。列表单独使用 `post_accessible` 过滤。

手动导出和素材打包必须返回文件错误。自动导出继续记录错误而不打断采集；二创内容已保存但导出失败时，通过 `exported` / `exportError` 告知前端，避免用户为重试导出重复生成内容。

导出根下的 `导出视频` **目录链接**启用视频分盘：只有视频本体写到该链接下的 `<postId>/视频.<扩展名>`，同目录的 `.beav-post-id` 保留 Web 归属校验。正文、图片、字幕和 `knowledge.md` 仍留在原帖子目录；知识卡使用 `![[导出视频/<postId>/视频.mp4]]`，本地卡与 hub 卡一致。没有该链接时保持原导出布局；链接断开时返回错误，不在内置盘创建替代目录。`monitor:get-export-root` 保留 `root` 字段，并返回 `effectiveRoot`、`videoCacheRoot` 和 `externalVideoStorageVersion`，供维护脚本确认实际空间与服务版本。

Mac 迁移方法见 [视频分盘脚本](../../../../scripts/README.md#视频分盘macos)。缓存目录通过目录链接保留旧路径，数据库无需改写。文件接口与桌面 asset scope 只放行工作区、缓存和导出目录的规范路径，不放行整块外接盘；Web 用户仍须通过帖子归属或共享校验。

Web 文件访问还会在 `assistant_core/web_access.rs` 校验记录归属。共享博主的原片、预览、抽帧和标记产物目录继续可读；素材包按请求生成独立目录，并记录请求用户及帖子清单。无归属清单的旧 ZIP 需重新打包。撤销共享后，文件和进度事件也会失去访问权限。

省略 `watchId` 的历史补采只选择当前用户拥有或获共享的监控项；后台管理员调度仍可处理全部启用项。SSE 的全局预取通知只向普通用户发送刷新信号，不包含其他用户的统计和诊断内容。

验证：`cargo check --bin beav`、`cargo test --bin beav commands::monitor::tests`。修改打包路径时同时运行 `commands::file_ops::archive::tests`。
