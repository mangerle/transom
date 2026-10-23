//! 连接就绪后的初始化命令调度。
//!
//! 命令必须写入交互式 Shell 才能保留 `cd`、环境变量与提权状态，因此这里
//! 通过 `TerminalView::send_text` 注入，而不是复用独立的 exec 通道。
//! 远端 PAM/MOTD 输出可能晚于连接握手完成，发送前保留一个短暂的稳定窗口；
//! 每条命令之间也留出间隔，降低多行粘贴被 Shell 合并解析的概率。

use crate::app::AppShell;
use gpui_kit::*;
use std::time::Duration;
use transom_core::{ConnectionId, TabId};

/// 连接建立到第一条命令之间的等待时间。
const STARTUP_GRACE: Duration = Duration::from_millis(700);
/// 相邻命令之间的等待时间。
const STARTUP_INTERVAL: Duration = Duration::from_millis(250);

impl AppShell {
    /// 为已就绪的终端安排初始化命令。
    ///
    /// 任务在标签关闭、重连或连接切换时通过 `_startup_task` 句柄取消；
    /// 每轮写入前都会校验 `connection_id`，避免旧连接的命令误写到新连接。
    pub(crate) fn schedule_startup_commands(
        &mut self,
        tab_id: TabId,
        connection_id: ConnectionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(commands) = self
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id && tab.connection_id == Some(connection_id))
            .map(|tab| tab.session.startup.commands.clone())
            .filter(|commands| !commands.is_empty())
        else {
            return;
        };

        let task = cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(STARTUP_GRACE).await;
            for command in commands {
                let delivered =
                    this.update_in(cx, |this, _window, cx| {
                        let Some(tab) = this.tabs.iter().find(|tab| {
                            tab.id == tab_id && tab.connection_id == Some(connection_id)
                        }) else {
                            return false;
                        };
                        let Some(terminal) = tab.terminal.clone() else {
                            return false;
                        };
                        terminal.update(cx, |view, cx| {
                            view.send_text(&format!("{command}\r"), cx);
                        });
                        true
                    });
                if !delivered.unwrap_or(false) {
                    break;
                }
                cx.background_executor().timer(STARTUP_INTERVAL).await;
            }
        });

        if let Some(tab) = self
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id && tab.connection_id == Some(connection_id))
        {
            tab._startup_task = Some(task);
        }
    }
}
