//! 清理「孤儿」SQLite WAL / 回滚日志副文件。
//!
//! WAL 模式（`PRAGMA journal_mode = WAL`）的库会在主库旁留下
//! `<db>.sqlite-wal` / `<db>.sqlite-shm` 副文件；回滚日志模式会留下
//! `<db>.sqlite-journal`。当主库 `<db>.sqlite`（或 `.sqlite3`）被删除——手动清理
//! 数据目录、进程被杀导致 WAL 未 checkpoint、或导入源库被移除——而这些副文件没跟着删时，
//! 它们就成了孤儿，长期积累变成垃圾（用户清理时进回收站）。
//!
//! 本模块的核心安全约束：**只在「主库文件已不存在」时才删除副文件**，绝不动主库
//! 仍然存在的库（包括正在被连接使用的库），因此对运行中的连接零风险。

use std::fs;
use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager};

/// 副文件相对主库的后缀（都带前导 `-`）。只认这些，避免误伤 Chrome 等其它进程的
/// `History-wal` / `Cookies-shm`（它们没有 `.sqlite` 主库，本来就不是我们的）。
const SIDECAR_SUFFIXES: &[&str] = &["-wal", "-shm", "-journal"];

/// 扫描应用数据目录，删除所有「主库已不存在」的 SQLite 副文件。
/// 返回清理掉的文件数（用于日志 / 统计）。
pub fn cleanup_orphaned_sqlite_sidecars(app: &AppHandle) -> usize {
    let base = match app.path().app_data_dir() {
        Ok(p) => p,
        Err(_) => return 0,
    };
    let mut removed = 0usize;
    remove_orphaned_in(&base, &mut removed);
    if removed > 0 {
        println!(
            "[sqlite-cleanup] removed {removed} orphaned WAL/rollback sidecar file(s) under {}",
            base.display()
        );
    }
    removed
}

fn remove_orphaned_in(dir: &Path, removed: &mut usize) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // 跳过体积大、无 SQLite 副文件、纯媒体的目录，避免无谓递归遍历：
            // - monitor_videos：下载的视频/抽帧
            // - browser-profile：Chrome 的 user-data-dir（其库无 .sqlite 扩展名，不归我们管）
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name == "monitor_videos" || name == "browser-profile" {
                    continue;
                }
            }
            remove_orphaned_in(&path, removed);
        } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if SIDECAR_SUFFIXES.iter().any(|s| name.ends_with(s)) {
                // 主库 = 去掉后缀。`media_jobs.sqlite-wal` -> `media_jobs.sqlite`；
                // `events.sqlite3-wal` -> `events.sqlite3`。
                let main = sidecar_to_main(&path);
                if !main.exists() {
                    if fs::remove_file(&path).is_ok() {
                        *removed += 1;
                    }
                }
            }
        }
    }
}

/// 把副文件路径还原成主库路径（截掉 `-wal` / `-shm` / `-journal` 后缀）。
fn sidecar_to_main(sidecar: &Path) -> PathBuf {
    let s = sidecar.to_string_lossy();
    for suffix in SIDECAR_SUFFIXES {
        if let Some(stripped) = s.strip_suffix(suffix) {
            return PathBuf::from(stripped);
        }
    }
    sidecar.to_path_buf()
}

/// 删除一个 SQLite 主库文件时，连带删除其 WAL / 回滚副文件，避免留下孤儿。
/// 供「删除数据库文件」的调用点（如 legacy_import 移除导入源库）复用。
///
/// 即便传入的不是 WAL 库，附加的 `-wal` / `-shm` / `-journal` 后缀文件本来就不存在，
/// `remove_file` 静默忽略，安全无副作用。
pub fn remove_sqlite_file_with_sidecars(main_path: &Path) {
    let _ = fs::remove_file(main_path);
    let base = main_path.to_string_lossy().to_string();
    for suffix in SIDECAR_SUFFIXES {
        let _ = fs::remove_file(PathBuf::from(format!("{base}{suffix}")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidecar_to_main_strips_suffix() {
        assert_eq!(
            sidecar_to_main(Path::new("/a/b/media_jobs.sqlite-wal")),
            PathBuf::from("/a/b/media_jobs.sqlite")
        );
        assert_eq!(
            sidecar_to_main(Path::new("/a/b/events.sqlite3-shm")),
            PathBuf::from("/a/b/events.sqlite3")
        );
        assert_eq!(
            sidecar_to_main(Path::new("/a/b/x.sqlite-journal")),
            PathBuf::from("/a/b/x.sqlite")
        );
    }
}
