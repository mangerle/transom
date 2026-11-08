//! `git log` 分页查询、提交文件清单与输出解析。
//!
//! 设计原理（Why）：
//! - 字段分隔用 `%x1f`、记录分隔用 `%x1e` 两个控制字符，而不是任何可打印串：
//!   提交正文可以包含任意文本，只有控制字符才不会被标题/正文撞车；
//! - 时间取 `%at`（Unix 秒）而不是 `%aI`：省掉一次时区字符串解析，界面按本地
//!   时区渲染即可，与 IntelliJ 的显示口径一致；
//! - 排序固定 `--topo-order`：提交图的列分配要求「父提交一定排在子提交之后」，
//!   默认的日期序会违反该前提，导致图上出现指向尚未出现节点的边；
//! - 分页用 `--skip/--max-count` 且多取一条作探针：判断「还有更多」不必再跑
//!   一趟子进程，也不把整个历史读进内存。
//!
//! 代价与局限：正文里恰好含 `\x1f` 的提交会被多切一刀，解析时把多出的段并回
//! 正文，不丢数据；`--grep` 走固定字符串匹配，正则检索暂不提供。

use crate::config::GitConfig;
use crate::error::GitError;
use crate::fields::{lossy, split_limited};
use crate::process::{RunOptions, run};
use crate::repo;
use crate::types::{CommitEntry, CommitFile, LogPage, LogQuery, LogScope, RefDecor, RefKind};
use std::path::Path;

/// 字段分隔符（Unit Separator）。
const FIELD: u8 = 0x1f;
/// 记录分隔符（Record Separator）。
const RECORD: u8 = 0x1e;
/// 字段数：oid/parents/作者名/邮箱/时间/装饰/标题/正文。
const FIELD_COUNT: usize = 8;
/// `--pretty=format:` 的字段顺序，必须与 [`FIELD_COUNT`] 及解析顺序一致。
pub(crate) const FORMAT: &str = "%x1e%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%D%x1f%s%x1f%b";

/// 执行一次日志查询。
///
/// # Errors
/// 起始修订不存在、仓库状态异常或输出格式不认识时返回对应 [`GitError`]。
pub(crate) async fn query(
    root: &Path,
    config: &GitConfig,
    query: &LogQuery,
) -> Result<LogPage, GitError> {
    let mut args: Vec<String> = vec![
        "log".into(),
        format!("--pretty=format:{FORMAT}"),
        "--topo-order".into(),
        "--no-color".into(),
        format!("--skip={}", query.skip),
        // 多取一条作为「是否还有下一页」的探针。
        format!("--max-count={}", query.limit.saturating_add(1)),
    ];
    match &query.scope {
        LogScope::CurrentBranch => {
            args.push(query.head.clone().unwrap_or_else(|| "HEAD".into()));
        }
        // `--all` 已覆盖 heads/tags/remotes，不再重复追加范围参数。
        LogScope::AllRefs => args.push("--all".into()),
        LogScope::Ref(name) => args.push(name.clone()),
    }
    if let Some(search) = query.search.as_deref().filter(|text| !text.is_empty()) {
        if !query.case_sensitive {
            args.push("-i".into());
        }
        // 正则与固定字符串二选一：默认固定字符串，`.*` 开关才切到正则。
        args.push(if query.regex {
            "--extended-regexp".into()
        } else {
            "--fixed-strings".into()
        });
        args.push(format!("--grep={search}"));
    }
    if let Some(since) = query.since.as_deref().filter(|text| !text.is_empty()) {
        args.push(format!("--since={since}"));
    }
    if let Some(until) = query.until.as_deref().filter(|text| !text.is_empty()) {
        args.push(format!("--until={until}"));
    }
    if let Some(author) = query.author.as_deref().filter(|text| !text.is_empty()) {
        args.push(format!("--author={author}"));
    }
    if !query.paths.is_empty() {
        args.push("--".into());
        args.extend(query.paths.iter().cloned());
    }

    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let output = run(root, config, &refs, RunOptions::default()).await?;
    let mut page = parse_records(&output.stdout)?;
    if page.entries.len() > query.limit {
        page.entries.truncate(query.limit);
        page.has_more = true;
    }
    Ok(page)
}

/// 查询单个提交改动的文件清单。
///
/// 合并提交默认不产出改动（combined diff 语义），这里统一按「相对第一父提交」
/// 展示，与 IntelliJ 的默认口径一致；根提交与空树比较。
pub(crate) async fn commit_files(
    root: &Path,
    config: &GitConfig,
    oid: &str,
) -> Result<Vec<CommitFile>, GitError> {
    let mut args: Vec<String> = vec![
        "diff-tree".into(),
        "--no-commit-id".into(),
        "--name-status".into(),
        "-r".into(),
        "-z".into(),
    ];
    if repo::resolve(root, config, &format!("{oid}^1"))
        .await?
        .is_some()
    {
        args.push(format!("{oid}^1"));
    } else {
        args.push("--root".into());
    }
    args.push(oid.to_string());
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let output = run(root, config, &refs, RunOptions::default()).await?;
    parse_name_status(&output.stdout).map_err(|reason| GitError::Decode {
        command: "diff-tree".into(),
        reason,
    })
}

