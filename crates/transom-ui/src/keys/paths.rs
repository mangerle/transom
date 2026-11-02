//! 密钥路径辅助：默认 SSH 目录与 `~` 展开。

use std::path::PathBuf;

pub(crate) fn default_ssh_dir() -> PathBuf {
    directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().join(".ssh"))
        .unwrap_or_else(|| PathBuf::from(".ssh"))
}

/// 展开 `~` 与 `~/` 前缀，方便用户按 OpenSSH 习惯输入路径。
pub(crate) fn expand_home(input: &str) -> PathBuf {
    let input = input.trim();
    let Some(dirs) = directories::BaseDirs::new() else {
        return PathBuf::from(input);
    };
    if input == "~" {
        return dirs.home_dir().to_path_buf();
    }
    if let Some(rest) = input.strip_prefix("~/") {
        return dirs.home_dir().join(rest);
    }
    PathBuf::from(input)
}
