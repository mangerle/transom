//! 远端地址 → 网页提交链接的折算，以及对应的只读取址。
//!
//! 设计原理（Why）：
//! - 「在浏览器中查看」要的地址不在 git 的对象库里，而在托管服务的网页上：git 只存
//!   `remote.url`，形态五花八门（scp 式 `git@host:path`、`ssh://`、`https://`、
//!   带端口、带凭据）。这里做成纯函数折算，好处是能离线单测，不必为每种形态起子进程；
//! - 输出恒以 `https://` 开头且只含白名单字符：远端地址是克隆来的外部数据，
//!   直接丢给 `xdg-open` 等于让仓库作者决定本机调用哪个协议处理器。折成固定 https
//!   形态后，最坏结果是网页 404，不会变成任意打开；
//! - 托管类型只对 GitLab 系（host 或路径含 `gitlab`）走 `/-/commit/`，其余按 GitHub
//!   家族的 `/commit/`（GitHub 企业版、Gitea、Forgejo、Gitee、Azure DevOps 都是这一
//!   形态）。猜错的代价是一页 404，比把菜单项永远灰掉有用；
//! - 取不到地址（无远端、形态不认识、提交号非十六进制）返回 `None`，界面据此提示，
//!   而不是拼一个看起来能用的假地址。

use crate::error::GitError;
use crate::process::{RunOptions, run, run_allow_fail};
use std::path::Path;

/// 折算提交链接；`remote` 是 `git remote get-url` 的原文，`oid` 需为十六进制。
///
/// 返回 `None` 表示形态不认识或输入不可信，调用方应提示用户而不是硬拼地址。
pub(crate) fn commit_link(remote: &str, oid: &str) -> Option<String> {
    if oid.is_empty() || !oid.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let (authority, path, keep_port) = split_remote(remote)?;
    // `user@host` / `oauth:tok@host` 的凭据段一律丢掉。
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    let host = match authority.split_once(':') {
        // ssh 端口与网页端口通常不同且无从推断，只保留主机名；
        // http(s) 的端口就是网页端口，必须留着。
        Some((host, port)) if keep_port => {
            if port.is_empty() || !port.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            host
        }
        Some((host, _)) => host,
        None => authority,
    };
    if !is_plausible_host(host) {
        return None;
    }
    let port = if keep_port {
        authority
            .split_once(':')
            .map(|(_, port)| format!(":{port}"))
            .unwrap_or_default()
    } else {
        String::new()
    };
    let mut path = path.trim_matches('/');
    path = path.strip_suffix(".git").unwrap_or(path);
    if path.is_empty() || !path.chars().all(is_path_char) {
        return None;
    }
    let tail = if host.contains("gitlab") || path.contains("gitlab") {
        "/-/commit/"
    } else {
        "/commit/"
    };
    Some(format!("https://{host}{port}/{path}{tail}{oid}"))
}

/// 拆出「主机段（可含用户与端口）、路径、端口是否属于网页端口」。
fn split_remote(remote: &str) -> Option<(&str, &str, bool)> {
    let remote = remote.trim();
    for (scheme, keep_port) in [
        ("ssh://", false),
        ("git://", false),
        ("https://", true),
        ("http://", true),
    ] {
        if let Some(rest) = remote.strip_prefix(scheme) {
            return Some(slash_split(rest, keep_port));
        }
    }
    // 本机路径、共享目录不是可浏览地址。
    if remote.starts_with("file://") || remote.starts_with('/') || remote.starts_with("\\\\") {
        return None;
    }
    // scp 式 `git@host:group/repo.git`：冒号前不允许出现斜杠，否则不是主机段。
    let (head, path) = remote.split_once(':')?;
    if head.is_empty() || head.contains('/') || path.trim().is_empty() {
        return None;
    }
    Some((head, path, false))
}

/// 按第一个斜杠切出主机段与路径段。
fn slash_split(rest: &str, keep_port: bool) -> (&str, &str, bool) {
    match rest.find('/') {
        Some(at) => (&rest[..at], &rest[at + 1..], keep_port),
        None => (rest, "", keep_port),
    }
}

/// 主机名白名单：字母数字与 `. _ -`，且必须含点（无点视为本机，不给浏览）。
fn is_plausible_host(host: &str) -> bool {
    host.contains('.')
        && !host.starts_with('.')
        && !host.ends_with('.')
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))
}

/// 路径允许的字符：仓库名里会出现 `.` `-` `_` 与 `/`；空格、`?`、`#` 等
/// 会改写查询串或片段的字符一律拒绝。
fn is_path_char(c: char) -> bool {
    c == '/' || c == '-' || c == '_' || c == '.' || c.is_alphanumeric()
}