/// 跟踪文件清单：日志的路径过滤下拉需要它，一次 `ls-files` 足够。
pub(crate) async fn tracked_files(
    root: &Path,
    config: &GitConfig,
) -> Result<Vec<String>, GitError> {
    let output = crate::process::run(root, config, &["ls-files"], RunOptions::default()).await?;
    Ok(output
        .lines()
        .into_iter()
        .filter(|line| !line.trim().is_empty())
        .collect())
}

/// 解析 `diff-tree --name-status -z`：`<状态>\0<路径>\0`，重命名/复制为
/// `<状态R100>\0<旧路径>\0<新路径>\0`。
///
/// `git stash show --name-status -z` 用的是同一套 NUL 分隔形态，故对搁置清单
/// 解析也复用这一条，避免两处各写一遍而漂移。
pub(crate) fn parse_name_status(bytes: &[u8]) -> Result<Vec<CommitFile>, String> {
    use crate::types::ChangeCode;
    let mut records = bytes.split(|b| *b == 0).filter(|r| !r.is_empty());
    let mut files = Vec::with_capacity(bytes.iter().filter(|b| **b == 0).count() / 2);
    while let Some(status) = records.next() {
        let Some(first) = records.next() else {
            return Err(format!("状态 {} 缺少路径字段", lossy(status)));
        };
        let text = lossy(status);
        let code = text
            .chars()
            .next()
            .map(ChangeCode::from_char)
            .unwrap_or(ChangeCode::Unknown('?'));
        // 重命名/复制是「旧路径、新路径」两条连续字段：本机 git 2.53 实测
        // `--name-status -z` 与文本形态 `R100\0旧\0新` 同序，现名在第二条。
        // 取错就会让提交详情的差异指向一个提交里已不存在的路径。
        let (path, old_path) = if matches!(code, ChangeCode::Renamed | ChangeCode::Copied) {
            let Some(new_path) = records.next() else {
                return Err(format!("状态 {text} 缺少重命名后的路径字段"));
            };
            (lossy(new_path), Some(lossy(first)))
        } else {
            (lossy(first), None)
        };
        files.push(CommitFile {
            path,
            old_path,
            change: code,
        });
    }
    Ok(files)
}

/// 解析 `--pretty=format:` 字节流；`has_more` 由调用方按分页探针设置。
///
/// # Errors
/// 记录字段数不足时返回 [`GitError::Decode`]，宁可报错也不给出错误的提交。
pub(crate) fn parse_records(bytes: &[u8]) -> Result<LogPage, GitError> {
    let mut entries = Vec::with_capacity(bytes.iter().filter(|b| **b == RECORD).count());
    for record in bytes.split(|b| *b == RECORD) {
        if record.is_empty() {
            continue;
        }
        // `--pretty=format:` 在记录之间插入换行，最后一条没有；先剥掉这一个。
        let body = match record.split_last() {
            Some((b'\n', rest)) => rest,
            _ => record,
        };
        let fields = split_limited(body, FIELD, FIELD_COUNT);
        if fields.len() < FIELD_COUNT {
            return Err(GitError::Decode {
                command: "log".into(),
                reason: format!(
                    "记录字段不足（期望 {FIELD_COUNT}，实际 {}）: {}",
                    fields.len(),
                    lossy(record)
                ),
            });
        }
        let parents = lossy(fields[1]);
        entries.push(CommitEntry {
            oid: lossy(fields[0]),
            parents: parents.split_whitespace().map(str::to_string).collect(),
            author_name: lossy(fields[2]),
            author_email: lossy(fields[3]),
            author_time: lossy(fields[4]).trim().parse().unwrap_or_default(),
            refs: parse_decor(lossy(fields[5]).as_ref()),
            subject: lossy(fields[6]),
            // 正文是最后一段，内含分隔符时已被并回，这里只去掉行尾换行。
            body: lossy(fields[7]).trim_end_matches('\n').to_string(),
        });
    }
    Ok(LogPage {
        entries,
        has_more: false,
    })
}

