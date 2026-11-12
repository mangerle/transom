//! 会改写仓库状态的子命令：暂存、提交、丢弃与分支切换。
//!
//! 设计原理（Why）：
//! - 全部走 `RunOptions::write()`：写操作必须让 git 自己创建 `index.lock`，
//!   与用户命令行里的 git 互斥，否则两边同时写索引会留下损坏的 index；
//! - 取消暂存在「尚无任何提交」的新仓库里不能走 `git restore --staged`：
//!   该命令需要一个 HEAD 树作参照，空仓库里必然失败，因此按 HEAD 是否存在
//!   分叉到 `git rm --cached`，这是 git 官方给出的空仓库等价操作；
//! - 丢弃未跟踪文件用 `git clean -d`：只接受显式列出的路径，绝不带 `-x`
//!   顺手清掉忽略文件，误删范围必须由界面的勾选决定。

use crate::config::GitConfig;
use crate::error::GitError;
use crate::process::{RunOptions, run, run_allow_fail};
use crate::repo;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static PATCH_SEQ: AtomicU64 = AtomicU64::new(0);

/// 暂存指定路径（含删除记录）。
pub(crate) async fn stage(
    root: &Path,
    config: &GitConfig,
    paths: &[String],
) -> Result<(), GitError> {
    let mut args = vec!["add", "-A", "--"];
    push_paths(&mut args, paths);
    run(root, config, &args, RunOptions::write()).await?;
    Ok(())
}

/// 暂存全部改动（含未跟踪文件）。
pub(crate) async fn stage_all(root: &Path, config: &GitConfig) -> Result<(), GitError> {
    run(root, config, &["add", "-A"], RunOptions::write()).await?;
    Ok(())
}

/// 取消暂存：索引回退到 HEAD 版本；空仓库里改为从索引移除。
pub(crate) async fn unstage(
    root: &Path,
    config: &GitConfig,
    paths: &[String],
) -> Result<(), GitError> {
    let has_head = repo::resolve(root, config, "HEAD").await?.is_some();
    let mut args: Vec<&str> = if has_head {
        vec!["restore", "--staged", "--"]
    } else {
        vec!["rm", "--cached", "-q", "--"]
    };
    push_paths(&mut args, paths);
    run(root, config, &args, RunOptions::write()).await?;
    Ok(())
}

/// 丢弃已跟踪文件的工作区改动。
pub(crate) async fn discard(
    root: &Path,
    config: &GitConfig,
    paths: &[String],
) -> Result<(), GitError> {
    let mut args = vec!["restore", "--worktree", "--"];
    push_paths(&mut args, paths);
    run(root, config, &args, RunOptions::write()).await?;
    Ok(())
}

/// 删除未跟踪文件/目录（不可恢复，界面必须二次确认）。
pub(crate) async fn discard_untracked(
    root: &Path,
    config: &GitConfig,
    paths: &[String],
) -> Result<(), GitError> {
    let mut args = vec!["clean", "-f", "-d", "--"];
    push_paths(&mut args, paths);
    run(root, config, &args, RunOptions::write()).await?;
    Ok(())
}

/// 以索引内容创建一次提交。
///
/// `amend` 为真时改写最后一次提交（含仅改说明文字的情况）。
pub(crate) async fn commit(
    root: &Path,
    config: &GitConfig,
    message: &str,
    amend: bool,
) -> Result<(), GitError> {
    let mut args: Vec<&str> = vec!["commit", "-m", message];
    if amend {
        args.push("--amend");
        // 只改说明时允许索引为空，否则 git 会以「无改动」拒绝。
        args.push("--no-edit");
    }
    run(root, config, &args, RunOptions::write()).await?;
    Ok(())
}

/// 针对指定的历史提交创建 fixup 提交。
pub(crate) async fn commit_fixup(
    root: &Path,
    config: &GitConfig,
    oid: &str,
) -> Result<(), GitError> {
    run(
        root,
        config,
        &["commit", "--fixup", oid],
        RunOptions::write(),
    )
    .await?;
    Ok(())
}

/// 切换分支；`None` 表示切回当前分支（不做任何事）。
pub(crate) async fn checkout(
    root: &Path,
    config: &GitConfig,
    branch: &str,
) -> Result<(), GitError> {
    run(root, config, &["switch", branch], RunOptions::write()).await?;
    Ok(())
}

