//! 三层透明记忆契约与路径安全读写辅助。
//!
//! 设计原理（Why）：
//! - 运维场景严禁黑盒幻觉记忆：长期记忆以纯文本 Markdown 保存，用户可随时查看、编辑与删除；
//! - 严格区分“全局用户偏好（`global.md`）”与“主机环境画像（`hosts/<id>.md`）”，按主机隔离防止串台；
//! - 所有文件系统读写均校验沙箱根路径与单文件大小上限，杜绝目录穿越与大文件 OOM。

use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use transom_platform::private_fs;

use crate::error::AiError;

/// 记忆文件读取上限（16 KiB），避免意外读取超大文件撑爆上下文或内存。
pub const MAX_MEMORY_BYTES: usize = 16 * 1024;

/// 主机或全局记忆读取快照。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemorySnapshot {
    #[serde(default)]
    pub global_markdown: String,
    #[serde(default)]
    pub host_markdown: String,
}

/// AI 提议写入主机记忆的变更请求（须经用户确认才真正落盘）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryProposal {
    pub host_id: String,
    pub host_name: String,
    pub fact: String,
    #[serde(default)]
    pub category: String,
}

/// 校验并解析受控沙箱目录内的安全子路径，防止 `../` 路径穿越。
pub fn resolve_sandbox_path(root: &Path, relative: &Path) -> Result<PathBuf, AiError> {
    if relative.is_absolute() {
        return Err(AiError::PathViolation {
            path: relative.display().to_string(),
            reason: "不允许传入绝对路径".to_owned(),
        });
    }
    for component in relative.components() {
        if matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        ) {
            return Err(AiError::PathViolation {
                path: relative.display().to_string(),
                reason: "路径包含越权跳转片段".to_owned(),
            });
        }
    }

    if fs::symlink_metadata(root).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(AiError::PathViolation {
            path: root.display().to_string(),
            reason: "沙箱根目录不得为符号链接".to_owned(),
        });
    }
    private_fs::ensure_private_dir(root).map_err(|err| AiError::StorageFailed {
        target: root.display().to_string(),
        reason: err.to_string(),
    })?;
    let canonical_root = fs::canonicalize(root).map_err(|err| AiError::StorageFailed {
        target: root.display().to_string(),
        reason: err.to_string(),
    })?;
    let candidate = canonical_root.join(relative);
    if !candidate.starts_with(&canonical_root) {
        return Err(AiError::PathViolation {
            path: candidate.display().to_string(),
            reason: "解析后路径超出受控沙箱根目录".to_owned(),
        });
    }

    let mut probe = canonical_root.clone();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            continue;
        };
        probe.push(name);
        match fs::symlink_metadata(&probe) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(AiError::PathViolation {
                    path: probe.display().to_string(),
                    reason: "沙箱路径不得包含符号链接".to_owned(),
                });
            }
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                return Err(AiError::StorageFailed {
                    target: probe.display().to_string(),
                    reason: err.to_string(),
                });
            }
        }
    }
    Ok(candidate)
}

/// 安全读取有限大小的记忆文件；不存在时返回空字符串。
pub fn read_memory_file_limited(path: &Path) -> Result<String, AiError> {
    if !path.exists() {
        return Ok(String::new());
    }
    let meta = fs::metadata(path).map_err(|err| AiError::StorageFailed {
        target: path.display().to_string(),
        reason: err.to_string(),
    })?;
    if meta.len() > MAX_MEMORY_BYTES as u64 {
        return Err(AiError::StorageFailed {
            target: path.display().to_string(),
            reason: format!("记忆文件超过体积上限（最大 {MAX_MEMORY_BYTES} 字节）"),
        });
    }
    fs::read_to_string(path).map_err(|err| AiError::StorageFailed {
        target: path.display().to_string(),
        reason: err.to_string(),
    })
}

/// 将主机记忆片段规范化追加到已有 Markdown 内容中。
pub fn append_host_fact(existing: &str, fact: &str, category: &str) -> String {
    let clean_fact = fact.trim();
    if clean_fact.is_empty() {
        return existing.to_owned();
    }
    let bullet = if category.trim().is_empty() {
        format!("- {clean_fact}")
    } else {
        format!("- [{}] {clean_fact}", category.trim())
    };
    let trimmed = existing.trim_end();
    if trimmed.is_empty() {
        format!("# 主机运维记忆\n\n{bullet}\n")
    } else {
        format!("{trimmed}\n{bullet}\n")
    }
}

#[cfg(test)]
mod tests {
    use super::{append_host_fact, resolve_sandbox_path};

    use crate::error::AiError;
    use std::path::Path;

    #[test]
    fn sandbox_rejects_parent_traversal() {
        let dir = tempfile::tempdir().expect("创建临时目录失败");
        let err = resolve_sandbox_path(dir.path(), Path::new("../../etc/passwd"))
            .expect_err("越权路径必须被拦截");
        assert!(matches!(err, AiError::PathViolation { .. }));
    }

    #[cfg(unix)]
    #[test]
    fn sandbox_creates_owner_only_root() {
        use std::os::unix::fs::PermissionsExt as _;

        let base = tempfile::tempdir().expect("创建临时目录失败");
        let root = base.path().join("ai-private");
        resolve_sandbox_path(&root, Path::new("entry.txt")).expect("解析沙箱路径失败");
        let mode = std::fs::metadata(root)
            .expect("读取沙箱属性失败")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700);
    }

    #[cfg(unix)]
    #[test]
    fn sandbox_rejects_symlink_root() {
        use std::os::unix::fs::symlink;

        let base = tempfile::tempdir().expect("创建临时目录失败");
        let outside = tempfile::tempdir().expect("创建外部目录失败");
        let root = base.path().join("linked-root");
        symlink(outside.path(), &root).expect("创建符号链接失败");
        let err = resolve_sandbox_path(&root, Path::new("entry.txt"))
            .expect_err("符号链接根目录必须被拦截");
        assert!(matches!(err, AiError::PathViolation { .. }));
    }

    #[cfg(unix)]
    #[test]
    fn sandbox_rejects_symlink_escape() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().expect("创建沙箱目录失败");
        let outside = tempfile::tempdir().expect("创建外部目录失败");
        symlink(outside.path(), root.path().join("link")).expect("创建符号链接失败");
        let err = resolve_sandbox_path(root.path(), Path::new("link/secret"))
            .expect_err("符号链接逃逸必须被拦截");
        assert!(matches!(err, AiError::PathViolation { .. }));
    }

    #[test]
    fn append_host_fact_formats_clean_markdown() {
        let initial = append_host_fact("", "Nginx 配置位于 /etc/nginx/conf.d", "路径");
        assert!(initial.contains("- [路径] Nginx 配置位于 /etc/nginx/conf.d"));
        let second = append_host_fact(&initial, "禁止重启数据库", "");
        assert!(second.contains("- 禁止重启数据库"));
    }
}
