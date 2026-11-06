//! 克隆地址解析、路径拼接与进度百分比解析。

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
    let current = current.trim().trim_end_matches('/');
    let base = previous_repo
        .and_then(|previous| current.strip_suffix(&format!("/{previous}")))
        .unwrap_or(current);
    let base = if base.is_empty() {
        fallback_base.trim_end_matches('/')
    } else {
        base
    };
    std::path::Path::new(base).join(repo).display().to_string()
}

#[cfg(test)]
mod tests {
    use super::{clone_destination, clone_percent, repo_name_from_url};

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
        assert_eq!(
            clone_destination("/home/wangle/data", None, "absf", "/home/wangle"),
            "/home/wangle/data/absf"
        );
        assert_eq!(
            clone_destination(
                "/home/wangle/data/absf",
                Some("absf"),
                "xyz",
                "/home/wangle"
            ),
            "/home/wangle/data/xyz"
        );
        assert_eq!(
            clone_destination("/tmp/manual", Some("absf"), "xyz", "/home/wangle"),
            "/tmp/manual/xyz"
        );
        assert_eq!(
            clone_destination("", None, "absf", "/home/wangle"),
            "/home/wangle/absf"
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
