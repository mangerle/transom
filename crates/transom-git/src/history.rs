//! 历史改写与整合类动作：重置、撤销提交、拣选、合并、变基。
//!
//! 设计原理（Why）：
//! - 全部以子命令原样下发，不做"更安全"的包装：IDEA 的「将当前分支重置到此处」
//!   也是直接把 `git reset --hard` 摆出来让用户选力度。界面负责二次确认，
//!   本层擅自加保护只会让命令行里能做的事在这里做不了；
//! - 合并与变基不自动开编辑器：`GIT_EDITOR=true` 已在 `process.rs` 里设过，
//!   合并提交信息一律用 git 生成的默认值，需要改文案走「编辑提交消息」；
//! - 冲突不是错误终态而是待办状态：`merge`/`rebase` 返回非零时把 stderr 原样
//!   带回，界面按 `GitError::Failed` 提示，用户解完冲突再点继续。

use crate::config::GitConfig;
use crate::error::GitError;
use crate::process::{RunOptions, run, run_allow_fail};
use std::path::Path;

/// 重置力度的契约在 core（命令总线要用），这里只做别名再导出。
pub type ResetKind = transom_core::GitResetKind;

/// 档位对应的 git 旗标。
fn flag(kind: ResetKind) -> &'static str {
    match kind {
        ResetKind::Soft => "--soft",
        ResetKind::Mixed => "--mixed",
        ResetKind::Hard => "--hard",
    }
}

/// 将当前分支重置到指定修订。
pub(crate) async fn reset(
    root: &Path,
    config: &GitConfig,
    target: &str,
    kind: ResetKind,
) -> Result<(), GitError> {
    run(
        root,
        config,
        &["reset", flag(kind), target],
        RunOptions::write(),
    )
    .await?;
    Ok(())
}

/// 生成一个反向提交以撤销指定提交。
pub(crate) async fn revert(root: &Path, config: &GitConfig, oid: &str) -> Result<(), GitError> {
    run(
        root,
        config,
        &["revert", "--no-edit", oid],
        RunOptions::write(),
    )
    .await?;
    Ok(())
}

/// 把指定提交拣选到当前分支。
pub(crate) async fn cherry_pick(
    root: &Path,
    config: &GitConfig,
    oid: &str,
) -> Result<(), GitError> {
    run(root, config, &["cherry-pick", oid], RunOptions::write()).await?;
    Ok(())
}

/// 把指定引用合并进当前分支。
pub(crate) async fn merge(
    root: &Path,
    config: &GitConfig,
    reference: &str,
) -> Result<(), GitError> {
    run(
        root,
        config,
        &["merge", "--no-edit", reference],
        RunOptions::write(),
    )
    .await?;
    Ok(())
}

/// 将当前分支变基到指定上游。
pub(crate) async fn rebase_onto(
    root: &Path,
    config: &GitConfig,
    upstream: &str,
) -> Result<(), GitError> {
    run(root, config, &["rebase", upstream], RunOptions::write()).await?;
    Ok(())
}

/// 中止进行中的变基。
pub(crate) async fn rebase_abort(root: &Path, config: &GitConfig) -> Result<(), GitError> {
    run(root, config, &["rebase", "--abort"], RunOptions::write()).await?;
    Ok(())
}

/// 折叠方式：压缩保留两条说明，优选丢弃尾部提交的说明。
pub type FoldMode = transom_core::GitFoldMode;