/// 解析 `%D` 引用装饰：`HEAD -> main, origin/main, tag: v1.0`。
fn parse_decor(text: &str) -> Vec<RefDecor> {
    if text.trim().is_empty() {
        return Vec::new();
    }
    text.split(", ")
        .filter_map(|item| {
            let item = item.trim();
            if item.is_empty() {
                return None;
            }
            if let Some(name) = item.strip_prefix("tag: ") {
                return Some(RefDecor {
                    kind: RefKind::Tag,
                    name: name.to_string(),
                });
            }
            if let Some((head, target)) = item.split_once(" -> ")
                && head == "HEAD"
            {
                return Some(RefDecor {
                    kind: RefKind::HeadAt,
                    name: target.to_string(),
                });
            }
            Some(RefDecor {
                // git 的装饰里远程跟踪分支必带 `远程名/` 前缀，据此区分。
                kind: if item.contains('/') {
                    RefKind::RemoteBranch
                } else {
                    RefKind::Branch
                },
                name: item.to_string(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{parse_decor, parse_name_status, parse_records};
    use crate::types::{LogQuery, LogScope, RefKind};

    /// 复刻 git `--pretty=format:` 的真实产出：记录以 \x1e 开头，记录之间插入 \n。
    fn records(items: &[String]) -> Vec<u8> {
        items.join("\n").into_bytes()
    }

    fn record(fields: [&str; 8]) -> String {
        format!(
            "\u{1e}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}",
            fields[0], fields[1], fields[2], fields[3], fields[4], fields[5], fields[6], fields[7]
        )
    }

    #[test]
    fn parses_a_two_commit_history() {
        let bytes = records(&[
            record([
                "b2",
                "b1",
                "张三",
                "z@x",
                "1700000000",
                "HEAD -> main",
                "second",
                "",
            ]),
            record([
                "b1",
                "",
                "李四",
                "l@x",
                "1699999000",
                "tag: v1.0",
                "first: 中文",
                "正文一行\n第二行",
            ]),
        ]);
        let page = parse_records(&bytes).expect("两条记录解析成功");
        assert_eq!(page.entries.len(), 2);
        assert_eq!(page.entries[0].subject, "second");
        assert_eq!(page.entries[0].author_time, 1700000000);
        assert_eq!(page.entries[0].parents, vec!["b1".to_string()]);
        assert_eq!(page.entries[1].body, "正文一行\n第二行");
        assert_eq!(page.entries[1].refs[0].kind, RefKind::Tag);
        assert!(!page.entries[0].is_merge());
    }

    #[test]
    fn merge_lists_all_parents() {
        let bytes = records(&[record(["m", "p1 p2", "a", "a@b", "1", "", "merge", ""])]);
        let page = parse_records(&bytes).expect("合并记录解析成功");
        assert_eq!(page.entries[0].parents.len(), 2);
        assert!(page.entries[0].is_merge());
    }

    #[test]
    fn short_record_is_a_decode_error() {
        let bytes = records(&["\u{1e}only\u{1f}two".to_string()]);
        let error = parse_records(&bytes).expect_err("字段不足必须报错");
        assert!(matches!(error, crate::error::GitError::Decode { .. }));
    }

    #[test]
    fn decor_distinguishes_head_branch_and_remote() {
        let decor = parse_decor("HEAD -> main, origin/main, origin/HEAD, tag: v1.5.0");
        let kinds: Vec<_> = decor
            .iter()
            .map(|item| (item.kind, item.name.as_str()))
            .collect();
        assert_eq!(
            kinds,
            vec![
                (RefKind::HeadAt, "main"),
                (RefKind::RemoteBranch, "origin/main"),
                (RefKind::RemoteBranch, "origin/HEAD"),
                (RefKind::Tag, "v1.5.0"),
            ]
        );
    }

    #[test]
    fn empty_decor_yields_no_refs() {
        assert!(parse_decor("").is_empty());
    }

    #[test]
    fn default_query_pages_from_head() {
        let query = LogQuery::default();
        assert_eq!(query.scope, LogScope::CurrentBranch);
        assert_eq!(query.limit, 200);
    }

    #[test]
    fn name_status_keeps_new_path_for_renames() {
        // 本机 git 2.53 的 `--name-status -z` 实测形态：状态、旧路径、新路径。
        let bytes = b"M\0a.txt\0R100\0old dir/\xe5\xa4\xa7.txt\0new.txt\0D\0gone.txt\0";
        let files = parse_name_status(bytes).expect("清单解析成功");
        assert_eq!(files.len(), 3);
        assert_eq!(files[0].path, "a.txt");
        assert_eq!(files[0].old_path, None);
        assert_eq!(files[1].path, "new.txt", "重命名后的现名必须落在 path 上");
        assert_eq!(files[1].old_path.as_deref(), Some("old dir/大.txt"));
        assert_eq!(files[2].change, crate::types::ChangeCode::Deleted);
    }

    #[test]
    fn name_status_rejects_truncated_rename_record() {
        let bytes = b"R100\0only-old.txt\0";
        let err = parse_name_status(bytes).expect_err("缺新路径必须报错");
        assert!(err.contains("重命名后的路径"), "实际: {err}");
    }
}