/// 该提交在托管服务上的网页地址；无远端或形态不认识时返回 `None`。
///
/// 取哪个远端：优先 `origin`，否则按 `git remote` 给出的顺序取第一个——与
/// IDEA 一样只浏览一个主远端，不做多远端选择器。
pub(crate) async fn commit_url(root: &Path, oid: &str) -> Result<Option<String>, GitError> {
    let output = run(root, &["remote"], RunOptions::default()).await?;
    let names = output.lines();
    let Some(selected) = names
        .iter()
        .find(|name| name.as_str() == "origin")
        .or_else(|| names.first())
    else {
        return Ok(None);
    };
    let url = run_allow_fail(
        root,
        &["remote", "get-url", selected.as_str()],
        RunOptions::default(),
    )
    .await?;
    if !url.ok() {
        log::debug!("读取远端地址失败，远端: {selected}");
        return Ok(None);
    }
    let remote = String::from_utf8_lossy(&url.stdout).trim().to_string();
    Ok(commit_link(&remote, oid))
}

#[cfg(test)]
mod tests {
    use super::commit_link;

    const OID: &str = "0123456789abcdef0123456789abcdef01234567";

    #[test]
    fn scp_style_becomes_https_commit_link() {
        assert_eq!(
            commit_link("git@github.com:someone/transom.git", OID).as_deref(),
            Some(
                "https://github.com/someone/transom/commit/0123456789abcdef0123456789abcdef01234567"
            )
        );
    }

    #[test]
    fn ssh_style_drops_user_and_port() {
        // ssh 的 29418 不是网页端口，无从推断，只保留主机名。
        assert_eq!(
            commit_link("ssh://git@git.internal.corp:29418/team/mirror.git", OID).as_deref(),
            Some(
                "https://git.internal.corp/team/mirror/commit/0123456789abcdef0123456789abcdef01234567"
            )
        );
    }

    #[test]
    fn https_style_keeps_port_and_strips_credentials() {
        assert_eq!(
            commit_link("https://oauth:tok@git.corp:8443/a/b", OID).as_deref(),
            Some("https://git.corp:8443/a/b/commit/0123456789abcdef0123456789abcdef01234567")
        );
    }

    #[test]
    fn gitlab_style_uses_dash_segment() {
        assert_eq!(
            commit_link("git@gitlab.com:group/proj.git", OID).as_deref(),
            Some("https://gitlab.com/group/proj/-/commit/0123456789abcdef0123456789abcdef01234567")
        );
        assert!(
            commit_link("https://gitlab.corp/t/x.git", OID)
                .is_some_and(|url| url.contains("/-/commit/")),
            "自建 GitLab 也应按 GitLab 形态"
        );
    }

    #[test]
    fn azure_and_gitea_use_plain_commit_segment() {
        assert!(
            commit_link("https://dev.azure.com/org/proj/_git/repo", OID)
                .is_some_and(|url| url
                    .ends_with("/_git/repo/commit/0123456789abcdef0123456789abcdef01234567")),
            "Azure DevOps 的提交路径就是 /commit/<oid>"
        );
        assert!(
            commit_link("git@gitee.com:someone/x.git", OID)
                .is_some_and(|url| url.starts_with("https://gitee.com/someone/x/commit/"))
        );
    }

    #[test]
    fn rejects_untrusted_or_unbrowseable_input() {
        // 提交号必须十六进制：界面传什么都不可能拼出注入。
        assert_eq!(
            commit_link("git@github.com:a/b.git", "../../etc/passwd"),
            None
        );
        assert_eq!(commit_link("git@github.com:a/b.git", ""), None);
        // 本机与共享目录不是可浏览地址。
        assert_eq!(commit_link("file:///srv/repo", OID), None);
        assert_eq!(commit_link("/srv/repo", OID), None);
        assert_eq!(commit_link(r"\\nas\share\repo", OID), None);
        // 畸形主机与空路径。
        assert_eq!(commit_link("git@github.com:", OID), None);
        assert_eq!(commit_link("git@localhost:a/b.git", OID), None);
        assert_eq!(commit_link("https://host:notaport/x.git", OID), None);
    }

    #[test]
    fn path_injection_cannot_rewrite_the_url() {
        // 远端地址是克隆来的外部数据：带空格、`?`、`#` 的路径一律拒绝，
        // 否则查询串或片段会把我们拼好的地址改写掉。
        assert_eq!(commit_link("git@github.com:a/b?x=1.git", OID), None);
        assert_eq!(commit_link("git@github.com:a/b#frag.git", OID), None);
        assert_eq!(commit_link("git@github.com:a/b c.git", OID), None);
    }
}
