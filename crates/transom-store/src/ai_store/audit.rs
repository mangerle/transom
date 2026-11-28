//! AI 运维审计日志的受限追加与轮转。

use std::fs;
use std::io::Write;
use std::path::PathBuf;

use transom_ai_contracts::{AiAuditEntry, AiError};
use transom_platform::private_fs;

use super::AiStore;

/// 单个审计文件的最大体积。
const MAX_AUDIT_FILE_BYTES: u64 = 16 * 1024 * 1024;
/// 单条审计记录的最大体积。
const MAX_AUDIT_ENTRY_BYTES: usize = 1024 * 1024;
/// 保留的历史审计文件数量。
const AUDIT_RETAINED_FILES: usize = 5;

impl AiStore {
    /// 追加一条 AI 运维审计记录，并在文件达到上限时轮转。
    ///
    /// 审计写入使用进程内串行锁，保证尺寸检查、轮转和追加是一个逻辑事务；
    /// 历史文件采用 `audit.1.jsonl` 到 `audit.5.jsonl` 的固定序号保留。
    ///
    /// # Errors
    /// 审计记录超过单条上限、目录或文件操作失败时返回 [`AiError::StorageFailed`]。
    pub fn append_audit(&self, entry: &AiAuditEntry) -> Result<(), AiError> {
        let _guard = self.audit_lock.lock();
        self.append_audit_with_limits(entry, MAX_AUDIT_FILE_BYTES, AUDIT_RETAINED_FILES)
    }

    fn append_audit_with_limits(
        &self,
        entry: &AiAuditEntry,
        max_file_bytes: u64,
        retained_files: usize,
    ) -> Result<(), AiError> {
        private_fs::ensure_private_dir(&self.root).map_err(|error| AiError::StorageFailed {
            target: self.root.display().to_string(),
            reason: error.to_string(),
        })?;
        let line = entry.to_json_line()?;
        if line.len() > MAX_AUDIT_ENTRY_BYTES {
            return Err(AiError::StorageFailed {
                target: "AI 审计记录".to_owned(),
                reason: format!(
                    "单条审计记录超过体积上限（实际 {} 字节，上限 {MAX_AUDIT_ENTRY_BYTES} 字节）",
                    line.len()
                ),
            });
        }

        let path = self.audit_path(0);
        let current_bytes = match fs::metadata(&path) {
            Ok(metadata) => metadata.len(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => {
                return Err(AiError::StorageFailed {
                    target: path.display().to_string(),
                    reason: error.to_string(),
                });
            }
        };
        let next_bytes = current_bytes
            .saturating_add(line.len() as u64)
            .saturating_add(1);
        if next_bytes > max_file_bytes && current_bytes > 0 {
            self.rotate_audit(max_file_bytes, retained_files)?;
        }

        let path = self.audit_path(0);
        let mut file =
            private_fs::open_private_append(&path).map_err(|error| AiError::StorageFailed {
                target: path.display().to_string(),
                reason: error.to_string(),
            })?;
        writeln!(file, "{line}").map_err(|error| AiError::StorageFailed {
            target: path.display().to_string(),
            reason: error.to_string(),
        })
    }

    fn rotate_audit(&self, max_file_bytes: u64, retained_files: usize) -> Result<(), AiError> {
        let retained_files = retained_files.max(1);
        let oldest = self.audit_path(retained_files);
        if oldest.exists() {
            fs::remove_file(&oldest).map_err(|error| AiError::StorageFailed {
                target: oldest.display().to_string(),
                reason: error.to_string(),
            })?;
        }
        for index in (1..retained_files).rev() {
            let from = self.audit_path(index);
            let to = self.audit_path(index + 1);
            if !from.exists() {
                continue;
            }
            if to.exists() {
                fs::remove_file(&to).map_err(|error| AiError::StorageFailed {
                    target: to.display().to_string(),
                    reason: error.to_string(),
                })?;
            }
            fs::rename(&from, &to).map_err(|error| AiError::StorageFailed {
                target: format!("{} -> {}", from.display(), to.display()),
                reason: error.to_string(),
            })?;
        }
        let current = self.audit_path(0);
        if current.exists() {
            let first = self.audit_path(1);
            if first.exists() {
                fs::remove_file(&first).map_err(|error| AiError::StorageFailed {
                    target: first.display().to_string(),
                    reason: error.to_string(),
                })?;
            }
            fs::rename(&current, &first).map_err(|error| AiError::StorageFailed {
                target: format!("{} -> {}", current.display(), first.display()),
                reason: error.to_string(),
            })?;
        }
        log::debug!(
            "AI 审计日志达到上限，已轮转并保留最近 {} 个历史文件（单文件上限 {} 字节）",
            retained_files,
            max_file_bytes
        );
        Ok(())
    }

    fn audit_path(&self, index: usize) -> PathBuf {
        if index == 0 {
            self.root.join("audit.jsonl")
        } else {
            self.root.join(format!("audit.{index}.jsonl"))
        }
    }
}

#[cfg(test)]
mod tests {
    use transom_ai_contracts::{AiAuditEntry, ToolRisk};

    use super::AiStore;

    fn entry(sequence: usize) -> AiAuditEntry {
        AiAuditEntry {
            timestamp: format!("2026-11-28T00:00:{sequence:02}Z"),
            conversation_id: "conv-audit".to_owned(),
            host_label: "host-a".to_owned(),
            tool_name: "remote_exec".to_owned(),
            risk: ToolRisk::SafeRead,
            summary: "uptime".to_owned(),
            approved: true,
            ok: true,
            duration_ms: 1,
        }
    }

    #[test]
    fn rotates_audit_files_with_bounded_retention() {
        let dir = tempfile::tempdir().expect("创建临时目录失败");
        let store = AiStore::with_root(dir.path().to_path_buf());
        for sequence in 0..8 {
            store
                .append_audit_with_limits(&entry(sequence), 420, 2)
                .expect("追加审计失败");
        }

        assert!(dir.path().join("audit.jsonl").exists());
        assert!(dir.path().join("audit.1.jsonl").exists());
        assert!(dir.path().join("audit.2.jsonl").exists());
        assert!(!dir.path().join("audit.3.jsonl").exists());
        for path in [
            dir.path().join("audit.jsonl"),
            dir.path().join("audit.1.jsonl"),
            dir.path().join("audit.2.jsonl"),
        ] {
            assert!(std::fs::metadata(path).expect("读取审计文件属性失败").len() <= 420);
        }
    }
}
