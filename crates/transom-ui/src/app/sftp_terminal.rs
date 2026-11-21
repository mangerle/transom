//! SFTP 目录快捷动作到已有终端会话的桥接。

use super::AppShell;
use crate::sftp::SftpTerminalDirectoryPath;
use gpui_kit::*;
use transom_core::ConnectionId;
use transom_terminal::TerminalDirectoryError;

impl AppShell {
    /// 激活连接对应的已有终端，并请求它进入指定目录。
    ///
    /// 设计原理（Why）：目录快捷动作不创建新会话；连接号在运行期唯一标识一个终端
    /// 标签，因此这里按连接号定位目标。真正的提示符与输入行安全检查归终端实体，
    /// 外壳只负责跨实体激活、调用和把拒绝原因转成应用内状态提示。
    pub(super) fn change_terminal_directory(
        &mut self,
        connection_id: ConnectionId,
        path: SftpTerminalDirectoryPath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path_text) = terminal_path_text(&path) else {
            self.set_transient_status(
                rust_i18n::t!("sftp.terminal_directory.invalid_path").to_string(),
                cx,
            );
            return;
        };
        let Some(tab_id) = self
            .state
            .workspace
            .tabs
            .iter()
            .find(|tab| tab.connection_id() == Some(connection_id) && tab.terminal().is_some())
            .map(|tab| tab.id)
        else {
            self.set_transient_status(
                rust_i18n::t!("sftp.terminal_directory.no_session").to_string(),
                cx,
            );
            return;
        };
        let Some(terminal) = self
            .state
            .workspace
            .tab(tab_id)
            .and_then(|tab| tab.terminal())
        else {
            self.set_transient_status(
                rust_i18n::t!("sftp.terminal_directory.no_session").to_string(),
                cx,
            );
            return;
        };

        self.activate_tab(tab_id, window, cx);
        let result = terminal.update(cx, |view, cx| view.change_directory(&path_text, cx));
        let message = match result {
            Ok(()) => {
                log::debug!(
                    "SFTP 目录请求已交给已有终端，标签: {tab_id}，连接: {connection_id:?}，路径: {path_text}"
                );
                rust_i18n::t!("sftp.terminal_directory.changed", path = path_text).to_string()
            }
            Err(error) => terminal_directory_error_text(&error),
        };
        self.set_transient_status(message, cx);
    }
}

/// 把目录事件中的平台路径或远端路径转换为可传给终端的文本。
fn terminal_path_text(path: &SftpTerminalDirectoryPath) -> Option<String> {
    match path {
        SftpTerminalDirectoryPath::Local(path) => path.to_str().map(str::to_owned),
        SftpTerminalDirectoryPath::Remote(path) => Some(path.clone()),
    }
}

/// 把终端层的精细错误映射为面向用户的中文提示。
fn terminal_directory_error_text(error: &TerminalDirectoryError) -> String {
    match error {
        TerminalDirectoryError::TerminalExited => {
            rust_i18n::t!("sftp.terminal_directory.exited").to_string()
        }
        TerminalDirectoryError::ShellUnknown => {
            rust_i18n::t!("sftp.terminal_directory.shell_unknown").to_string()
        }
        TerminalDirectoryError::UnsupportedShell { .. } => {
            rust_i18n::t!("sftp.terminal_directory.unsupported_shell").to_string()
        }
        TerminalDirectoryError::PromptUnknown => {
            rust_i18n::t!("sftp.terminal_directory.prompt_unknown").to_string()
        }
        TerminalDirectoryError::PromptEdited => {
            rust_i18n::t!("sftp.terminal_directory.input_pending").to_string()
        }
        TerminalDirectoryError::CommandRunning => {
            rust_i18n::t!("sftp.terminal_directory.running").to_string()
        }
        TerminalDirectoryError::InputClosed => {
            rust_i18n::t!("sftp.terminal_directory.input_closed").to_string()
        }
        TerminalDirectoryError::InvalidPath { .. } => {
            rust_i18n::t!("sftp.terminal_directory.invalid_path").to_string()
        }
    }
}
