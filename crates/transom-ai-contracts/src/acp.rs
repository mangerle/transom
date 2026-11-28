//! ACP 命令发现与用户级工具目录解析。
//!
//! 设计原理（Why）：界面需要展示外部 Agent 是否已安装，而该判断只依赖环境
//! 变量与文件元数据，不应迫使 UI 链接进程启动、网络客户端或模型运行时。

use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// 返回用户级免 root 安装的 ACP 工具 bin 目录（`$HOME/.transom/acp-tools/bin`）。
pub fn acp_tools_bin_dir() -> Option<PathBuf> {
    let home = env::var_os("HOME").or_else(|| env::var_os("USERPROFILE"))?;
    let base = PathBuf::from(home).join(".transom").join("acp-tools");
    Some(if cfg!(windows) {
        base
    } else {
        base.join("bin")
    })
}

fn augmented_path_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(bin) = acp_tools_bin_dir() {
        dirs.push(bin);
    }
    if let Some(home) = env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join(".local").join("bin"));
    }
    if let Some(path_var) = env::var_os("PATH") {
        dirs.extend(env::split_paths(&path_var));
    }
    dirs
}

/// 返回加入 Transom ACP 工具目录后的完整 `PATH`。
pub fn augmented_path_env() -> Option<OsString> {
    env::join_paths(augmented_path_dirs()).ok()
}

#[cfg(windows)]
fn is_executable_file(candidate: &Path) -> bool {
    candidate.is_file()
        || (candidate.extension().is_none()
            && ["exe", "cmd", "bat", "com"]
                .iter()
                .any(|ext| candidate.with_extension(ext).is_file()))
}

#[cfg(not(windows))]
fn is_executable_file(candidate: &Path) -> bool {
    candidate.is_file()
}

/// 检测外部 ACP Agent 可执行命令是否在当前 `PATH`（或用户级 `.transom/acp-tools/bin`）中可用。
pub fn is_acp_command_available(command: &str) -> bool {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.contains(std::path::MAIN_SEPARATOR) || trimmed.contains('/') {
        return is_executable_file(Path::new(trimmed));
    }
    augmented_path_dirs()
        .into_iter()
        .any(|dir| is_executable_file(&dir.join(trimmed)))
}

#[cfg(test)]
mod tests {
    use super::is_acp_command_available;

    #[test]
    fn empty_command_is_unavailable() {
        assert!(!is_acp_command_available(""));
        assert!(!is_acp_command_available("   "));
    }
}
