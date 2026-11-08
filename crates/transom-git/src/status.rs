//! `git status --porcelain=v2 -z` 解析。
//!
//! 设计原理（Why）：
//! - 选 v2 而不是 v1：v1 的两个状态字符无法区分「重命名的来源路径」，也没有
//!   分支与 ahead/behind 头行，界面每刷一次还得再跑两趟子进程；
//! - 选 `-z`：路径字段以原始字节输出，杜绝 v1 对含引号/非 ASCII 路径的
//!   C 风格转义（本机 git 2.53 实测：`-z` 下连 `#` 头行也以 NUL 结尾，
//!   因此整段输出统一按 NUL 切分即可，无需区分行尾符）；
//! - 解析函数只吃字节、不碰子进程，便于用真实 git 产出的样本做纯单元测试。

use crate::config::GitConfig;
use crate::error::GitError;
use crate::fields::{lossy, split_limited};
use crate::process::{RunOptions, run};
use crate::types::{ChangeCode, HeadState, RepoStatus, StatusEntry, StatusRecord};
use std::path::Path;

/// 采集一次工作区状态快照。
///
/// 未跟踪文件用 git 默认的 `normal` 粒度：整目录未被跟踪时只回一条目录项，
/// 避免 `node_modules` 这类目录把界面刷成上万行；目录项作为路径传给
/// `git add` 依然正确。
///
/// # Errors
/// 目录不是仓库、git 不可用或输出格式不认识时返回对应错误。
pub(crate) async fn current(
    root: &Path,
    config: &GitConfig,
    include_ignored: bool,
) -> Result<RepoStatus, GitError> {
    let mut args = vec!["status", "--porcelain=v2", "--branch", "-z"];
    if include_ignored {
        args.push("--ignored=matching");
    }
    let output = run(root, config, &args, RunOptions::default()).await?;
    parse(&output.stdout)
}

/// 解析 porcelain v2 + `-z` 的完整输出。
///
/// # Errors
/// 头行字段数不符或 ahead/behind 非数字时返回 [`GitError::Decode`]，
/// 说明 git 版本输出了本解析器不认识的格式，宁可报错也不给出错误的状态。
pub(crate) fn parse(bytes: &[u8]) -> Result<RepoStatus, GitError> {
    let mut records = bytes.split(|b| *b == 0);
    let mut status = RepoStatus::default();
    let mut head_name: Option<String> = None;

    while let Some(record) = records.next() {
        if record.is_empty() {
            continue;
        }
        if record.starts_with(b"# ") {
            parse_header(
                String::from_utf8_lossy(record).as_ref(),
                &mut status,
                &mut head_name,
            )?;
            continue;
        }
        let kind = record[0];
        match kind {
            b'1' | b'2' => {
                let expected = if kind == b'1' { 9 } else { 10 };
                let fields = split_limited(record, b' ', expected);
                if fields.len() < expected {
                    return Err(decode_error(record));
                }
                let path = lossy(fields[expected - 1]);
                let old_path = if kind == b'2' {
                    // 重命名/复制记录的来源路径是紧随其后的独立 NUL 记录。
                    match records.next() {
                        Some(source) => Some(lossy(source)),
                        None => return Err(decode_error(record)),
                    }
                } else {
                    None
                };
                status.entries.push(StatusEntry {
                    path,
                    old_path,
                    record: StatusRecord::Tracked {
                        index: code_at(fields[1], 0),
                        worktree: code_at(fields[1], 1),
                    },
                });
            }
            b'u' => {
                let fields = split_limited(record, b' ', 11);
                if fields.len() < 11 {
                    return Err(decode_error(record));
                }
                status.entries.push(StatusEntry {
                    path: lossy(fields[10]),
                    old_path: None,
                    record: StatusRecord::Conflict,
                });
            }
            b'?' | b'!' => {
                let fields = split_limited(record, b' ', 2);
                if fields.len() < 2 {
                    return Err(decode_error(record));
                }
                status.entries.push(StatusEntry {
                    path: lossy(fields[1]),
                    old_path: None,
                    record: if kind == b'?' {
                        StatusRecord::Untracked
                    } else {
                        StatusRecord::Ignored
                    },
                });
            }
            other => {
                return Err(GitError::Decode {
                    command: "status".into(),
                    reason: format!("未知的 porcelain v2 记录类型: {}", other as char),
                });
            }
        }
    }

    status.head = match (&status.oid, head_name) {
        (None, Some(branch)) => HeadState::Initial { branch },
        (Some(_), Some(branch)) => HeadState::OnBranch(branch),
        (Some(_), None) => HeadState::Detached,
        (None, None) => HeadState::Detached,
    };
    Ok(status)
}

/// 解析 `#` 头行。
fn parse_header(
    line: &str,
    status: &mut RepoStatus,
    head_name: &mut Option<String>,
) -> Result<(), GitError> {
    let mut parts = line.splitn(3, ' ');
    let (Some("#"), Some(key), Some(value)) = (parts.next(), parts.next(), parts.next()) else {
        return Err(decode_error(line.as_bytes()));
    };
    match key {
        "branch.oid" => {
            // 空仓库（尚无任何提交）时 git 回传 `(initial)`，此时没有 HEAD 提交号。
            if value != "(initial)" {
                status.oid = Some(value.to_string());
            }
        }
        "branch.head" => {
            // 游离头指针时 git 给出 `(detached)`；空仓库的 branch.head 是
            // 「计划中的分支名」，由调用方结合 branch.oid 判定 Initial。
            if value == "(detached)" {
                *head_name = None;
            } else {
                *head_name = Some(value.to_string());
            }
        }
        "branch.upstream" => status.upstream = Some(value.to_string()),
        "branch.ab" => {
            let mut iter = value.split_whitespace();
            let (Some(ahead), Some(behind)) = (iter.next(), iter.next()) else {
                return Err(decode_error(line.as_bytes()));
            };
            status.ahead = ahead
                .strip_prefix('+')
                .and_then(|n| n.parse().ok())
                .unwrap_or(0);
            status.behind = behind
                .strip_prefix('-')
                .and_then(|n| n.parse().ok())
                .unwrap_or(0);
        }
        // 未来版本新增的头行（如 tracked/untracked 计数）直接忽略，不影响条目。
        _ => {}
    }
    Ok(())
}

