//! 已有终端会话中的目录切换入口与安全前置条件。

use super::shell_command::{directory_change_command, fallback_directory_change_command};
use crate::engine::PromptInputState;
use gpui_kit::Context;
use thiserror::Error;

/// 已有终端无法安全切换目录的原因。
///
/// 设计原理（Why）：上层需要区分“连接已结束”“Shell 状态未知”“正在运行命令”
/// 与“用户已有未提交输入”，这些分支对用户提示和后续排查完全不同，不能折叠成
/// 一个笼统错误字符串。
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TerminalDirectoryError {
    /// 终端进程已经退出，无法再写入任何输入。
    #[error("对应终端会话已结束")]
    TerminalExited,
    /// Shell 集成尚未确认可用，无法确定提示符与输入状态。
    #[error("无法确认对应终端的 Shell 状态，Shell 集成未就绪")]
    ShellUnknown,
    /// 当前 Shell 无法生成安全的目录切换命令。
    #[error("对应终端 Shell 不支持安全的目录切换，Shell: {shell}")]
    UnsupportedShell { shell: &'static str },
    /// 尚未收到足以确认空闲提示符的 Shell 标记。
    #[error("对应终端尚未确认处于空闲提示符状态")]
    PromptUnknown,
    /// 用户已经编辑当前提示符的输入行。
    #[error("对应终端已有未提交输入")]
    PromptEdited,
    /// 前台命令仍在运行。
    #[error("对应终端正在执行命令")]
    CommandRunning,
    /// 输入通道已经关闭。
    #[error("对应终端输入通道已关闭")]
    InputClosed,
    /// 目标路径无法安全构造成 Shell 命令。
    #[error("终端目录路径无效，路径: {path}")]
    InvalidPath { path: String },
}

impl crate::TerminalView {
    /// 在已有终端中切换到指定目录。
    ///
    /// 设计原理（Why）：目录动作不修改调用方当前输入，也不新开会话；优先在确认提示符
    /// 空闲且输入行为空时写入经过方言转义的 `cd`。在未配置 Shell 集成或状态未知的普通
    /// 会话中，只要终端未退出、未处于全屏程序且无进行中的命令，采用安全转义后的通用
    /// `cd` 命令进行优雅降级，避免普通 SSH 会话功能不可用。
    ///
    /// # Errors
    ///
    /// - 终端已退出或输入通道关闭；
    /// - 终端正处于全屏程序、正在运行命令或用户已有未提交输入；
    /// - 目标路径为空或包含控制字符。
    pub fn change_directory(
        &mut self,
        path: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), TerminalDirectoryError> {
        if self.exited {
            return Err(TerminalDirectoryError::TerminalExited);
        }
        if path.is_empty() || path.chars().any(char::is_control) {
            return Err(TerminalDirectoryError::InvalidPath {
                path: path.to_string(),
            });
        }

        // 先把已经到达但尚未解析的输出并入引擎，避免依据过期提示符状态写入命令。
        self.drain_pending_output(cx);

        let (prompt_state, is_alt_screen) = {
            let engine = self.state.engine.lock();
            (engine.prompt_input_state(), engine.is_alt_screen())
        };
        if is_alt_screen {
            return Err(TerminalDirectoryError::CommandRunning);
        }
        match prompt_state {
            PromptInputState::Empty | PromptInputState::Unknown => {}
            PromptInputState::Edited => return Err(TerminalDirectoryError::PromptEdited),
            PromptInputState::Running => return Err(TerminalDirectoryError::CommandRunning),
        }

        let command = self
            .shell_integration
            .shell_kind()
            .and_then(|kind| directory_change_command(kind, path))
            .or_else(|| fallback_directory_change_command(path))
            .ok_or_else(|| TerminalDirectoryError::InvalidPath {
                path: path.to_string(),
            })?;

        if !self.send_text(&format!("{command}\r"), cx) {
            return Err(TerminalDirectoryError::InputClosed);
        }
        self.state
            .engine
            .lock()
            .mark_programmatic_command_submitted();
        Ok(())
    }
}
