//! 应用事件的域处理方法：连接、认证与 SFTP/遥测等分组处理。
//!
//! 设计原理（Why）：`handle_app_event` 曾是 709 行的单体 match，每个分支
//! 内含数十行状态更新，阅读与定位都要跨过整个函数。分支体按域搬进本模块
//! 的具名方法后，顶层只剩"分发 + 是否重绘"的骨架；方法返回 `false` 表示
//! 事件已过期/内部自管重绘，调用方不再统一 `notify`。

use crate::app::{AppShell, AuthPromptState, HostKeyConfirmState, TabStatus};
use crate::palette::localized_error;
use gpui_kit::component::input::InputState;
use gpui_kit::*;
use std::sync::Arc;
use transom_core::{HostAppEvent, PromptKind, SecretKind, TerminalAppEvent};
use transom_terminal::{KeyFilter, TerminalEvent, TerminalView};

impl AppShell {
    /// 连接建立：构造终端视图、挂接订阅与外观覆盖，并处理焦点与启动命令。
    ///
    /// 返回 `false` 表示标签或连接已过期（迟到的完成事件），不触发重绘。
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
        let Some(index) = self.workspace.tabs.iter().position(|tab| tab.id == tab_id) else {
            return false;
        };
        if self.workspace.tabs[index].connection_id != Some(connection_id) {
            return false;
        }
        let session_id = self.workspace.tabs[index].session_id;
        let mut profile = self
            .catalog
            .sessions
            .iter()
            .find(|session| session.id == session_id)
            .map(|session| session.terminal.clone())
            .unwrap_or_else(|| self.workspace.tabs[index].session.terminal.clone());
        // 光标外观与外观偏好（字体族、中文字体、字号、行高、关键词高亮）属于全局设置，
        // 覆盖会话里可能残留的旧默认值：界面上没有任何会话级字体入口，
        // 若这里不覆盖，用户在设置里改的字体只会作用于已打开的窗格。
        profile.cursor_shape = self.settings.terminal_profile.cursor_shape;
        profile.cursor_blink = self.settings.terminal_profile.cursor_blink;
        profile.font_family = self.settings.terminal_profile.font_family.clone();
        profile.cjk_font_family = self.settings.terminal_profile.cjk_font_family.clone();
        profile.font_size = self.settings.terminal_profile.font_size;
        profile.line_height = self.settings.terminal_profile.line_height;
        // 分隔符同样没有会话级入口，且老会话文件里存的是内核默认值；
        // 不覆盖的话，用户在设置里改的分隔符会被会话里的陈旧值压掉。
        profile.word_separators = self.settings.terminal_profile.word_separators.clone();
        // 自定义按键映射属于全局终端偏好：会话文件里的旧副本不参与生效。
        profile.key_bindings = self.settings.terminal_profile.key_bindings.clone();
        profile.highlight = self.settings.terminal_profile.highlight.clone();
        profile.alternate_scroll = self.settings.terminal_profile.alternate_scroll;
        // 终端配色同理：会话文件里残留的旧调色板会压掉全局选择，
        // 表现为"改了终端配色但重开会话不生效"。运行期程序仍可通过
        // OSC 4/10/11 动态改色，不受此静态基线影响。
        profile.theme_name = self.settings.terminal_profile.theme_name.clone();
        profile.palette = self.settings.terminal_profile.palette.clone();
        // 终端背景透明度跟随窗口整体透明：窗口半透明时终端区也要透出
        // 桌面，否则只有界面变透明、终端仍是一块实色。
        profile.background_transparency = self.settings.window.window_transparency;
        let interaction = self.settings.terminal_interaction;
        let reserved = self.shortcut_recording.filter.clone();
        let key_filter: KeyFilter = Arc::new(move |keystroke| {
            reserved
                .read()
                .iter()
                .any(|binding| keystroke.should_match(binding))
        });
        // 审计落盘按连接粒度开启：路径在派发前算好，读线程内才创建文件。
        let audit_path =
            crate::audit::audit_path_for(&self.workspace.tabs[index], self.settings.audit.enabled);
        let terminal = cx.new(|cx| {
            TerminalView::new(
                window,
                cx,
                io,
                profile,
                interaction,
                Some(key_filter),
                audit_path,
            )
        });
        // 建议能力挂到视图：查询回调是全应用共享的一份内存快照。
        let provider = self.history_provider.clone();
        let triggers = self.settings.triggers.clone();
        let block_override = self
            .catalog
            .sessions
            .iter()
            .find(|session| session.id == session_id)
            .map_or(transom_core::BlockViewOverride::FollowGlobal, |session| {
                session.block_view
            });
        let block_view = self.settings.block_view.resolved(block_override);
        terminal.update(cx, |view, cx| {
            view.attach_history(provider);
            view.reload_triggers(&triggers);
            view.set_block_view(Some(block_view), cx);
        });
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
        let tab = &mut self.workspace.tabs[index];
        tab.terminal = Some(terminal);
        tab.status = TabStatus::Connected;
        log::info!(
            "连接建立完成，窗格: {tab_id}，连接: {connection_id:?}，会话: {}，目标: {}",
            tab.session.name,
            crate::connection::describe_target(&tab.session)
        );
        // 连接成功后自动重连计数清零，下次断线重新享有完整退避序列。
        tab.reconnect_attempts = 0;
        tab.network_traffic = None;
        tab._subscription = Some(subscription);
        // 连接建立期间用户可能已经切走：启动期的默认标签与深链目录标签
        // 会并发建连，晚完成者若无条件激活，会把焦点从后创建的深链标签
        // 抢回启动标签。只有该窗格仍是当前选中项时才切换组内焦点并聚焦。
        let still_active = self.workspace.active_tab == Some(tab_id);
        if still_active && let Some(group_index) = self.group_index_for_tab(tab_id) {
            self.workspace.groups[group_index].active_pane = tab_id;
        }
        // 认证已确定通过，此刻才允许把用户勾选保存的凭据写入钥匙串。
        self.flush_pending_credential(session_id, connection_id, cx);
        if still_active {
            window.focus(&focus_handle, cx);
        }
        self.schedule_startup_commands(tab_id, connection_id, window, cx);
        self.auto_start_forwards(tab_id, connection_id, session_id, cx);
        true
    }

    /// 连接建立失败：标记标签状态、回收未落盘凭据并安排自动重连。
    pub(crate) fn on_connection_failed(
        &mut self,
        event: TerminalAppEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        let TerminalAppEvent::ConnectionFailed {
            tab_id,
            connection_id,
            error,
        } = event
        else {
            return true;
        };
        if let Some(tab) = self.workspace.tabs.iter_mut().find(|tab| tab.id == tab_id)
            && tab.connection_id == Some(connection_id)
        {
            tab.status = TabStatus::Failed(error.clone());
            tab.network_traffic = None;
            let message = localized_error(&error);
            log::error!(
                "连接建立失败，窗格: {tab_id}，连接: {connection_id:?}，会话: {}，目标: {}，原因: {message}",
                tab.session.name,
                crate::connection::describe_target(&tab.session)
            );
            self.error = Some(message);
        }
        self.discard_pending_credential(connection_id);
        self.mark_forwards_stopped(tab_id);
        self.schedule_auto_reconnect(tab_id, connection_id, cx);
        true
    }

    /// 连接断开：仅在当前任连接生效时更新状态，并安排自动重连。
    pub(crate) fn on_disconnected(
        &mut self,
        event: TerminalAppEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        let TerminalAppEvent::Disconnected {
            tab_id,
            connection_id,
        } = event
        else {
            return true;
        };
        if let Some(tab) = self.workspace.tabs.iter_mut().find(|tab| tab.id == tab_id)
            && tab.connection_id == Some(connection_id)
            && !matches!(tab.status, TabStatus::Failed(_))
        {
            tab.status = TabStatus::Disconnected;
            tab.network_traffic = None;
            log::info!(
                "连接已断开，窗格: {tab_id}，连接: {connection_id:?}，会话: {}，目标: {}",
                tab.session.name,
                crate::connection::describe_target(&tab.session)
            );
        }
        self.mark_forwards_stopped(tab_id);
        self.schedule_auto_reconnect(tab_id, connection_id, cx);
        true
    }

    /// 认证询问：建立掩码输入框并登记"认证通过后才落盘"的保存意图。
    ///
    /// 返回 `false` 表示连接已过期（关闭标签/重连），不弹出提示。
    pub(crate) fn on_auth_prompt(
        &mut self,
        event: TerminalAppEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let TerminalAppEvent::AuthPrompt {
            tab_id,
            connection_id,
            name,
            instructions,
            prompts,
            responder,
        } = event
        else {
            return true;
        };
        let Some(tab) = self.workspace.tabs.iter().find(|tab| tab.id == tab_id) else {
            return false;
        };
        if tab.connection_id != Some(connection_id) {
            return false;
        }
        let session_id = tab.session_id;
        // 同一连接的新问题会作废上一次答案的保存意图：只有最后一次提交的
        // 凭据才可能认证成功，留着旧值会把已被拒绝的口令写进钥匙串。
        self.discard_pending_credential(connection_id);
        let inputs = prompts
            .iter()
            .map(|prompt| {
                let placeholder = prompt.prompt.clone();
                let masked = !prompt.echo;
                cx.new(|cx| {
                    let state = InputState::new(window, cx).placeholder(placeholder);
                    if masked { state.masked(true) } else { state }
                })
            })
            .collect();
        // 只有我们自己索取的提示才具备"保存"的意义；服务器下发的 keyboard-interactive
        // 文本可能是一次性挑战（OTP），存下来下次也用不上。
        let secret_kind = match prompts.first().map(|info| info.kind) {
            Some(PromptKind::Password) => Some(SecretKind::Password),
            Some(PromptKind::Passphrase) => Some(SecretKind::KeyPassphrase),
            _ => None,
        };
        self.modals.auth_prompt = Some(AuthPromptState {
            connection_id,
            name,
            instructions,
            prompts,
            inputs,
            responder,
            error: None,
            session_id,
            secret_kind,
            save: false,
        });
        true
    }

    /// 首连指纹确认：仅在连接仍现任时弹出核对窗口。
    ///
    /// 返回 `false` 表示连接已过期，不弹出提示。
    pub(crate) fn on_host_key_confirm(&mut self, event: TerminalAppEvent) -> bool {
        let TerminalAppEvent::HostKeyConfirm {
            tab_id,
            connection_id,
            host,
            port,
            key_type,
            fingerprint,
            responder,
        } = event
        else {
            return true;
        };
        let Some(tab) = self.workspace.tabs.iter().find(|tab| tab.id == tab_id) else {
            return false;
        };
        if tab.connection_id != Some(connection_id) {
            return false;
        }
        self.modals.host_key_confirm = Some(HostKeyConfirmState {
            host,
            port,
            key_type,
            fingerprint,
            responder,
        });
        true
    }

    /// 系统统计样本：只更新数据；可见标签（或打开着看板）才重绘。
    ///
    /// 返回 `false`：重绘由本方法按可见性与沉浸模式条件决定。
    pub(crate) fn on_system_stats(&mut self, event: HostAppEvent, cx: &mut Context<Self>) -> bool {
        let HostAppEvent::SystemStatsUpdated {
            tab_id,
            connection_id,
            stats,
        } = event
        else {
            return true;
        };
        // 看板打开时先把网卡快照推入波形历史，再移动 stats。
        if self.monitor.open && self.workspace.active_tab == Some(tab_id) {
            self.monitor_record_network(tab_id, &stats.interfaces);
        }
        let mut applied = false;
        if let Some(tab) = self.workspace.tabs.iter_mut().find(|tab| tab.id == tab_id)
            && tab.connection_id == Some(connection_id)
        {
            if tab.system_stats.as_ref() == Some(&stats) {
                // 与上一轮样本完全一致：不更新也不重绘。
                return false;
            }
            tab.system_stats = Some(stats);
            applied = true;
        }
        // 周期采样事件只在数据真的属于可见标签时才重绘底栏。
        if applied
            && self.workspace.active_tab == Some(tab_id)
            && (!self.immersive || self.monitor.open)
        {
            cx.notify();
        }
        false
    }

    /// 网络速率样本：只更新数据；可见标签才重绘。
    ///
    /// 返回 `false`：重绘由本方法按可见性与沉浸模式条件决定。
    pub(crate) fn on_network_traffic(
        &mut self,
        event: HostAppEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        let HostAppEvent::NetworkTrafficUpdated {
            tab_id,
            connection_id,
            traffic,
        } = event
        else {
            return true;
        };
        let mut applied = false;
        if let Some(tab) = self.workspace.tabs.iter_mut().find(|tab| tab.id == tab_id)
            && tab.connection_id == Some(connection_id)
        {
            if tab.network_traffic == Some(traffic) {
                // 速率与上一帧相同（空闲连接恒为 0/0）：数据无需更新，也不
                // 触发整壳重绘——否则空闲标签会以 1 Hz 持续整窗重建。
                return false;
            }
            tab.network_traffic = Some(traffic);
            applied = true;
        }
        if applied && self.workspace.active_tab == Some(tab_id) && !self.immersive {
            cx.notify();
        }
        false
    }
}
