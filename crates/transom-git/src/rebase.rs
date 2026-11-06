//! 交互式变基：取回待办清单，并按用户编辑后的清单一次执行完。
//!
//! 设计原理（Why）：
//! - 清单来自 `git log --reverse <base>..HEAD`，顺序与 git 自己的 todo 相同
//!   （旧→新），界面只做「移动与改动作」，不重排语义；
//! - 执行不弹编辑器：把清单写成临时文件，用 `GIT_SEQUENCE_EDITOR="cp <文件>"`
//!   让 git 用我们的内容覆盖它的 todo，`GIT_EDITOR=true` 接受 git 预填的合并说明。
//!   实测（本机 git 2.53）产物与手工交互式变基一致，且不需要往环境里塞脚本；
//! - 不支持 `edit`：它会把变基停在中间，需要「继续/中止」的现场控制，
//!   那是另一条能力；这里放出 `edit` 只会把用户带进界面处理不完的状态；
//! - 冲突时不自动中止：git 会停在冲突处，界面保留既有的「中止变基」入口，
//!   由用户决定继续处理还是回退——自动回滚会吞掉用户本可以手工解决的冲突。
use crate::error::GitError;
use crate::fields::{lossy, split_limited};
use crate::process::{RunOptions, run};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use transom_core::{GitRebaseAction, GitRebaseStep};

/// 记录分隔符（与日志侧同一套控制字符约定）。
const RECORD: u8 = 0x1e;
/// 字段分隔符。
const FIELD: u8 = 0x1f;

/// 取 `<base>..HEAD` 的待办清单（旧→新）。
///
/// # Errors
/// 基线不是合法提交号、子进程失败或输出形态不符时返回 [`GitError`]。
pub(crate) async fn todo(root: &Path, base: &str) -> Result<Vec<GitRebaseStep>, GitError> {
    check_rev(base)?;
    let range = format!("{base}..HEAD");
    let output = run(
        root,
        &[
            "log",
            "--reverse",
            "--format=%x1e%H%x1f%s",
            "--end-of-options",
            &range,
        ],
        RunOptions::default(),
    )
    .await?;
    parse(&output.stdout)
}

/// 把清单渲染成 git todo 文本。
pub(crate) fn todo_text(steps: &[GitRebaseStep]) -> String {
    let mut text = String::with_capacity(steps.len() * 48);
    for step in steps {
        text.push_str(step.action.keyword());
        text.push(' ');
        text.push_str(&step.oid);
        text.push('\n');
    }
    text
}

/// 按清单执行变基。
///
/// # Errors
/// 清单为空以外的任何子进程失败都原样上报；发生冲突时仓库会停在变基中间状态，
/// 由界面「中止变基」收尾。
pub(crate) async fn run_todo(
    root: &Path,
    base: &str,
    steps: &[GitRebaseStep],
) -> Result<(), GitError> {
    check_rev(base)?;
    if steps.is_empty() {
        return Ok(());
    }
    if steps
        .iter()
        .any(|step| !is_hex(step.oid.as_bytes()) || step.oid.len() != 40)
    {
        return Err(GitError::Decode {
            command: "rebase -i".into(),
            reason: "待办清单含非法提交号".into(),
        });
    }
    let file = TodoFile::create()?;
    std::fs::write(file.path(), todo_text(steps)).map_err(|source| GitError::Spawn {
        command: format!("写入变基待办清单 {}", file.path().display()),
        source,
    })?;
    run(
        root,
        &["rebase", "-i", "--end-of-options", base],
        RunOptions::with_todo(file.path().to_path_buf()),
    )
    .await?;
    Ok(())
}

/// 解析 `--format=%x1e%H%x1f%s` 的清单。
fn parse(bytes: &[u8]) -> Result<Vec<GitRebaseStep>, GitError> {
    let mut steps = Vec::with_capacity(bytes.iter().filter(|b| **b == RECORD).count());
    for record in bytes.split(|b| *b == RECORD) {
        if record.is_empty() {
            continue;
        }
        // `--pretty=format:` 把换行插在记录**之后**（不是之前），因此剥尾不剥头：
        // 剥错方向会让每条标题都带一个尾随换行，界面上就是一行提交撑成两行。
        let body = match record.split_last() {
            Some((b'\n', rest)) => rest,
            _ => record,
        };
        let fields = split_limited(body, FIELD, 2);
        if fields.len() < 2 {
            return Err(GitError::Decode {
                command: "rebase todo".into(),
                reason: format!("待办记录字段不足: {}", lossy(body)),
            });
        }
        steps.push(GitRebaseStep {
            action: GitRebaseAction::Pick,
            oid: lossy(fields[0]),
            subject: lossy(fields[1]),
        });
    }
    Ok(steps)
}

/// 修订号必须是十六进制提交号：界面只会把日志里的提交传进来，
/// 但这一层不信任调用方，`-` 开头的值会被 git 当成选项。
fn check_rev(rev: &str) -> Result<(), GitError> {
    if rev.is_empty() || !is_hex(rev.as_bytes()) {
        return Err(GitError::InvalidPath {
            path: rev.to_string(),
            reason: "变基基线必须是十六进制提交号".into(),
        });
    }
    Ok(())
}

/// 是否全为十六进制字符。
fn is_hex(bytes: &[u8]) -> bool {
    bytes.iter().all(|b| b.is_ascii_hexdigit())
}

/// 待办清单临时文件：离开作用域即删除，避免在 /tmp 里留下历史改写的痕迹。
struct TodoFile(PathBuf);

impl TodoFile {
    fn create() -> Result<Self, GitError> {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or_default();
        let path = std::env::temp_dir().join(format!(
            "transom-rebase-{}-{nanos:x}.todo",
            std::process::id()
        ));
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TodoFile {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_file(&self.0)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            log::warn!(
                "清理变基待办清单失败，路径: {}，原因: {error}",
                self.0.display()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{check_rev, todo_text};
    use transom_core::{GitRebaseAction, GitRebaseStep};

    fn step(action: GitRebaseAction, oid: &str) -> GitRebaseStep {
        GitRebaseStep {
            action,
            oid: oid.to_string(),
            subject: String::new(),
        }
    }

    #[test]
    fn todo_text_keeps_order_and_keywords() {
        let oid = "0123456789abcdef0123456789abcdef01234567";
        let text = todo_text(&[
            step(GitRebaseAction::Pick, oid),
            step(GitRebaseAction::Squash, &"b".repeat(40)),
            step(GitRebaseAction::Drop, &"c".repeat(40)),
        ]);
        assert_eq!(
            text,
            format!(
                "pick {oid}\nsquash {}\ndrop {}\n",
                "b".repeat(40),
                "c".repeat(40)
            )
        );
    }

    #[test]
    fn rejects_non_hex_baseline() {
        assert!(check_rev("--exec=rm -rf /").is_err());
        assert!(check_rev("").is_err());
        assert!(check_rev(&"a".repeat(40)).is_ok());
    }
}