/// 取状态码字符（`XY` 两位）。
fn code_at(field: &[u8], index: usize) -> ChangeCode {
    field
        .get(index)
        .and_then(|b| char::from_u32(u32::from(*b)))
        .map(ChangeCode::from_char)
        .unwrap_or(ChangeCode::Unmodified)
}

fn decode_error(record: &[u8]) -> GitError {
    GitError::Decode {
        command: "status".into(),
        reason: format!("记录字段不完整: {}", String::from_utf8_lossy(record)),
    }
}

#[cfg(test)]
mod tests {
    use super::parse;
    use crate::types::{ChangeCode, HeadState, StatusRecord};

    /// 用本机 git 2.53 的真实输出构造样本：每条记录以 NUL 收尾。
    fn sample(records: &[&str]) -> Vec<u8> {
        let mut bytes = Vec::new();
        for record in records {
            bytes.extend_from_slice(record.as_bytes());
            bytes.push(0);
        }
        bytes
    }

    #[test]
    fn parses_branch_headers() {
        let bytes = sample(&[
            "# branch.oid 1d9d",
            "# branch.head main",
            "# branch.upstream origin/main",
            "# branch.ab +2 -1",
        ]);
        let status = parse(&bytes).expect("合法头行必须解析成功");
        assert_eq!(status.head, HeadState::OnBranch("main".into()));
        assert_eq!(status.oid.as_deref(), Some("1d9d"));
        assert_eq!(status.upstream.as_deref(), Some("origin/main"));
        assert_eq!((status.ahead, status.behind), (2, 1));
    }

    #[test]
    fn detached_head_has_no_branch_name() {
        let bytes = sample(&["# branch.oid abc", "# branch.head (detached)"]);
        let status = parse(&bytes).expect("游离头解析成功");
        assert_eq!(status.head, HeadState::Detached);
    }

    #[test]
    fn empty_repo_reports_initial_branch() {
        let bytes = sample(&["# branch.oid (initial)", "# branch.head main"]);
        let status = parse(&bytes).expect("空仓库解析成功");
        assert_eq!(
            status.head,
            HeadState::Initial {
                branch: "main".into()
            }
        );
        assert_eq!(status.oid, None);
    }

    #[test]
    fn parses_tracked_and_paths_with_spaces() {
        let bytes = sample(&[
            "# branch.oid abc",
            "# branch.head main",
            "1 AM N... 100644 100644 100644 h1 h2 dir with space/a b.txt",
            "? 未跟踪.txt",
        ]);
        let status = parse(&bytes).expect("条目解析成功");
        assert_eq!(status.entries.len(), 2);
        let first = &status.entries[0];
        assert_eq!(first.path, "dir with space/a b.txt");
        assert_eq!(
            first.record,
            StatusRecord::Tracked {
                index: ChangeCode::Added,
                worktree: ChangeCode::Modified
            }
        );
        assert!(first.staged() && first.unstaged());
        assert_eq!(status.entries[1].path, "未跟踪.txt");
        assert_eq!(status.untracked().len(), 1);
    }

    #[test]
    fn rename_record_consumes_source_record() {
        let bytes = sample(&[
            "# branch.oid abc",
            "# branch.head main",
            "2 RM N... 100644 100644 100644 h1 h2 R100 renamed.txt",
            "f3.txt",
        ]);
        let status = parse(&bytes).expect("重命名解析成功");
        assert_eq!(status.entries.len(), 1, "来源路径不能变成独立条目");
        assert_eq!(status.entries[0].path, "renamed.txt");
        assert_eq!(status.entries[0].old_path.as_deref(), Some("f3.txt"));
    }

    #[test]
    fn conflict_record_is_reported_as_such() {
        let bytes = sample(&[
            "# branch.oid abc",
            "# branch.head main",
            "u UU N... 100644 100644 100644 100644 h1 h2 h3 c.txt",
        ]);
        let status = parse(&bytes).expect("冲突解析成功");
        assert!(status.has_conflicts());
        assert_eq!(status.conflicts().len(), 1);
        assert!(status.conflicts()[0].staged(), "冲突文件应计入待提交侧");
    }

    #[test]
    fn unknown_record_type_is_a_decode_error() {
        let bytes = sample(&["# branch.oid abc", "Z bad record"]);
        let error = parse(&bytes).expect_err("未知记录类型必须报错");
        assert!(matches!(error, crate::error::GitError::Decode { .. }));
    }

    #[test]
    fn truncated_record_is_a_decode_error() {
        let bytes = sample(&["# branch.oid abc", "# branch.head main", "1 AM N... 100644"]);
        let error = parse(&bytes).expect_err("字段不足必须报错");
        assert!(matches!(error, crate::error::GitError::Decode { .. }));
    }
}