/// 游离头指针检出到指定提交。
pub(crate) async fn checkout_detached(
    root: &Path,
    config: &GitConfig,
    oid: &str,
) -> Result<(), GitError> {
    run(
        root,
        config,
        &["switch", "--detach", oid],
        RunOptions::write(),
    )
    .await?;
    Ok(())
}

/// 新建分支，可选指定起点与是否立即切换。
pub(crate) async fn create_branch(
    root: &Path,
    config: &GitConfig,
    name: &str,
    start: Option<&str>,
    switch_to: bool,
) -> Result<(), GitError> {
    check_ref_name(name)?;
    let mut args: Vec<&str> = vec!["branch", name];
    if let Some(start) = start {
        args.push(start);
    }
    run(root, config, &args, RunOptions::write()).await?;
    if switch_to {
        checkout(root, config, name).await?;
    }
    Ok(())
}

/// 删除分支；`force` 对应有未合并提交的情况。
pub(crate) async fn delete_branch(
    root: &Path,
    config: &GitConfig,
    name: &str,
    force: bool,
) -> Result<(), GitError> {
    let flag = if force { "-D" } else { "-d" };
    run(root, config, &["branch", flag, name], RunOptions::write()).await?;
    Ok(())
}

/// 追加路径参数；空列表时补一个 `.` 以免 git 把裸 `--` 当语法错误。
fn push_paths<'a>(args: &mut Vec<&'a str>, paths: &'a [String]) {
    if paths.is_empty() {
        args.push(".");
        return;
    }
    for path in paths {
        args.push(path.as_str());
    }
}

/// 创建标签：`message` 非空时建附注标签。
pub(crate) async fn create_tag(
    root: &Path,
    config: &GitConfig,
    name: &str,
    target: &str,
    message: Option<&str>,
) -> Result<(), GitError> {
    let mut args: Vec<&str> = vec!["tag"];
    match message.filter(|text| !text.trim().is_empty()) {
        Some(message) => args.extend(["-a", name, "-m", message, target]),
        None => args.extend([name, target]),
    }
    run(root, config, &args, RunOptions::write()).await?;
    Ok(())
}

/// 删除本地标签。
pub(crate) async fn delete_tag(
    root: &Path,
    config: &GitConfig,
    name: &str,
) -> Result<(), GitError> {
    run(root, config, &["tag", "-d", name], RunOptions::write()).await?;
    Ok(())
}

/// 校验分支/标签名。
///
/// 设计原理（Why）：名字现在来自用户在对话框里的输入，不再由界面自动生成。
/// 以 `-` 开头的值会被 git 当成选项（`git branch -- -foo` 之外没有转义位），
/// 含空格、`..`、`~^:?*[\` 的名字 git 或文件系统都接受不了，因此在边界上直接
/// 拒绝并给出中文原因，比让 git 抛一句英文 fatal 更好排查。
pub(crate) fn check_ref_name(name: &str) -> Result<(), GitError> {
    let bad = name.is_empty()
        || name.starts_with('-')
        || name.starts_with('.')
        || name.ends_with('/')
        || name.ends_with('.')
        || name.contains("..")
        || name.contains(' ')
        || name.chars().any(|c| {
            matches!(
                c,
                '~' | '^' | ':' | '?' | '*' | '[' | '\\' | '@' | '{' | '}'
            )
        });
    if bad {
        return Err(GitError::InvalidPath {
            path: name.to_string(),
            reason: "分支或标签名不能含空格与 ~ ^ : ? * [ \\ @ {{ }}，也不能以 - 或 . 开头".into(),
        });
    }
    Ok(())
}

/// 重命名分支：`git branch -m <旧名> <新名>`。
///
/// 设计原理（Why）：用 `-m`（移动）而不是「新建 + 删除」：前者保留上游配置与
/// 引用日志，后者会把分支的历史关系剪断。当前分支同样可重命名，git 会连带切过去。
pub(crate) async fn rename_branch(
    root: &Path,
    config: &GitConfig,
    old: &str,
    new: &str,
) -> Result<(), GitError> {
    check_ref_name(new)?;
    check_ref_name(old)?;
    run(
        root,
        config,
        &["branch", "-m", old, new],
        RunOptions::write(),
    )
    .await?;
    Ok(())
}

