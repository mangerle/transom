//! 提交补丁导出：把一条提交打成 `git am` / 邮件可直接吃的 mbox 补丁。
//!
//! 设计原理（Why）：
//! - 用 `format-patch --stdout -1` 而不是 `diff` 拼接：前者自带作者、日期与
//!   `Subject: [PATCH]` 头，收件人和 `git am` 都能直接用，也与 IDEA 的
//!   「创建补丁」产物一致；根提交同样能出（`-1` 不需要父提交）。
//! - 体积上限留在本层判定：补丁是把整条提交的正文内联进文本，遇到一次
//!   vendor 目录提交可能上百 MB，界面拿到就 OOM；超限如实报错，不截断
//!   ——截断的补丁是坏的补丁，比没有补丁更危险。
//! - 只读命令（不改索引也不改工作区），因此可以并发执行、可被取消；
//! - 中文标题会被 git 编成 `=?UTF-8?q?...=` 的 RFC2047 形式（本机 git 2.53 实测），
//!   这是补丁格式本身的规定，`git am` 收得回去，界面按原样落盘不做二次处理。

use crate::error::GitError;
use crate::process::{RunOptions, run};
use std::path::Path;

/// 补丁上限：与差异单侧上限同量级，再大就该走仓库打包而不是补丁。
pub const MAX_PATCH_BYTES: u64 = 16 * 1024 * 1024;

/// 生成一条提交的补丁文本。
///
/// # Errors
/// 提交号不存在、子进程失败或超时返回 [`GitError`]；补丁超过
/// [`MAX_PATCH_BYTES`] 返回 [`GitError::TooLarge`]。
pub(crate) async fn commit_patch(root: &Path, oid: &str) -> Result<Vec<u8>, GitError> {
    let output = run(
        root,
        &["format-patch", "--stdout", "-1", "--end-of-options", oid],
        RunOptions::default(),
    )
    .await?;
    let bytes = output.stdout.len() as u64;
    if bytes > MAX_PATCH_BYTES {
        return Err(GitError::TooLarge {
            path: format!("补丁 {oid}"),
            limit: MAX_PATCH_BYTES,
        });
    }
    Ok(output.stdout)
}
