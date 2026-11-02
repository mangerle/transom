//! 终端就绪事件：创建终端视图并完成连接收尾。

use super::event_handlers::TerminalSpawn;
use super::event_handlers::apply_global_terminal_profile;
use super::*;
use crate::app::AppShell;
use transom_core::TerminalAppEvent;
use transom_terminal::{KeyFilter, TerminalEvent};

pub(super) struct TerminalReadyFinish {
    tab_id: transom_core::TabId,
    connection_id: transom_core::ConnectionId,
    session_id: transom_core::SessionId,
    focus_handle: gpui_kit::FocusHandle,
}

/// 终端就绪时一次性取好的启动参数。
pub(super) struct TerminalReadySpawnSettings {
    profile: transom_core::TerminalProfile,
    interaction: transom_core::TerminalInteraction,
    key_filter: KeyFilter,
    audit_path: Option<std::path::PathBuf>,
    shell_integration_enabled: bool,
}

impl AppShell {
    /// 连接建立：构造终端视图、挂接订阅与外观覆盖，并处理焦点与启动命令。
    ///
    /// 返回 `false` 表示标签或连接已过期（迟到的完成事件），不触发重绘。
    /// 终端就绪时的档位快照：配置档、交互、键过滤、审计路径与集成开关。
    pub(super) fn terminal_ready_spawn_settings(
        &self,
        index: usize,
        cx: &mut Context<Self>,
    ) -> TerminalReadySpawnSettings {
        let session_id = self.state.workspace.tabs[index].session_id;
        let mut profile = self
            .state
            .catalog
            .sessions
            .iter()
            .find(|session| session.id == session_id)
            .map(|session| session.terminal.clone())
            .unwrap_or_else(|| self.state.workspace.tabs[index].session.terminal.clone());
        apply_global_terminal_profile(&mut profile, cx);
        let interaction = app_settings(cx).terminal_interaction;
        let reserved = self.state.shortcut_recording.filter.clone();
        let key_filter: KeyFilter = Arc::new(move |keystroke| {
            reserved
                .read()
                .iter()
                .any(|binding| keystroke.should_match(binding))
        });
        // 审计落盘按连接粒度开启：路径在派发前算好，读线程内才创建文件。
        let audit_path = crate::audit::audit_path_for(
            &self.state.workspace.tabs[index],
            app_settings(cx).audit.enabled,
        );
        // 开关在闭包外取值：`cx.new` 的闭包里再读 `self` 会与外层借用冲突。
        // 命令执行标签（X-TerminalArgExec）不是交互式 Shell：集成探针会直接写进
        // 目标程序，必须关闭，否则 htop 这类全屏程序会收到一串注入序列。
        let exec_local = matches!(
            &self.state.workspace.tabs[index].session.connection,
            transom_core::Connection::Local(local) if local.command.is_some()
        );
        let shell_integration_enabled =
            app_settings(cx).block_view.shell_integration && !exec_local;
        TerminalReadySpawnSettings {
            profile,
            interaction,
            key_filter,
            audit_path,
            shell_integration_enabled,
        }
    }

    pub(crate) fn on_terminal_ready(
        &mut self,
        event: TerminalAppEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let TerminalAppEvent::TerminalReady {
            tab_id,
            connection_id,
            io,
        } = event
        else {
            return true;
        };
        let Some(index) = self.state.workspace.tab_index(tab_id) else {
            return false;
        };
        if self.state.workspace.tabs[index].connection_id != Some(connection_id) {
            return false;
        }
        let session_id = self.state.workspace.tabs[index].session_id;
        let spawn_settings = self.terminal_ready_spawn_settings(index, cx);
        let terminal = self.spawn_terminal_view(
            window,
            cx,
            TerminalSpawn {
                io,
                profile: spawn_settings.profile,
                interaction: spawn_settings.interaction,
                key_filter: spawn_settings.key_filter,
                audit_path: spawn_settings.audit_path,
                shell_integration_enabled: spawn_settings.shell_integration_enabled,
                session_id,
            },
        );
        let focus_handle = terminal.read(cx).focus_handle().clone();
        // 用 subscribe_in：通知要不要弹取决于"窗口是否被系统聚焦"，
        // 普通 subscribe 拿不到 Window。
        let subscription = cx.subscribe_in(
            &terminal,
            window,
            move |this, _, event: &TerminalEvent, window, cx| {
                this.handle_terminal_event(tab_id, event, window, cx)
            },
        );
        let tab = &mut self.state.workspace.tabs[index];
        tab.status = TabStatus::Connected;
        if let Some(pane) = tab.session_pane_mut() {
            pane.terminal = Some(terminal);
            pane.reconnect_attempts = 0;
            pane.network_traffic = None;
            pane._subscription = Some(subscription);
        }
        log::info!(
            "连接建立完成，窗格: {tab_id}，连接: {connection_id:?}，会话: {}，目标: {}",
            tab.session.name,
            crate::connection::describe_target(&tab.session)
        );
        self.finish_terminal_ready(
            TerminalReadyFinish {
                tab_id,
                connection_id,
                session_id,
                focus_handle,
            },
            window,
            cx,
        );
        true
    }

    /// 终端就绪收尾：焦点/组内激活、凭据落盘、启动命令、转发与进程采样。
    pub(super) fn finish_terminal_ready(
        &mut self,
        finish: TerminalReadyFinish,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let TerminalReadyFinish {
            tab_id,
            connection_id,
            session_id,
            focus_handle,
        } = finish;
        // 连接建立期间用户可能已经切走：启动期的默认标签与深链目录标签
        // 会并发建连，晚完成者若无条件激活，会把焦点从后创建的深链标签
        // 抢回启动标签。只有该窗格仍是当前选中项时才切换组内焦点并聚焦。
        let still_active = self.state.workspace.active_tab == Some(tab_id);
        if still_active && let Some(group_index) = self.state.workspace.group_index_for_tab(tab_id)
        {
            self.state.workspace.groups[group_index].active_pane = tab_id;
        }
        // 认证已确定通过，此刻才允许把用户勾选保存的凭据写入钥匙串。
        self.flush_pending_credential(session_id, connection_id, cx);
        if still_active {
            window.focus(&focus_handle, cx);
        }
        self.schedule_startup_commands(tab_id, connection_id, window, cx);
        self.auto_start_forwards(tab_id, connection_id, session_id, cx);
        if still_active && self.state.monitor.read(cx).is_open() {
            self.sample_processes(self.state.monitor.read(cx).processes_collapsed(), cx);
        }
    }
}
