# `persistence/` 模块

## 职责

- 构建/定位本地状态文件路径。
- 读写 `AppStore` 持久化状态。
- 将工作区文件系统数据 hydrate 到内存状态。

## 关键点

- 所有状态写入应通过 `with_store_mut`，避免绕过持久化。
- `load_store` 返回 `Result`：仅状态文件不存在时初始化默认值；读取或 JSON 解析失败必须停止启动，防止后续保存覆盖原数据。
- 主快照、会话 JSONL 和索引均先写临时文件、`sync_all`、关闭文件后替换。会话或索引写入失败必须中止本轮保存；后台失败记录日志，不能继续更新主快照。
- `sessions/index.json` 损坏或不可读时停止加载与保存；只有索引不存在才走旧格式迁移。会话正文按需加载。
- 归档、恢复和后台快照共用写盘锁。归档索引保留未加载的历史会话，旧快照不能撤销归档；移动文件后更新索引失败时恢复原路径。
- 磁盘清理先暂存待删文件，再更新索引；索引失败时恢复文件，成功后同步清理内存，避免下一轮保存重新创建会话。整个数据目录仍不具备跨文件事务性。
- hydrate 逻辑依赖 `workspace_loaders`，禁止在命令层重复实现文件扫描。
- `redbox-state.json` 现在只保留主快照；高体积会话产物会拆到同级 `session-artifacts/` 目录，按 session 单文件持久化。
- 加载旧快照时会自动把内嵌的 `chatMessages`、`sessionTranscriptRecords`、`sessionCheckpoints`、`sessionToolResults` 迁移到 `session-artifacts/`，并在后续保存时保持主文件瘦身。
