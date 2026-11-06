//! 仓库定位与修订解析。
//!
//! 设计原理（Why）：
//! - 一切子命令都以「工作区根」为 `current_dir`，因此 [`Repo::open`] 必须先把
//!   用户给的目录归一化到 toplevel：否则同一仓库在子目录与根目录下会算出
//!   两份不同的相对路径，暂存与差异都会指错文件；
//! - 是否仓库用 `rev-parse --is-inside-work-tree` 的退出码判定，不解析
//!   stderr 文案：本机 git 在未设 `LC_ALL` 时回传中文 fatal，靠字符串匹配
//!   会随 locale 失效；
//! - 路径在拼接前一律做穿越校验：仓库根之外的绝对路径与含 `..` 的相对路径
//!   直接拒绝，杜绝把宿主机任意文件读进差异视图。

use crate::error::GitError;
use crate::process::{RunOptions, run, run_allow_fail};
use crate::types::RepoInfo;
use std::path::{Component, Path, PathBuf};

/// 一个已定位到工作区根的仓库句柄。
///
/// 句柄本身只是路径，可自由克隆并在任务间传递；真正的状态都在 `.git` 里。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    root: PathBuf,
}

impl Repo {
    /// 校验并打开一个仓库。
    ///
    /// # Errors
    /// 路径非法返回 [`GitError::InvalidPath`]，目录不在任何仓库内返回
    /// [`GitError::NotRepo`]，git 不可用返回 [`GitError::Spawn`]。
    pub async fn open(path: &Path) -> Result<Self, GitError> {
        let info = probe(path).await?;
        Ok(Self {
            root: info.toplevel,
        })
    }

    /// 以「路径已经是规范化工作区根」为前提直接构造句柄。
    ///
    /// 设计原理（Why）：[`Repo::open`] 要跑三趟 `rev-parse` 子进程，而项目路径
    /// 在添加时就已探测并规范化过（存进 `GitProject`），每条命令再探测一次纯属
    /// 浪费。因此这里只做零成本的构造，前提是调用方保证路径可信。
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// 工作区根目录（规范化绝对路径）。
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 把仓库相对路径解析为宿主机绝对路径，并拒绝越出仓库根的输入。
    ///
    /// # Errors
    /// 见 [`resolve_in`]。
    pub fn resolve_path(&self, relative: &str) -> Result<PathBuf, GitError> {
        resolve_in(&self.root, relative)
    }
}

/// 把仓库相对路径解析为宿主机绝对路径，并拒绝越出仓库根的输入。
///
/// # Errors
/// 绝对路径、含 `..` 或规范化后不在仓库根之下时返回 [`GitError::InvalidPath`]。
pub(crate) fn resolve_in(root: &Path, relative: &str) -> Result<PathBuf, GitError> {
    let rejected = |reason: &str| GitError::InvalidPath {
        path: relative.to_string(),
        reason: reason.to_string(),
    };
    let candidate = Path::new(relative);
    if candidate.is_absolute() {
        return Err(rejected("必须是仓库相对路径"));
    }
    if candidate
        .components()
        .any(|part| matches!(part, Component::ParentDir | Component::RootDir))
    {
        return Err(rejected("路径不得包含上级目录片段"));
    }
    // 仓库根本身在打开时已规范化；子路径可能因符号链接指向仓库外，
    // 因此再比一次前缀，确保读写的确实是工作区内的文件。
    let joined = root.join(candidate);
    let canonical = std::fs::canonicalize(&joined).unwrap_or_else(|_| joined.clone());
    if !canonical.starts_with(root) {
        return Err(rejected("解析后的路径越出了仓库根"));
    }
    Ok(joined)
}

/// 初始化一个新仓库并返回句柄。
///
/// # Errors
/// 目录不存在或 git 拒绝初始化时返回对应错误。
pub async fn init(path: &Path) -> Result<Repo, GitError> {
    if !path.is_dir() {
        return Err(GitError::InvalidPath {
            path: path.display().to_string(),
            reason: "目录不存在".to_string(),
        });
    }
    run(path, &["init", "-q"], RunOptions::write()).await?;
    Repo::open(path).await
}

/// 探测目录所属仓库的工作区根与 git 目录。
///
/// # Errors
/// 目录不存在返回 [`GitError::InvalidPath`]，不在仓库内返回 [`GitError::NotRepo`]。
pub async fn probe(path: &Path) -> Result<RepoInfo, GitError> {
    let canonical = std::fs::canonicalize(path).map_err(|source| GitError::InvalidPath {
        path: path.display().to_string(),
        reason: source.to_string(),
    })?;
    if !canonical.is_dir() {
        return Err(GitError::InvalidPath {
            path: canonical.display().to_string(),
            reason: "不是目录".to_string(),
        });
    }
    let inside = run_allow_fail(
        &canonical,
        &["rev-parse", "--is-inside-work-tree"],
        RunOptions::default(),
    )
    .await?;
    if !inside.ok() || String::from_utf8_lossy(&inside.stdout).trim() != "true" {
        return Err(GitError::NotRepo {
            path: canonical.display().to_string(),
        });
    }
    let toplevel = rev_parse(&canonical, "--show-toplevel").await?;
    let git_dir = rev_parse(&canonical, "--absolute-git-dir").await?;
    // 链接工作树的判据是「本工作树的 git 目录不等于公共 git 目录」：
    // `git worktree add` 出来的目录有独立的 HEAD/index，但 refs 与对象库共用主仓库。
    let common_dir = rev_parse(&canonical, "--git-common-dir").await?;
    let common_dir = if common_dir.is_absolute() {
        common_dir
    } else {
        canonical.join(common_dir)
    };
    let linked_worktree = normalize(&git_dir) != normalize(&common_dir);
    Ok(RepoInfo {
        toplevel,
        git_dir,
        linked_worktree,
    })
}

/// 去掉路径尾部的 `.git` 差异成分，使不同写法可比。
fn normalize(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// 解析一个修订为对象号；解析不到返回 `None`（不视为错误）。
pub(crate) async fn resolve(root: &Path, rev: &str) -> Result<Option<String>, GitError> {
    let output = run_allow_fail(
        root,
        &["rev-parse", "--verify", "--quiet", rev],
        RunOptions::default(),
    )
    .await?;
    if !output.ok() {
        return Ok(None);
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string))
}

/// 取 `rev-parse` 的单行输出（目录类信息）。
async fn rev_parse(root: &Path, flag: &str) -> Result<PathBuf, GitError> {
    let output = run(root, &["rev-parse", flag], RunOptions::default()).await?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        return Err(GitError::Decode {
            command: format!("rev-parse {flag}"),
            reason: "输出为空".to_string(),
        });
    }
    Ok(PathBuf::from(text))
}
