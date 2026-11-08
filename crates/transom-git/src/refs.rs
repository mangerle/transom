//! 分支与标签清单（`git for-each-ref`）。
//!
//! 设计原理（Why）：
//! - `for-each-ref` 不支持 `-z`（本机 git 2.53 实测报「未知开关 z」），因此
//!   记录之间用 `%1e` 控制字符显式收尾，比按换行切分更抗字段内嵌换行；
//! - 不取 `%(ahead-behind:HEAD)`：该占位符对每个引用都要跑一趟可达性计算，
//!   分支多的仓库会明显卡顿；当前分支的领先/落后已由 `status` 的
//!   `# branch.ab` 头行给出，列表只需要上游名字。

use crate::config::GitConfig;
use crate::error::GitError;
use crate::fields::{lossy, split_limited};
use crate::process::{RunOptions, run};
use crate::types::{BranchEntry, TagEntry};
use std::path::Path;

/// 剥掉记录开头的换行（`\r\n` 与 `\n` 都要处理，Windows 上 git 会给出 CRLF）。
fn trim_leading_newlines(record: &[u8]) -> &[u8] {
    let mut start = 0;
    while matches!(record.get(start), Some(b'\n' | b'\r')) {
        start += 1;
    }
    &record[start..]
}

/// 记录终止符，避免引用内容里的换行干扰切分。
const RECORD: &str = "%1e";

/// 列出本地与远程分支。
pub(crate) async fn branches(
    root: &Path,
    config: &GitConfig,
) -> Result<Vec<BranchEntry>, GitError> {
    let format = format!(
        "%(refname){}%(objectname){}%(HEAD){}%(upstream:short){RECORD}",
        "%1f", "%1f", "%1f"
    );
    let output = run(
        root,
        config,
        &[
            "for-each-ref",
            &format!("--format={format}"),
            "refs/heads",
            "refs/remotes",
        ],
        RunOptions::default(),
    )
    .await?;
    let mut entries = Vec::new();
    for record in output.stdout.split(|b| *b == 0x1e) {
        // for-each-ref 会在每条记录后补一个换行，落在下一条记录的开头，必须先剥掉。
        let record = trim_leading_newlines(record);
        if record.is_empty() {
            continue;
        }
        let fields = split_limited(record, 0x1f, 4);
        if fields.len() < 4 {
            return Err(GitError::Decode {
                command: "for-each-ref".into(),
                reason: format!("引用记录字段不足: {}", lossy(record)),
            });
        }
        let full_name = lossy(fields[0]);
        // 不使用 `%(refname:short)`：本地分支名为 `origin` 时会得到歧义的
        // `heads/origin`，远端 HEAD 还可能被压成 `origin`。直接按完整引用名剥离前缀，
        // 才能稳定区分本地分支、远程分支与同名引用。
        let (name, is_remote) = if let Some(name) = full_name.strip_prefix("refs/heads/") {
            (name, false)
        } else if let Some(name) = full_name.strip_prefix("refs/remotes/") {
            // `refs/remotes/origin/HEAD` 是远端默认分支的符号引用，不是可检出分支。
            if name.ends_with("/HEAD") {
                continue;
            }
            (name, true)
        } else {
            continue;
        };
        entries.push(BranchEntry {
            remote: is_remote,
            current: lossy(fields[2]).trim() == "*",
            name: name.to_string(),
            oid: lossy(fields[1]),
            upstream: Some(lossy(fields[3])).filter(|text| !text.is_empty()),
            ahead: 0,
            behind: 0,
        });
    }
    Ok(entries)
}

/// 列出标签，附注标签解引用到目标提交。
pub(crate) async fn tags(root: &Path, config: &GitConfig) -> Result<Vec<TagEntry>, GitError> {
    let format =
        format!("%(refname:short)%1f%(objectname)%1f%(*objectname)%1f%(contents:subject){RECORD}");
    let output = run(
        root,
        config,
        &["for-each-ref", &format!("--format={format}"), "refs/tags"],
        RunOptions::default(),
    )
    .await?;
    let mut entries = Vec::new();
    for record in output.stdout.split(|b| *b == 0x1e) {
        let record = trim_leading_newlines(record);
        if record.is_empty() {
            continue;
        }
        let fields = split_limited(record, 0x1f, 4);
        if fields.len() < 4 {
            return Err(GitError::Decode {
                command: "for-each-ref".into(),
                reason: format!("标签记录字段不足: {}", lossy(record)),
            });
        }
        // `*objectname` 只有附注标签才有值；轻量标签的目标就是自身。
        let peeled = lossy(fields[2]);
        let annotated = !peeled.is_empty();
        // 注意 `%(contents:subject)` 对轻量标签会回落到所指提交的标题，
        // 因此只有附注标签才把该字段当作标签说明，否则界面会给每个轻量标签
        // 挂上一句提交信息，与 IntelliJ 的展示口径不符。
        let contents = lossy(fields[3]);
        entries.push(TagEntry {
            name: lossy(fields[0]),
            oid: lossy(fields[1]),
            target: if annotated { peeled } else { lossy(fields[1]) },
            message: annotated
                .then_some(contents)
                .filter(|text| !text.is_empty()),
        });
    }
    Ok(entries)
}
