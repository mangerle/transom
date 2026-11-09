//! 克隆地址解析、路径拼接与进度百分比解析。

use std::path::Path;

/// 从一段进度文本里取最新的百分比。
///
/// git 用 `\r` 反复刷新同一行，控制器按行拼接后送上来；这里取最后一个 `%`
/// 前面的数字，取不到（还没开始接收对象）时返回 `None`，进度条走不确定动画。
pub(crate) fn clone_percent(text: &str) -> Option<f32> {
    let percent_index = text.rfind('%')?;
    let prefix = &text[..percent_index];
    let number: String = prefix
        .chars()
        .rev()
        .take_while(|ch| ch.is_ascii_digit() || *ch == '.')
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    number
        .parse::<f32>()
        .ok()
        .map(|value| value.clamp(0.0, 100.0))
}

/// 从克隆地址里取仓库名：取最后一个路径段并去掉 `.git` 后缀。
///
/// 支持 `https://host/org/repo.git`、`ssh://user@host:port/org/repo` 与
/// `git@host:org/repo.git`；只有主机没有路径时返回 `None`。
pub(crate) fn repo_name_from_url(url: &str) -> Option<String> {
    let trimmed = url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    let tail = if let Some((_, rest)) = trimmed.split_once("://") {
        let (_, path) = rest.split_once('/')?;
        path.trim_end_matches('/').rsplit('/').next()?
    } else {
        trimmed.rsplit(['/', ':']).next()?
    };
    let name = tail.strip_suffix(".git").unwrap_or(tail);
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\']) {
        return None;
    }
    Some(name.to_string())
}

/// 把仓库名补到目录末尾：`previous_repo` 是上一次自动追加的那一段，
/// 只有当前目录仍以它结尾时才摘掉，用户手改过的目录不被误删。
pub(crate) fn clone_destination(
    current: &str,
    previous_repo: Option<&str>,
    repo: &str,
    fallback_base: &str,
) -> String {
    let current = Path::new(current.trim());
    let should_replace_previous = previous_repo.is_some_and(|previous| {
        current.file_name().and_then(|name| name.to_str()) == Some(previous)
    });
    let base = if should_replace_previous {
        current.parent().unwrap_or_else(|| Path::new(""))
    } else {
        current
    };
    let base = if base.as_os_str().is_empty() {
        Path::new(fallback_base.trim())
    } else {
        base
    };
    base.join(repo).display().to_string()
}

#[cfg(test)]
mod tests {
    use super::{clone_destination, clone_percent, repo_name_from_url};
    use std::path::Path;

    #[test]
    fn repo_name_supports_ssh_https_and_scp_forms() {
        assert_eq!(
            repo_name_from_url("ssh://wangle@192.168.180.150:29418/AB5/absf").as_deref(),
            Some("absf")
        );
        assert_eq!(
            repo_name_from_url("https://host/org/repo.git").as_deref(),
            Some("repo")
        );
        assert_eq!(
            repo_name_from_url("git@host:org/repo.git").as_deref(),
            Some("repo")
        );
        assert_eq!(
            repo_name_from_url("http://host/repo/").as_deref(),
            Some("repo")
        );
        assert_eq!(repo_name_from_url("https://host"), None);
    }

    #[test]
    fn destination_appends_and_replaces_only_the_previous_auto_repo() {
        #[cfg(unix)]
        let base = "/home/wangle/data";
        #[cfg(windows)]
        let base = r"C:\home\wangle\data";
        #[cfg(unix)]
        let fallback = "/home/wangle";
        #[cfg(windows)]
        let fallback = r"C:\home\wangle";
        let previous = Path::new(base).join("absf").display().to_string();
        assert_eq!(
            clone_destination(base, None, "absf", fallback),
            Path::new(base).join("absf").display().to_string()
        );
        assert_eq!(
            clone_destination(&previous, Some("absf"), "xyz", fallback),
            Path::new(base).join("xyz").display().to_string()
        );
        assert_eq!(
            clone_destination(
                &Path::new(fallback).join("manual").display().to_string(),
                Some("absf"),
                "xyz",
                fallback,
            ),
            Path::new(fallback)
                .join("manual")
                .join("xyz")
                .display()
                .to_string()
        );
        assert_eq!(
            clone_destination("", None, "absf", fallback),
            Path::new(fallback).join("absf").display().to_string()
        );
    }

    #[test]
    fn clone_percent_reads_the_latest_percentage() {
        assert_eq!(
            clone_percent("Receiving objects:  45% (12/34)\nResolving deltas: 100% (5/5)"),
            Some(100.0)
        );
        assert_eq!(clone_percent("Cloning into 'repo'..."), None);
    }
}