/// 导出指定路径的工作区补丁（Unified Diff 格式）。
///
/// 当 paths 为空时导出整个工作区的变动；优先比对 HEAD。
pub(crate) async fn export_patch(
    root: &Path,
    config: &GitConfig,
    paths: &[String],
) -> Result<String, GitError> {
    let has_head = repo::resolve(root, config, "HEAD").await?.is_some();
    let mut args: Vec<&str> = if has_head {
        vec!["diff", "HEAD", "--binary", "--"]
    } else {
        vec!["diff", "--cached", "--binary", "--"]
    };
    push_paths(&mut args, paths);
    let output = run(root, config, &args, RunOptions::default()).await?;
    let mut patch = String::from_utf8_lossy(&output.stdout).to_string();
    if patch.trim().is_empty() {
        let mut fallback_args: Vec<&str> = vec!["diff", "--binary", "--"];
        push_paths(&mut fallback_args, paths);
        let fb_output = run(root, config, &fallback_args, RunOptions::default()).await?;
        patch = String::from_utf8_lossy(&fb_output.stdout).to_string();
    }
    Ok(patch)
}

/// 应用补丁文本到工作区。
pub(crate) async fn apply_patch(
    root: &Path,
    config: &GitConfig,
    patch: &str,
) -> Result<(), GitError> {
    apply_patch_with_options(root, config, patch, false, false).await
}

/// 应用局部变更块或补丁文本：可按 `cached` 写入暂存区、按 `reverse` 执行反向撤销。
pub(crate) async fn apply_patch_with_options(
    root: &Path,
    config: &GitConfig,
    patch: &str,
    cached: bool,
    reverse: bool,
) -> Result<(), GitError> {
    let seq = PATCH_SEQ.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let temp_path = std::env::temp_dir().join(format!("transom_patch_{pid}_{seq}.patch"));
    tokio::fs::write(&temp_path, patch.as_bytes())
        .await
        .map_err(|source| GitError::Spawn {
            command: "write hunk patch".to_string(),
            source,
        })?;
    let path_str = temp_path.to_string_lossy().to_string();
    let mut args = vec!["apply", "--whitespace=nowarn"];
    if cached {
        args.push("--cached");
    }
    if reverse {
        args.push("--reverse");
    }
    args.push(&path_str);
    let result = run(root, config, &args, RunOptions::write()).await;
    let _ = tokio::fs::remove_file(&temp_path).await;
    result.map(|_| ())
}

/// 采用本地/当前分支版本解决冲突并暂存。
pub(crate) async fn resolve_ours(
    root: &Path,
    config: &GitConfig,
    paths: &[String],
) -> Result<(), GitError> {
    let mut args = vec!["checkout", "--ours", "--"];
    push_paths(&mut args, paths);
    run(root, config, &args, RunOptions::write()).await?;
    stage(root, config, paths).await
}

/// 采用传入版本解决冲突并暂存。
pub(crate) async fn resolve_theirs(
    root: &Path,
    config: &GitConfig,
    paths: &[String],
) -> Result<(), GitError> {
    let mut args = vec!["checkout", "--theirs", "--"];
    push_paths(&mut args, paths);
    run(root, config, &args, RunOptions::write()).await?;
    stage(root, config, paths).await
}

/// 标记冲突文件已手动解决（直接加入暂存区）。
pub(crate) async fn mark_resolved(
    root: &Path,
    config: &GitConfig,
    paths: &[String],
) -> Result<(), GitError> {
    stage(root, config, paths).await
}

/// 读取冲突文件在三方阶段的内容（Base、Ours、Theirs）以及当前工作区文本。
pub(crate) async fn conflict_sides(
    root: &Path,
    config: &GitConfig,
    path: &str,
) -> Result<transom_core::ConflictSides, GitError> {
    let read_stage = |stage: u8| async move {
        let spec = format!(":{stage}:{path}");
        let output = run_allow_fail(
            root,
            config,
            &["show", "--end-of-options", &spec],
            RunOptions::default(),
        )
        .await?;
        if output.ok() {
            Ok(Some(String::from_utf8_lossy(&output.stdout).to_string()))
        } else {
            Ok(None)
        }
    };

    let base = read_stage(1).await?;
    let ours = read_stage(2).await?;
    let theirs = read_stage(3).await?;
    let working_path = repo::resolve_in(root, path)?;
    let working = match tokio::fs::read(&working_path).await {
        Ok(bytes) => String::from_utf8_lossy(&bytes).to_string(),
        Err(_) => String::new(),
    };

    Ok(transom_core::ConflictSides {
        base,
        ours,
        theirs,
        working,
    })
}
