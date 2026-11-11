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

验证：`cargo check --bin beav`、`cargo test --bin beav commands::monitor::tests`。修改打包路径时同时运行 `commands::file_ops::archive::tests`。
