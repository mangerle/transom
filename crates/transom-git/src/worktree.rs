//! 工作树列表、锁定与删除。
//!
//! 设计原理（Why）：`git worktree list --porcelain` 是唯一稳定可解析的形态
//! （每段 `worktree <路径>` / `HEAD <号>` / `branch <引用>` / `bare` / `detached`
//! / `locked "<原因>"` / `prunable <原因>`），表格输出会随终端宽度折行，不适合机器读。
//! 锁定原因走 C 风格引号（`\"`、`\\`、非 ASCII 用八位转义，本机 git 2.53 实测），
//! 而 `prunable` 的原因是裸文本到行尾——两者规则不同，必须分开处理。

use crate::error::GitError;
use crate::process::{RunOptions, run};
use std::path::{Path, PathBuf};
use transom_core::WorktreeEntry;

/// 列出全部工作树（主工作树在首）。
pub(crate) async fn list(root: &Path) -> Result<Vec<WorktreeEntry>, GitError> {
    let output = crate::process::run(
        root,
        &["worktree", "list", "--porcelain"],
        RunOptions::default(),
    )
    .await?;
    let mut entries: Vec<WorktreeEntry> = Vec::new();
    let mut current: Option<WorktreeEntry> = None;
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            if let Some(entry) = current.take() {
                entries.push(entry);
            }
            current = Some(WorktreeEntry {
                path: PathBuf::from(path),
                head: String::new(),
                branch: None,
                detached: false,
                bare: false,
                locked: false,
                lock_reason: None,
                prune_reason: None,
            });
            continue;
        }
        let Some(entry) = current.as_mut() else {
            continue;
        };
        if let Some(head) = line.strip_prefix("HEAD ") {
            entry.head = head.to_string();
        } else if let Some(reference) = line.strip_prefix("branch ") {
            entry.branch = Some(
                reference
                    .strip_prefix("refs/heads/")
                    .unwrap_or(reference)
                    .to_string(),
            );
        } else if line == "detached" {
            entry.detached = true;
        } else if line == "bare" {
            entry.bare = true;
        } else if let Some(reason) = line.strip_prefix("locked ") {
            // 带原因的锁：git 用 C 风格引号包住原因文本。
            entry.locked = true;
            entry.lock_reason = Some(unquote(reason));
        } else if line == "locked" {
            entry.locked = true;
        } else if let Some(reason) = line.strip_prefix("prunable ") {
            // 可清理的原因文本原样到行尾，不做二次解释。
            entry.prune_reason = Some(reason.to_string());
        }
    }
    if let Some(entry) = current.take() {
        entries.push(entry);
    }
    for entry in &entries {
        log::trace!(
            "工作树: {}，分支: {:?}，锁定: {}，可清理: {:?}",
            entry.path.display(),
            entry.branch,
            entry.locked,
            entry.prune_reason
        );
    }
    Ok(entries)
}

/// 解开 git 的 C 风格引号字符串：`"a\"b\\c"`、`"\344\270\264..."`。
///
/// 设计原理（Why）：锁定原因由用户自己敲，可能是中文，也可能是带引号或反斜杠的
/// 文本；git 会把它包进双引号并按 C 转义。不还原就会在界面上显示成
/// `"\344\270\264..."` 这种八位乱码。非引号形态原样返回，兼容将来 git 的输出变化。
fn unquote(raw: &str) -> String {
    let inner = raw.strip_prefix('"').and_then(|s| s.strip_suffix('"'));
    let Some(inner) = inner else {
        return raw.to_string();
    };
    let bytes = inner.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0usize;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte != b'\\' || index + 1 >= bytes.len() {
            out.push(byte);
            index += 1;
            continue;
        }
        let next = bytes[index + 1];
        match next {
            b'n' => {
                out.push(b'\n');
                index += 2;
            }
            b't' => {
                out.push(b'\t');
                index += 2;
            }
            b'r' => {
                out.push(b'\r');
                index += 2;
            }
            b'\\' | b'"' | b'\'' => {
                out.push(next);
                index += 2;
            }
            // 八位转义：最多三位数字，攒成一个字节。
            b'0'..=b'7' => {
                let mut value: u32 = 0;
                let mut taken = 0usize;
                while taken < 3 && index + 1 + taken < bytes.len() {
                    match bytes[index + 1 + taken] {
                        digit @ b'0'..=b'7' => {
                            value = value * 8 + u32::from(digit - b'0');
                            taken += 1;
                        }
                        _ => break,
                    }
                }
                out.push(value as u8);
                index += 1 + taken;
            }
            // 认不出的转义按字面保留，避免把内容吃掉。
            other => {
                out.push(b'\\');
                out.push(other);
                index += 2;
            }
        }
    }
    // 字节序列按 UTF-8 还原；万一不是合法 UTF-8，退回原始文本而不是报错，
    // 因为这是展示用的原因字符串，不值得让整个工作树列表失败。
    String::from_utf8(out)
        // 字节流不合法时按 lossy 还原已解码部分，而不是丢掉整条原因。
        .unwrap_or_else(|broken| String::from_utf8_lossy(broken.as_bytes()).to_string())
}

/// 锁定一个工作树（可带原因）：锁定后 git 拒绝删除，也不会被 prune 掉。
pub(crate) async fn lock(root: &Path, path: &str, reason: Option<&str>) -> Result<(), GitError> {
    let mut args: Vec<&str> = vec!["worktree", "lock"];
    if let Some(reason) = reason {
        args.push("--reason");
        args.push(reason);
    }
    args.push(path);
    run(root, &args, RunOptions::write()).await?;
    Ok(())
}

/// 解锁一个工作树。
pub(crate) async fn unlock(root: &Path, path: &str) -> Result<(), GitError> {
    run(root, &["worktree", "unlock", path], RunOptions::write()).await?;
    Ok(())
}

/// 删除一个工作树的登记（不删文件由 git 自己处理：`remove` 会删目录）。
pub(crate) async fn remove(root: &Path, path: &str, force: bool) -> Result<(), GitError> {
    let mut args: Vec<&str> = vec!["worktree", "remove"];
    if force {
        args.push("--force");
    }
    args.push(path);
    run(root, &args, RunOptions::write()).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::unquote;

    #[test]
    fn unquote_decodes_c_style_escapes() {
        assert_eq!(unquote("\"a\\\"b\\\\c\""), "a\"b\\c");
        assert_eq!(unquote("\"第一行\\n第二行\\t\""), "第一行\n第二行\t");
    }

    #[test]
    fn unquote_decodes_octal_bytes_into_utf8() {
        // git 对中文原因的输出形态（本机 git 2.53 实测）。
        assert_eq!(unquote("\"\\344\\270\\264\\346\\227\\266\""), "临时");
    }

    #[test]
    fn unquote_passes_through_unquoted_text() {
        assert_eq!(unquote("没被引号包住"), "没被引号包住");
        assert_eq!(unquote("\"带个尾引号"), "\"带个尾引号");
    }

    #[test]
    fn unquote_keeps_unknown_escapes() {
        assert_eq!(unquote("\"a\\qb\""), "a\\qb");
    }
}