/// 把分支头提交压进它的父提交（IDEA 的「压缩到／优选到」在单条尾提交上的形态）。
///
/// 设计原理（Why）：
/// - 用 `reset --soft` + `commit --amend` 而不是脚本化 `rebase -i` 的待办清单：
///   这两步是 git 自己的原语，产物与手工交互式压缩逐字节一致，却不引入
///   「往 GIT_SEQUENCE_EDITOR 里塞一段脚本」这种既难验证又难排错的做法；
/// - 只做「头提交压进父提交」这一种形态：更一般的多提交重排需要交互式变基
///   编辑器，那是另一条能力，不在这里偷偷近似实现成一个会改错历史的东西；
/// - 前置校验三件事（挂在分支上、头提交恰有一个父、暂存区干净），任一不满足
///   就直接拒绝：游离头压完会丢掉原头指针的引用，合并提交压平会丢掉第二父的
///   合并结构，暂存区不干净则用户的待提交内容会被一起吞进父提交。
pub(crate) async fn fold_tip_into_parent(
    root: &Path,
    config: &GitConfig,
    mode: FoldMode,
) -> Result<(), GitError> {
    let (tip, parent) = preconditions(root, config).await?;
    let message = match mode {
        FoldMode::Fixup => commit_message(root, config, &parent).await?,
        FoldMode::Squash => {
            let parent_message = commit_message(root, config, &parent).await?;
            let tip_message = commit_message(root, config, &tip).await?;
            format!("{parent_message}\n\n{tip_message}")
        }
    };
    run(
        root,
        config,
        &["reset", "--soft", &parent],
        RunOptions::write(),
    )
    .await?;
    if let Err(error) = run(
        root,
        config,
        &["commit", "--amend", "--no-edit", "-m", &message],
        RunOptions::write(),
    )
    .await
    {
        // 提交被钩子拒绝时把头指针移回去：`reset --soft` 已经把分支退到父提交，
        // 不回滚就等于留下一条「少了一个提交且改动全在暂存区」的状态给人误读。
        if let Err(rollback) = run(
            root,
            config,
            &["reset", "--soft", &tip],
            RunOptions::write(),
        )
        .await
        {
            log::error!(
                "回滚头指针失败，原因: {rollback}，需人工确认分支位置，项目: {}",
                root.display()
            );
        } else {
            log::warn!("压缩提交失败，已把头指针回滚到原提交，提交: {tip}");
        }
        return Err(error);
    }
    log::info!("头提交已压入父提交，方式: {mode:?}，原提交: {tip}，父提交: {parent}");
    Ok(())
}

/// 校验前置条件并返回 `(头提交, 父提交)`。
async fn preconditions(root: &Path, config: &GitConfig) -> Result<(String, String), GitError> {
    let branch = run_allow_fail(
        root,
        config,
        &["symbolic-ref", "--short", "-q", "HEAD"],
        RunOptions::default(),
    )
    .await?;
    if !branch.ok() {
        return Err(GitError::Precondition {
            reason: "头指针未挂在分支上，压缩会丢掉原头提交引用".into(),
        });
    }
    let parents = run(
        root,
        config,
        &["rev-list", "--parents", "-n", "1", "HEAD"],
        RunOptions::default(),
    )
    .await?;
    // `rev-list --parents -n 1` 是「一行内空格分隔」：头提交在前，父提交依次跟后。
    let fields: Vec<String> = parents
        .lines()
        .pop()
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_string)
        .collect();
    if fields.len() < 2 {
        return Err(GitError::Precondition {
            reason: "头提交没有父提交（仓库首个提交）".into(),
        });
    }
    if fields.len() > 2 {
        return Err(GitError::Precondition {
            reason: "头提交是合并提交，压平会丢掉第二父的合并结构".into(),
        });
    }
    let staged = run_allow_fail(
        root,
        config,
        &["diff", "--cached", "--quiet"],
        RunOptions::default(),
    )
    .await?;
    if !staged.ok() {
        return Err(GitError::Precondition {
            reason: "暂存区有待提交内容，先取消暂存或另行提交".into(),
        });
    }
    Ok((fields[0].clone(), fields[1].clone()))
}

/// 取一条提交的完整说明（标题 + 正文）。
async fn commit_message(root: &Path, config: &GitConfig, oid: &str) -> Result<String, GitError> {
    let output = run(
        root,
        config,
        &["log", "-1", "--format=%B", "--end-of-options", oid],
        RunOptions::default(),
    )
    .await?;
    Ok(String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .to_string())
}
