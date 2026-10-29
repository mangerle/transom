//! 终端 OSC 8 代码位置链接的应用层落地：远端路径解析、编辑器跳行与本地兜底。
//!
//! 设计原理（Why）：
//! - 终端 crate 不认识 SFTP 与会话，只投递"路径 + 行 + 列"；把它映射到哪一侧
//!   文件系统是应用层职责。当前标签有 SSH 连接时走远端编辑器，本地标签没有
//!   内置本地编辑器，退化为系统默认程序打开文件（不强行造第二套编辑器）。
//! - 相对路径的基准取当前 SFTP 面板的远端目录：那是唯一可靠的远端 cwd 来源；
//!   面板未打开时无法确定基准，宁可明确拒绝也不猜一个目录去读文件。

use crate::app::AppShell;
use crate::editor::RemoteCodeLocation;
use gpui_kit::*;
use transom_core::TabId;

impl AppShell {
    /// 处理终端投递的代码位置链接。
    pub(crate) fn open_terminal_code_location(
        &mut self,
        tab_id: TabId,
        path: &str,
        line: u32,
        column: Option<u32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.workspace.tab(tab_id) else {
            return;
        };
        let connection_id = tab.connection_id;
        let remote_dir = tab.sftp().map(|panel| panel.remote.path.clone());
        let Some(resolved) = resolve_code_path(path, remote_dir.as_deref()) else {
            self.status =
                Some(rust_i18n::t!("editor.error_unresolved_link", path = path).to_string());
            cx.notify();
            return;
        };
        let Some(connection_id) = connection_id else {
            // 本地标签：直接用系统默认程序打开文件；行号跳转只在内置编辑器可用。
            let local = std::path::Path::new(&resolved);
            if local.is_absolute() {
                crate::audit::open_local_path(local);
            } else {
                self.status =
                    Some(rust_i18n::t!("editor.error_local_absolute", path = resolved).to_string());
                cx.notify();
            }
            return;
        };
        self.open_remote_code_location(
            RemoteCodeLocation {
                tab_id,
                connection_id,
                remote_path: resolved,
                line,
                column,
            },
            window,
            cx,
        );
    }
}

/// 绝对路径原样使用；相对路径以远端当前目录为基准拼接。
///
/// `..` 组件在终端侧已被拒绝，这里只需处理目录末尾斜杠与根目录拼接。
fn resolve_code_path(path: &str, remote_dir: Option<&str>) -> Option<String> {
    if path.starts_with('/') {
        return Some(path.to_string());
    }
    let dir = remote_dir?;
    if dir == "/" {
        Some(format!("/{path}"))
    } else {
        Some(format!("{}/{}", dir.trim_end_matches('/'), path))
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_code_path;

    #[test]
    fn absolute_paths_ignore_remote_dir() {
        assert_eq!(
            resolve_code_path("/etc/nginx/nginx.conf", Some("/home/dev")),
            Some("/etc/nginx/nginx.conf".to_string())
        );
    }

    #[test]
    fn relative_paths_join_remote_cwd() {
        assert_eq!(
            resolve_code_path("src/main.rs", Some("/home/dev/app")),
            Some("/home/dev/app/src/main.rs".to_string())
        );
        assert_eq!(
            resolve_code_path("src/main.rs", Some("/")),
            Some("/src/main.rs".to_string())
        );
    }

    #[test]
    fn relative_paths_without_cwd_are_rejected() {
        assert_eq!(resolve_code_path("src/main.rs", None), None);
    }
}
