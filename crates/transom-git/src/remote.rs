//! 远程动作与工作树创建。
//!
//! 设计原理（Why）：
//! - 网络子命令单独放宽超时（120 秒）：本地查询 30 秒不通就算异常，但一次
//!   跨城 `git fetch` 正常就可能要几十秒，用同一个阈值会把正常操作判成故障；
//! - 凭据一律交给系统 git 的 credential helper 与 ssh-agent：本层显式关闭
//!   终端与 askpass 提示（见 `process.rs`），因此无凭据时会直接失败并把
//!   git 的原始提示回传界面，比在应用里再造一套口令存储更可排查；
//! - `worktree add` 的目录由 git 自己创建：应用只负责校验父目录存在，
//!   避免两边对「已存在目录」的判定规则不一致。

use crate::error::GitError;
use crate::process::{RunOptions, run, run_allow_fail};
use std::path::Path;
use std::time::Duration;

/// 网络动作超时：覆盖慢链路下的大仓库 fetch/push。
const NETWORK_TIMEOUT: Duration = Duration::from_secs(120);

fn network() -> RunOptions {
    RunOptions {
        timeout: NETWORK_TIMEOUT,
        ..RunOptions::write()
    }
}

/// 拉取远端引用（不改工作区）。
pub(crate) async fn fetch(root: &Path) -> Result<(), GitError> {
    run(root, &["fetch", "--all", "--prune"], network()).await?;
    Ok(())
}

/// 推送分支；`tags` 为真时附带 `--follow-tags`。
pub(crate) async fn push(root: &Path, branch: Option<&str>, tags: bool) -> Result<(), GitError> {
    let mut args: Vec<&str> = vec!["push"];
    if tags {
        args.push("--follow-tags");
    }
    match branch {
        Some(branch) => {
            args.push("origin");
            args.push(branch);
        }
        None => {
            // 无参数时按 push.default 配置推送当前分支，与命令行行为一致。
        }
    }
    run(root, &args, network()).await?;
    Ok(())
}

/// 把「到此提交为止」的历史推到远端同名分支：`git push <remote> <oid>:refs/heads/<branch>`。
///
/// 设计原理（Why）：
/// - 用显式 refspec 而不是先 `reset` 再 `push`：不改本地分支与索引，用户手上有
///   未提交改动时也不会被牵连；远端如果不是该提交的下游，git 自己按
///   non-fast-forward 拒绝，本层不做二次判断也不开 `--force`。
/// - 远端取该分支的上游（`<branch>@{upstream}` 的前半段），没有上游时退回
///   `origin`——这与命令行的 `git push origin` 默认一致，也比"让用户选远端"
///   少一次弹窗。
pub(crate) async fn push_up_to(root: &Path, oid: &str, branch: &str) -> Result<(), GitError> {
    if oid.is_empty() || !oid.chars().all(|c| c.is_ascii_hexdigit()) || branch.trim().is_empty() {
        return Err(GitError::InvalidPath {
            path: format!("{oid}:{branch}"),
            reason: "推送目标必须是提交号与分支名".into(),
        });
    }
    let remote = upstream_remote(root, branch).await;
    let refspec = format!("{oid}:refs/heads/{branch}");
    run(root, &["push", &remote, &refspec], network()).await?;
    Ok(())
}

/// 分支的上游远端名；未设置上游时退回 `origin`。
async fn upstream_remote(root: &Path, branch: &str) -> String {
    let query = format!("{branch}@{{upstream}}");
    let output = match run_allow_fail(
        root,
        &["rev-parse", "--abbrev-ref", "--symbolic-full-name", &query],
        RunOptions::default(),
    )
    .await
    {
        Ok(output) if output.ok() => output,
        // 取不到上游不是错误：退回 origin，与命令行默认一致。
        _ => {
            log::debug!("分支未设置上游，退回默认远端，分支: {branch}");
            return "origin".to_string();
        }
    };
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    match text.split_once('/') {
        Some((remote, _)) if !remote.is_empty() => remote.to_string(),
        _ => {
            log::debug!("上游名称形态不含远端，退回默认远端，上游: {text}");
            "origin".to_string()
        }
    }
}

/// 拉取并整合上游。
pub(crate) async fn pull(root: &Path, rebase: bool) -> Result<(), GitError> {
    let args: Vec<&str> = if rebase {
        vec!["pull", "--rebase"]
    } else {
        vec!["pull"]
    };
    run(root, &args, network()).await?;
    Ok(())
}

/// 远端名列表。
pub(crate) async fn remotes(root: &Path) -> Result<Vec<String>, GitError> {
    let output = run(root, &["remote"], RunOptions::default()).await?;
    Ok(output
        .lines()
        .into_iter()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect())
}

/// 新建工作树：`git worktree add -b <branch> <path> [start]`。
pub(crate) async fn add_worktree(
    root: &Path,
    path: &str,
    branch: &str,
    start: Option<&str>,
) -> Result<(), GitError> {
    let mut args: Vec<&str> = vec!["worktree", "add", "-b", branch, path];
    if let Some(start) = start {
        args.push(start);
    }
    run(root, &args, RunOptions::write()).await?;
    Ok(())
}
