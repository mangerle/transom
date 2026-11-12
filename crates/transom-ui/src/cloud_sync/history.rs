//! 云同步历史：有界 JSON Lines 持久化与后台追加线程。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;
use transom_core::{SyncOperation, SyncTrigger};

/// 历史文件读取与保留的上限。
pub(crate) const MAX_HISTORY_ENTRIES: usize = 200;
/// 历史文件最大字节数；超过时由写线程重写成最近记录。
const MAX_HISTORY_BYTES: u64 = 512 * 1024;
/// 有界追加通道容量；历史不是关键事务，满载时丢弃新项并记录告警。
const HISTORY_QUEUE_CAPACITY: usize = 16;

/// 历史条目状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SyncHistoryStatus {
    Idle,
    Success,
    Failed,
    Conflict,
    Running,
}

/// 可持久化的云同步活动记录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SyncHistoryEntry {
    pub(crate) id: String,
    pub(crate) at_ms: i64,
    pub(crate) operation: SyncOperation,
    pub(crate) trigger: SyncTrigger,
    pub(crate) status: SyncHistoryStatus,
    pub(crate) provider: String,
    pub(crate) duration_ms: u64,
    pub(crate) sessions: usize,
    pub(crate) folders: usize,
    pub(crate) bytes: u64,
    pub(crate) message: String,
}

/// 历史文件路径；与设置同属配置目录。
pub(crate) fn sync_history_path() -> PathBuf {
    transom_platform::paths::config_dir().join("cloud-sync-history.jsonl")
}

/// 读取历史文件尾部并解析最近记录。
pub(crate) fn load_history(path: &Path) -> Vec<SyncHistoryEntry> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            log::warn!(
                "读取云同步历史失败，路径: {}，原因: {}",
                path.display(),
                error
            );
            return Vec::new();
        }
    };
    let bytes = if bytes.len() as u64 > MAX_HISTORY_BYTES {
        &bytes[bytes.len() - MAX_HISTORY_BYTES as usize..]
    } else {
        &bytes[..]
    };
    let mut entries = Vec::with_capacity(64);
    for line in String::from_utf8_lossy(bytes).lines() {
        let Ok(entry) = serde_json::from_str::<SyncHistoryEntry>(line) else {
            continue;
        };
        entries.push(entry);
        if entries.len() > MAX_HISTORY_ENTRIES {
            let excess = entries.len() - MAX_HISTORY_ENTRIES;
            entries.drain(0..excess);
        }
    }
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.at_ms));
    entries
}

/// 追加线程命令。
enum HistoryCommand {
    Append(SyncHistoryEntry),
    Shutdown,
}

/// 有界异步历史写入器；Drop 时等待最后一次写入结束后退出。
pub(crate) struct SyncHistoryWriter {
    tx: flume::Sender<HistoryCommand>,
    handle: Option<JoinHandle<()>>,
}

impl SyncHistoryWriter {
    /// 创建写入器；线程启动失败时返回 `None`，历史仅保留内存态。
    pub(crate) fn spawn(path: PathBuf) -> Option<Self> {
        let (tx, rx) = flume::bounded(HISTORY_QUEUE_CAPACITY);
        let handle = std::thread::Builder::new()
            .name("transom-sync-history".into())
            .spawn(move || {
                while let Ok(command) = rx.recv() {
                    match command {
                        HistoryCommand::Append(entry) => {
                            if let Err(error) = append_history(&path, entry) {
                                log::warn!(
                                    "写入云同步历史失败，路径: {}，原因: {}",
                                    path.display(),
                                    error
                                );
                            }
                        }
                        HistoryCommand::Shutdown => break,
                    }
                }
            })
            .map_err(|error| log::error!("启动云同步历史写线程失败，原因: {error}"))
            .ok()?;
        Some(Self {
            tx,
            handle: Some(handle),
        })
    }

    /// 快速入队；通道满时只记录告警，不阻塞界面线程。
    pub(crate) fn append(&self, entry: SyncHistoryEntry) {
        if self.tx.try_send(HistoryCommand::Append(entry)).is_err() {
            log::warn!("云同步历史队列已满，本次活动仅保留在内存中");
        }
    }
}

impl Drop for SyncHistoryWriter {
    fn drop(&mut self) {
        let _ = self.tx.send(HistoryCommand::Shutdown);
        if let Some(handle) = self.handle.take()
            && handle.join().is_err()
        {
            log::error!("云同步历史写线程异常退出");
        }
    }
}

fn append_history(path: &Path, entry: SyncHistoryEntry) -> std::io::Result<()> {
    let mut entries = load_history(path);
    entries.insert(0, entry);
    entries.sort_by_key(|item| std::cmp::Reverse(item.at_ms));
    entries.truncate(MAX_HISTORY_ENTRIES);
    let mut content = String::with_capacity(entries.len().saturating_mul(512));
    for entry in entries {
        match serde_json::to_string(&entry) {
            Ok(line) => {
                content.push_str(&line);
                content.push('\n');
            }
            Err(error) => log::warn!("序列化云同步历史失败，已跳过该条记录: {error}"),
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    transom_store::write_atomic(path, content.as_bytes())
        .map_err(|error| std::io::Error::other(error.to_string()))
}

/// 覆盖式追加单个条目；仅供测试与首次创建使用。
#[cfg(test)]
fn append_direct(path: &Path, entry: SyncHistoryEntry) -> std::io::Result<()> {
    use std::fs::OpenOptions;
    use std::io::Write;

    let line = serde_json::to_string(&entry).map_err(std::io::Error::other)?;
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{line}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(at_ms: i64, status: SyncHistoryStatus) -> SyncHistoryEntry {
        SyncHistoryEntry {
            id: format!("entry-{at_ms}"),
            at_ms,
            operation: SyncOperation::Check,
            trigger: SyncTrigger::Periodic,
            status,
            provider: "webdav".into(),
            duration_ms: 12,
            sessions: 3,
            folders: 1,
            bytes: 99,
            message: "测试记录".into(),
        }
    }

    #[test]
    fn history_load_sorts_and_limits_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        for index in 0..(MAX_HISTORY_ENTRIES + 5) {
            append_direct(&path, entry(index as i64, SyncHistoryStatus::Success)).unwrap();
        }
        let loaded = load_history(&path);
        assert_eq!(loaded.len(), MAX_HISTORY_ENTRIES);
        assert_eq!(loaded[0].at_ms, (MAX_HISTORY_ENTRIES + 4) as i64);
    }

    #[test]
    fn invalid_lines_are_ignored_and_missing_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        assert!(load_history(&path).is_empty());
        std::fs::write(&path, "not-json\n").unwrap();
        assert!(load_history(&path).is_empty());
    }

    #[test]
    fn writer_persists_entry_before_shutdown() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        let writer = SyncHistoryWriter::spawn(path.clone()).unwrap();
        writer.append(entry(1, SyncHistoryStatus::Conflict));
        drop(writer);
        let loaded = load_history(&path);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].status, SyncHistoryStatus::Conflict);
    }
}
