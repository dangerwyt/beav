# `workspace/`

## Responsibilities

- Resolves managed workspace roots and domain subdirectories.
- Maintains compatibility with legacy `.redconvert` workspace layouts.

## Rules

- Path helpers should only resolve and ensure directories.
- Do not hydrate store data, index files, or run slow scans inside path helpers.
- Media access roots include the canonical `monitor-videos` cache and the export root's `导出视频` directory link. Keep ownership checks in `assistant_core/web_access.rs`; do not grant access to the whole external volume. See [monitor export storage](../commands/monitor/README.md).
