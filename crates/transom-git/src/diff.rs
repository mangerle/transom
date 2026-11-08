//! 单文件两侧内容读取，供界面做分栏差异。
//!
//! 设计原理（Why）：
//! - 取「两侧文本」而不是 git 的 unified 补丁再喂给差异控件：gpui-kit 的
//!   `Diff` 组件与既有远程差异查看器都以两份全文为输入，复用同一套 hunk
//!   计算与逐块跳转，界面口径保持一致；
//! - 体积上限与二进制判定放在这一层：差异是只读旁路能力，绝不能因为一次
//!   `git show` 回传 200MB 而把内存打满，超限即截断并如实标注；
//! - 索引侧与提交侧统一走 `git show <rev>:<path>`：由 git 负责解引用与
//!   子模块边界，本层不碰对象库文件。

use crate::config::GitConfig;
use crate::error::GitError;
use crate::process::{RunOptions, run_allow_fail};
use crate::types::DiffContent;
use std::path::Path;

/// 单侧内容上限，与远程差异查看器保持同一口径。
pub const MAX_SIDE_BYTES: usize = 4 * 1024 * 1024;

/// 内容来源侧：契约归 core（界面与命令总线共用），这里只做别名再导出。
pub type Side = transom_core::GitSide;

/// 读取一侧内容；该侧不存在文件时返回 `None`。
async fn side_content(
    root: &Path,
    config: &GitConfig,
    side: &Side,
    path: &str,
) -> Result<Option<Vec<u8>>, GitError> {
    match side {
        Side::Worktree => {
            let absolute = crate::repo::resolve_in(root, path)?;
            match tokio::fs::read(&absolute).await {
                Ok(bytes) => Ok(Some(bytes)),
                Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(source) => Err(GitError::Spawn {
                    command: format!("read {path}"),
                    source,
                }),
            }
        }
        Side::Index => show(root, config, &format!(":{path}"), path).await,
        Side::Commit(oid) => show(root, config, &format!("{oid}:{path}"), path).await,
    }
}

/// `git show <rev>:<path>`；对象不存在按「该侧无此文件」处理。
async fn show(
    root: &Path,
    config: &GitConfig,
    spec: &str,
    path: &str,
) -> Result<Option<Vec<u8>>, GitError> {
    let output = run_allow_fail(
        root,
        config,
        &["show", "--end-of-options", spec],
        RunOptions::default(),
    )
    .await?;
    if !output.ok() {
        log::debug!("差异侧内容不存在，路径: {path}，定位: {spec}");
        return Ok(None);
    }
    Ok(Some(output.stdout))
}

/// 组装一个文件的两侧文本。
///
/// # Errors
/// 路径越出仓库根时返回 [`GitError::InvalidPath`]。
pub(crate) async fn content(
    root: &Path,
    config: &GitConfig,
    old: &Side,
    new: &Side,
    path: &str,
) -> Result<DiffContent, GitError> {
    let old_bytes = side_content(root, config, old, path).await?;
    let new_bytes = side_content(root, config, new, path).await?;
    let binary =
        old_bytes.as_deref().is_some_and(is_binary) || new_bytes.as_deref().is_some_and(is_binary);
    let (old_text, old_cut) = decode(old_bytes);
    let (new_text, new_cut) = decode(new_bytes);
    Ok(DiffContent {
        path: path.to_string(),
        old_text,
        new_text,
        binary,
        truncated: old_cut || new_cut,
    })
}

/// 二进制判定：前 8KB 出现 NUL 即按二进制处理，与 git 自己的启发式一致。
fn is_binary(bytes: &[u8]) -> bool {
    bytes.get(..8 * 1024).unwrap_or(bytes).contains(&0)
}

/// 解码并截断到上限，返回是否发生了截断。
fn decode(bytes: Option<Vec<u8>>) -> (String, bool) {
    let Some(bytes) = bytes else {
        return (String::new(), false);
    };
    let truncated = bytes.len() > MAX_SIDE_BYTES;
    let slice = if truncated {
        &bytes[..MAX_SIDE_BYTES]
    } else {
        &bytes[..]
    };
    (String::from_utf8_lossy(slice).into_owned(), truncated)
}
