//! 本地快捷路径解析：主目录、桌面与下载目录。
//!
//! 桌面/下载目录不能只拼 `$HOME/Desktop`、`$HOME/Downloads`：中文系统的 XDG
//! 用户目录通常叫“桌面”“下载”。这里读取 `user-dirs.dirs`，展开 `$HOME` 后取
//! 实际存在的目录；配置缺失或路径失效时再回退到英文目录名。

use super::local_home;
use std::path::{Path, PathBuf};

/// 本地快捷路径。目录不存在时为 `None`，界面据此禁用对应按钮。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct LocalQuickPaths {
    pub(crate) home: Option<PathBuf>,
    pub(crate) desktop: Option<PathBuf>,
    pub(crate) downloads: Option<PathBuf>,
}

impl LocalQuickPaths {
    /// 解析当前用户的常用目录。只在面板创建时调用，避免逐帧读配置文件。
    pub(crate) fn resolve() -> Self {
        let home = local_home();
        let home_dir = home.is_dir().then(|| home.clone());
        let config_root = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"));
        let user_dirs = std::fs::read_to_string(config_root.join("user-dirs.dirs"))
            .ok()
            .map(|content| parse_user_dirs(&content, &home))
            .unwrap_or_default();
        Self {
            home: home_dir,
            desktop: first_existing([user_dirs.desktop, Some(home.join("Desktop"))]),
            downloads: first_existing([user_dirs.downloads, Some(home.join("Downloads"))]),
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct ParsedUserDirs {
    desktop: Option<PathBuf>,
    downloads: Option<PathBuf>,
}

/// XDG 指定目录不存在时回退到传统英文目录名，两者都存在则优先 XDG。
fn first_existing(candidates: [Option<PathBuf>; 2]) -> Option<PathBuf> {
    candidates.into_iter().flatten().find(|path| path.is_dir())
}

fn parse_user_dirs(content: &str, home: &Path) -> ParsedUserDirs {
    let mut parsed = ParsedUserDirs::default();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let Some(path) = expand_home(value.trim().trim_matches('"'), home) else {
            continue;
        };
        match key.trim() {
            "XDG_DESKTOP_DIR" => parsed.desktop = Some(path),
            "XDG_DOWNLOAD_DIR" => parsed.downloads = Some(path),
            _ => {}
        }
    }
    parsed
}

fn expand_home(value: &str, home: &Path) -> Option<PathBuf> {
    if value.is_empty() {
        return None;
    }
    let expanded = if let Some(rest) = value.strip_prefix("$HOME/") {
        home.join(rest)
    } else if value == "$HOME" {
        home.to_path_buf()
    } else if let Some(rest) = value.strip_prefix("${HOME}/") {
        home.join(rest)
    } else if value == "${HOME}" {
        home.to_path_buf()
    } else {
        PathBuf::from(value)
    };
    expanded.is_absolute().then_some(expanded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_home_expanded_xdg_dirs() {
        let home = Path::new("/home/tester");
        let parsed = parse_user_dirs(
            r#"# XDG user dirs
XDG_DESKTOP_DIR="$HOME/桌面"
XDG_DOWNLOAD_DIR="${HOME}/下载"
XDG_DOCUMENTS_DIR="$HOME/文档"
"#,
            home,
        );
        assert_eq!(parsed.desktop, Some(home.join("桌面")));
        assert_eq!(parsed.downloads, Some(home.join("下载")));
    }

    #[test]
    fn parses_absolute_and_ignores_malformed_lines() {
        let home = Path::new("/home/tester");
        let parsed = parse_user_dirs(
            r#"XDG_DESKTOP_DIR="/data/Desktop"
not-a-pair
XDG_DOWNLOAD_DIR=
XDG_DOWNLOAD_DIR="relative/path"
"#,
            home,
        );
        assert_eq!(parsed.desktop, Some(PathBuf::from("/data/Desktop")));
        assert_eq!(parsed.downloads, None);
    }

    #[test]
    fn expands_home_only_at_path_start() {
        let home = Path::new("/home/tester");
        assert_eq!(expand_home("$HOME/a", home), Some(home.join("a")));
        assert_eq!(expand_home("${HOME}/b", home), Some(home.join("b")));
        assert_eq!(expand_home("$HOME", home), Some(home.to_path_buf()));
        assert_eq!(expand_home("prefix/$HOME", home), None);
        assert_eq!(expand_home("", home), None);
    }
}
