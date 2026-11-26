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
    ) -> Option<TerminalReadySpawnSettings> {
        let session_id = self.state.workspace.tabs[index].session_id()?;
        let fallback_terminal = self.state.workspace.tabs[index]
            .session()
            .map(|session| session.terminal.clone())?;
        let mut profile = self
            .state
            .session
            .catalog
            .sessions
            .iter()
            .find(|session| session.id == session_id)
            .map(|session| session.terminal.clone())
            .unwrap_or(fallback_terminal);
        apply_global_terminal_profile(&mut profile, cx);
        let interaction = app_settings(cx).terminal_interaction;
        let reserved = self.state.ui.shortcut_recording.filter.clone();
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
            self.state.workspace.tabs[index].session().map(|session| &session.connection),
            Some(transom_core::Connection::Local(local)) if local.command.is_some()
        );
        let shell_integration_enabled =
            app_settings(cx).block_view.shell_integration && !exec_local;
        Some(TerminalReadySpawnSettings {
            profile,
            interaction,
            key_filter,
            audit_path,
            shell_integration_enabled,
        })
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
        if self.state.workspace.tabs[index].connection_id() != Some(connection_id) {
            return false;
        }
        let Some(session_id) = self.state.workspace.tabs[index].session_id() else {
            return false;
        };
        let Some(spawn_settings) = self.terminal_ready_spawn_settings(index, cx) else {
            return false;
        };
        let existing_terminal = self.state.workspace.tabs[index].terminal();
        let (focus_handle, new_terminal, new_subscription) =
            if let Some(terminal) = existing_terminal {
                let remote_clipboard_allowed = app_settings(cx)
                    .terminal_interaction
                    .default_allow_remote_clipboard_write;
                let triggers = app_settings(cx).triggers.clone();
                let block_override = self
                    .state
                    .session
                    .catalog
                    .sessions
                    .iter()
                    .find(|session| session.id == session_id)
                    .map_or(transom_core::BlockViewOverride::FollowGlobal, |session| {
                        session.block_view
                    });
                let block_view = app_settings(cx).block_view.resolved(block_override);
                terminal.update(cx, |view, cx| {
                    view.reattach_io(
                        transom_terminal::TerminalReattachParams {
                            io,
                            profile: spawn_settings.profile,
                            interaction: spawn_settings.interaction,
                            audit_path: spawn_settings.audit_path,
                            shell_integration_enabled: spawn_settings.shell_integration_enabled,
                            remote_clipboard_allowed,
                        },
                        cx,
                    );
                    view.reload_triggers(&triggers);
                    view.set_block_view(Some(block_view), cx);
                });
                let focus_handle = terminal.read(cx).focus_handle().clone();
                (focus_handle, None, None)
            } else {
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
                (focus_handle, Some(terminal), Some(subscription))
            };
        let session_context = self.state.workspace.tabs[index].session().map(|session| {
            (
                session.name.clone(),
                crate::connection::describe_target(session),
            )
        });
        let tab = &mut self.state.workspace.tabs[index];
        tab.status = TabStatus::Connected;
        if let Some(pane) = tab.session_pane_mut() {
            if let Some(terminal) = new_terminal {
                pane.terminal = Some(terminal);
            }
            pane.reconnect_attempts = 0;
            pane.reconnect_task = None;
            pane.reconnect_detail = None;
            pane.network_traffic = None;
            if let Some(subscription) = new_subscription {
                pane._subscription = Some(subscription);
            }
        }
        if let Some((session_name, target)) = session_context {
            log::info!(
                "连接建立完成，窗格: {tab_id}，连接: {connection_id:?}，会话: {}，目标: {}",
                session_name,
                target
            );
        }
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

    /// FTP 会话就绪：没有终端流可挂，文件面板就是本会话的正文。
    ///
    /// 设计原理（Why）：FTP 标签的交互面只有 SFTP 文件面板，连接成功立即
    /// 打开面板，正文渲染层（`render_pane_body`）检测到 FTP 会话后面板实体
    /// 存在时直接以面板作为窗格正文，不再占用侧槽。
    pub(crate) fn on_ftp_ready(
        &mut self,
        event: TerminalAppEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let TerminalAppEvent::FtpReady {
            tab_id,
            connection_id,
        } = event
        else {
            return true;
        };
        let Some(index) = self.state.workspace.tab_index(tab_id) else {
            return false;
        };
        if self.state.workspace.tabs[index].connection_id() != Some(connection_id) {
            return false;
        }
        let target = self.state.workspace.tabs[index]
            .session()
            .map(crate::connection::describe_target);
        let tab = &mut self.state.workspace.tabs[index];
        tab.status = TabStatus::Connected;
        if let Some(pane) = tab.session_pane_mut() {
            pane.reconnect_attempts = 0;
            pane.reconnect_task = None;
            pane.reconnect_detail = None;
        }
        if let Some(target) = target {
            log::info!("FTP 会话就绪，窗格: {tab_id}，连接: {connection_id:?}，目标: {target}");
        }
        if self.state.sftp_view(tab_id).is_some() {
            crate::sftp::reattach_sftp_panel(&mut self.state, tab_id, connection_id, cx);
        } else {
            crate::sftp::build_sftp_panel(&mut self.state, tab_id, index, true, window, cx);
        }
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
        crate::connection::flush_pending_credential(&mut self.state, session_id, connection_id, cx);
        if still_active {
            window.focus(&focus_handle, cx);
        }
        crate::startup::schedule_startup_commands(
            &mut self.state,
            tab_id,
            connection_id,
            window,
            cx,
        );
        crate::sftp::reattach_sftp_panel(&mut self.state, tab_id, connection_id, cx);
        crate::forward::auto_start_forwards(&mut self.state, tab_id, connection_id, session_id, cx);
        if still_active && self.state.features.monitor.read(cx).is_open() {
            let collapsed = self.state.features.monitor.read(cx).processes_collapsed();
            crate::monitor::MonitorOps::new(&mut self.state).sample_processes(collapsed, cx);
        }
    }
}
