//! 外部唤起请求。

use std::path::PathBuf;

/// 外部唤起请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeepLinkRequest {
    Ssh {
        url: String,
    },
    Dir {
        path: PathBuf,
    },
    /// 在本地终端里直接执行命令（`X-TerminalArgExec` / `-e`）。
    ///
    /// `dir` 是可选工作目录：`xdg-terminal-exec` 会把 `--dir=` 与 `-e` 一起
    /// 传入，两条信息必须随同一个请求到达，否则会先开一个多余的 Shell 标签。
    Exec {
        command: Vec<String>,
        dir: Option<PathBuf>,
    },
    Activate,
}
