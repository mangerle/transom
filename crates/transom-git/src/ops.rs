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

use crate::error::GitError;
use crate::process::{RunOptions, run};
use crate::repo;
use std::path::Path;

/// 暂存指定路径（含删除记录）。
pub(crate) async fn stage(root: &Path, paths: &[String]) -> Result<(), GitError> {
    let mut args = vec!["add", "-A", "--"];
    push_paths(&mut args, paths);
    run(root, &args, RunOptions::write()).await?;
    Ok(())
}

/// 暂存全部改动（含未跟踪文件）。
pub(crate) async fn stage_all(root: &Path) -> Result<(), GitError> {
    run(root, &["add", "-A"], RunOptions::write()).await?;
    Ok(())
}

/// 取消暂存：索引回退到 HEAD 版本；空仓库里改为从索引移除。
pub(crate) async fn unstage(root: &Path, paths: &[String]) -> Result<(), GitError> {
    let has_head = repo::resolve(root, "HEAD").await?.is_some();
    let mut args: Vec<&str> = if has_head {
        vec!["restore", "--staged", "--"]
    } else {
        vec!["rm", "--cached", "-q", "--"]
    };
    push_paths(&mut args, paths);
    run(root, &args, RunOptions::write()).await?;
    Ok(())
}

/// 丢弃已跟踪文件的工作区改动。
pub(crate) async fn discard(root: &Path, paths: &[String]) -> Result<(), GitError> {
    let mut args = vec!["restore", "--worktree", "--"];
    push_paths(&mut args, paths);
    run(root, &args, RunOptions::write()).await?;
    Ok(())
}

/// 删除未跟踪文件/目录（不可恢复，界面必须二次确认）。
pub(crate) async fn discard_untracked(root: &Path, paths: &[String]) -> Result<(), GitError> {
    let mut args = vec!["clean", "-f", "-d", "--"];
    push_paths(&mut args, paths);
    run(root, &args, RunOptions::write()).await?;
    Ok(())
}

/// 以索引内容创建一次提交。
///
/// `amend` 为真时改写最后一次提交（含仅改说明文字的情况）。
pub(crate) async fn commit(root: &Path, message: &str, amend: bool) -> Result<(), GitError> {
    let mut args: Vec<&str> = vec!["commit", "-m", message];
    if amend {
        args.push("--amend");
        // 只改说明时允许索引为空，否则 git 会以「无改动」拒绝。
        args.push("--no-edit");
    }
    run(root, &args, RunOptions::write()).await?;
    Ok(())
}

/// 切换分支；`None` 表示切回当前分支（不做任何事）。
pub(crate) async fn checkout(root: &Path, branch: &str) -> Result<(), GitError> {
    run(root, &["switch", branch], RunOptions::write()).await?;
    Ok(())
}

/// 游离头指针检出到指定提交。
pub(crate) async fn checkout_detached(root: &Path, oid: &str) -> Result<(), GitError> {
    run(root, &["switch", "--detach", oid], RunOptions::write()).await?;
    Ok(())
}

/// 新建分支，可选指定起点与是否立即切换。
pub(crate) async fn create_branch(
    root: &Path,
    name: &str,
    start: Option<&str>,
    switch_to: bool,
) -> Result<(), GitError> {
    check_ref_name(name)?;
    let mut args: Vec<&str> = vec!["branch", name];
    if let Some(start) = start {
        args.push(start);
    }
    run(root, &args, RunOptions::write()).await?;
    if switch_to {
        checkout(root, name).await?;
    }
    Ok(())
}

/// 删除分支；`force` 对应有未合并提交的情况。
pub(crate) async fn delete_branch(root: &Path, name: &str, force: bool) -> Result<(), GitError> {
    let flag = if force { "-D" } else { "-d" };
    run(root, &["branch", flag, name], RunOptions::write()).await?;
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
    name: &str,
    target: &str,
    message: Option<&str>,
) -> Result<(), GitError> {
    let mut args: Vec<&str> = vec!["tag"];
    match message.filter(|text| !text.trim().is_empty()) {
        Some(message) => args.extend(["-a", name, "-m", message, target]),
        None => args.extend([name, target]),
    }
    run(root, &args, RunOptions::write()).await?;
    Ok(())
}

/// 删除本地标签。
pub(crate) async fn delete_tag(root: &Path, name: &str) -> Result<(), GitError> {
    run(root, &["tag", "-d", name], RunOptions::write()).await?;
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
pub(crate) async fn rename_branch(root: &Path, old: &str, new: &str) -> Result<(), GitError> {
    check_ref_name(new)?;
    check_ref_name(old)?;
    run(root, &["branch", "-m", old, new], RunOptions::write()).await?;
    Ok(())
}
