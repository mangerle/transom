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

use crate::config::GitConfig;
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
pub(crate) async fn fetch(root: &Path, config: &GitConfig) -> Result<(), GitError> {
    run(root, config, &["fetch", "--all", "--prune"], network()).await?;
    Ok(())
}

use transom_core::{CommitEntry, GitPushOptions, PushTagsMode};

/// 高级推送：支持自定义目标远端、目标分支、强推租约、标签模式与跳过钩子。
pub(crate) async fn push_advanced(
    root: &Path,
    config: &GitConfig,
    opts: &GitPushOptions,
) -> Result<(), GitError> {
    let mut args: Vec<String> = vec!["push".to_string()];
    if opts.force_with_lease {
        args.push("--force-with-lease".to_string());
    }
    if opts.no_verify {
        args.push("--no-verify".to_string());
    }
    match opts.tags {
        PushTagsMode::FollowTags => args.push("--follow-tags".to_string()),
        PushTagsMode::AllTags => args.push("--tags".to_string()),
        PushTagsMode::None => {}
    }
    let remote = match &opts.remote {
        Some(r) if !r.trim().is_empty() => r.trim().to_string(),
        _ => match &opts.branch {
            Some(b) => upstream_remote(root, config, b).await,
            None => "origin".to_string(),
        },
    };
    args.push(remote);

    match (&opts.branch, &opts.remote_branch) {
        (Some(local), Some(remote)) if !local.is_empty() && !remote.is_empty() => {
            if local == remote {
                args.push(local.clone());
            } else {
                args.push(format!("{local}:{remote}"));
            }
        }
        (Some(local), None) if !local.is_empty() => {
            args.push(local.clone());
        }
        _ => {}
    }

    let str_args: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    run(root, config, &str_args, network()).await?;
    Ok(())
}

/// 兼容传统调用的简单推送。
pub(crate) async fn push(
    root: &Path,
    config: &GitConfig,
    branch: Option<&str>,
    tags: bool,
) -> Result<(), GitError> {
    push_advanced(
        root,
        config,
        &GitPushOptions {
            branch: branch.map(str::to_string),
            tags: if tags {
                PushTagsMode::FollowTags
            } else {
                PushTagsMode::None
            },
            ..Default::default()
        },
    )
    .await
}

/// 查询待推送到目标远端分支的提交列表（Outgoing Commits）。
pub(crate) async fn outgoing_commits(
    root: &Path,
    config: &GitConfig,
    branch: &str,
    remote: Option<&str>,
    remote_branch: Option<&str>,
) -> Result<Vec<CommitEntry>, GitError> {
    if branch.trim().is_empty() {
        return Ok(Vec::new());
    }

    let target_remote = match remote {
        Some(r) if !r.trim().is_empty() => r.trim().to_string(),
        _ => upstream_remote(root, config, branch).await,
    };
    let target_branch = match remote_branch {
        Some(b) if !b.trim().is_empty() => b.trim().to_string(),
        _ => branch.to_string(),
    };

    let remote_ref = format!("refs/remotes/{target_remote}/{target_branch}");
    let upstream_ref = format!("{branch}@{{upstream}}");

    let has_remote_ref = match run_allow_fail(
        root,
        config,
        &["rev-parse", "--verify", "--quiet", &remote_ref],
        RunOptions::default(),
    )
    .await
    {
        Ok(out) => out.ok(),
        Err(_) => false,
    };

    let range = if has_remote_ref {
        format!("{remote_ref}..{branch}")
    } else {
        let has_upstream = match run_allow_fail(
            root,
            config,
            &["rev-parse", "--verify", "--quiet", &upstream_ref],
            RunOptions::default(),
        )
        .await
        {
            Ok(out) => out.ok(),
            Err(_) => false,
        };
        if has_upstream {
            format!("{upstream_ref}..{branch}")
        } else {
            let base_main = match run_allow_fail(
                root,
                config,
                &["rev-parse", "--verify", "--quiet", "refs/heads/main"],
                RunOptions::default(),
            )
            .await
            {
                Ok(out) if out.ok() && branch != "main" => Some("main"),
                _ => None,
            };
            if let Some(base) = base_main {
                format!("{base}..{branch}")
            } else {
                branch.to_string()
            }
        }
    };

    let args = [
        "log",
        &format!("--pretty=format:{}", crate::log::FORMAT),
        "--topo-order",
        "--no-color",
        "-n",
        "100",
        &range,
    ];

    let output = match run_allow_fail(root, config, &args, RunOptions::default()).await {
        Ok(out) if out.ok() => out,
        _ => {
            let fallback_args = [
                "log",
                &format!("--pretty=format:{}", crate::log::FORMAT),
                "--topo-order",
                "--no-color",
                "-n",
                "20",
                branch,
            ];
            run(root, config, &fallback_args, RunOptions::default()).await?
        }
    };

    let page = crate::log::parse_records(&output.stdout)?;
    Ok(page.entries)
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
pub(crate) async fn push_up_to(
    root: &Path,
    config: &GitConfig,
    oid: &str,
    branch: &str,
) -> Result<(), GitError> {
    if oid.is_empty() || !oid.chars().all(|c| c.is_ascii_hexdigit()) || branch.trim().is_empty() {
        return Err(GitError::InvalidPath {
            path: format!("{oid}:{branch}"),
            reason: "推送目标必须是提交号与分支名".into(),
        });
    }
    let remote = upstream_remote(root, config, branch).await;
    let refspec = format!("{oid}:refs/heads/{branch}");
    run(root, config, &["push", &remote, &refspec], network()).await?;
    Ok(())
}

/// 分支的上游远端名；未设置上游时退回 `origin`。
async fn upstream_remote(root: &Path, config: &GitConfig, branch: &str) -> String {
    let query = format!("{branch}@{{upstream}}");
    let output = match run_allow_fail(
        root,
        config,
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
pub(crate) async fn pull(root: &Path, config: &GitConfig, rebase: bool) -> Result<(), GitError> {
    let args: Vec<&str> = if rebase {
        vec!["pull", "--rebase"]
    } else {
        vec!["pull"]
    };
    run(root, config, &args, network()).await?;
    Ok(())
}

/// 远端名列表。
pub(crate) async fn remotes(root: &Path, config: &GitConfig) -> Result<Vec<String>, GitError> {
    let output = run(root, config, &["remote"], RunOptions::default()).await?;
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
    config: &GitConfig,
    path: &str,
    branch: &str,
    start: Option<&str>,
) -> Result<(), GitError> {
    let mut args: Vec<&str> = vec!["worktree", "add", "-b", branch, path];
    if let Some(start) = start {
        args.push(start);
    }
    run(root, config, &args, RunOptions::write()).await?;
    Ok(())
}
