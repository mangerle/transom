//! 应用事件的域处理方法：连接、认证与 SFTP/遥测等分组处理。
//!
//! 设计原理（Why）：`handle_app_event` 曾是 709 行的单体 match，每个分支
//! 内含数十行状态更新，阅读与定位都要跨过整个函数。分支体按域搬进本模块
//! 的具名方法后，顶层只剩"分发 + 是否重绘"的骨架；方法返回 `false` 表示
//! 事件已过期/内部自管重绘，调用方不再统一 `notify`。

use crate::app::app_settings;
use crate::app::{AppShell, TabStatus};
use crate::connection::{AuthPromptState, HostKeyConfirmState};
use crate::palette::localized_error;
use gpui_kit::component::input::InputState;
use gpui_kit::*;
use transom_core::{HostAppEvent, PromptKind, SecretKind, TerminalAppEvent};
use transom_terminal::TerminalView;

impl AppShell {
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
        if let Some(tab) = self.state.workspace.tab_mut(tab_id)
            && tab.connection_id == Some(connection_id)
        {
            tab.status = TabStatus::Failed(error.clone());
            if let Some(pane) = tab.session_pane_mut() {
                pane.network_traffic = None;
            }
            let message = localized_error(&error);
            log::error!(
                "连接建立失败，窗格: {tab_id}，连接: {connection_id:?}，会话: {}，目标: {}，原因: {message}",
                tab.session.name,
                crate::connection::describe_target(&tab.session)
            );
            self.state.set_error(message);
        }
        self.state.discard_pending_credential(connection_id);
        self.state.mark_forwards_stopped(tab_id);
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
        if let Some(tab) = self.state.workspace.tab_mut(tab_id)
            && tab.connection_id == Some(connection_id)
            && !matches!(tab.status, TabStatus::Failed(_))
        {
            tab.status = TabStatus::Disconnected;
            if let Some(pane) = tab.session_pane_mut() {
                pane.network_traffic = None;
            }
            log::info!(
                "连接已断开，窗格: {tab_id}，连接: {connection_id:?}，会话: {}，目标: {}",
                tab.session.name,
                crate::connection::describe_target(&tab.session)
            );
        }
        self.state.mark_forwards_stopped(tab_id);
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
        let Some(tab) = self.state.workspace.tab(tab_id) else {
            return false;
        };
        if tab.connection_id != Some(connection_id) {
            return false;
        }
        let session_id = tab.session_id;
        // 同一连接的新问题会作废上一次答案的保存意图：只有最后一次提交的
        // 凭据才可能认证成功，留着旧值会把已被拒绝的口令写进钥匙串。
        self.state.discard_pending_credential(connection_id);
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
        self.state.modals.connection.auth_prompt = Some(AuthPromptState {
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
        let Some(tab) = self.state.workspace.tab(tab_id) else {
            return false;
        };
        if tab.connection_id != Some(connection_id) {
            return false;
        }
        self.state.modals.connection.host_key_confirm = Some(HostKeyConfirmState {
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
        // 看板打开时先把统计快照推入时序历史，再移动 stats。
        if self.state.monitor.read(cx).is_open() && self.state.workspace.active_tab == Some(tab_id)
        {
            let open = true;
            if open {
                self.state
                    .monitor
                    .update(cx, |view, _| view.record_stats(tab_id, &stats));
            }
        }

        let mut applied = false;
        if let Some(tab) = self.state.workspace.tab_mut(tab_id)
            && tab.connection_id == Some(connection_id)
            && let Some(pane) = tab.session_pane_mut()
        {
            if pane.system_stats.as_ref() == Some(&stats) {
                // 与上一轮样本完全一致：不更新也不重绘。
                return false;
            }
            pane.system_stats = Some(*stats);
            applied = true;
        }
        // 周期采样事件只在数据真的属于可见标签时才重绘底栏。
        if applied && self.state.workspace.active_tab == Some(tab_id) {
            self.refresh_status_metrics(cx);
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
        if let Some(tab) = self.state.workspace.tab_mut(tab_id)
            && tab.connection_id == Some(connection_id)
            && let Some(pane) = tab.session_pane_mut()
        {
            if pane.network_traffic == Some(traffic) {
                // 速率与上一帧相同（空闲连接恒为 0/0）：数据无需更新，也不
                // 触发整壳重绘——否则空闲标签会以 1 Hz 持续整窗重建。
                return false;
            }
            pane.network_traffic = Some(traffic);
            applied = true;
        }
        if applied && self.state.workspace.active_tab == Some(tab_id) {
            self.refresh_status_metrics(cx);
        }
        false
    }

    /// 按当前全局设置组装并创建终端视图。
    pub(super) fn spawn_terminal_view(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        spawn: TerminalSpawn,
    ) -> Entity<TerminalView> {
        let TerminalSpawn {
            io,
            profile,
            interaction,
            key_filter,
            audit_path,
            shell_integration_enabled,
            session_id,
        } = spawn;
        let terminal = cx.new(|cx| {
            TerminalView::new(
                window,
                cx,
                transom_terminal::TerminalViewParams {
                    io,
                    profile,
                    interaction,
                    key_filter: Some(key_filter),
                    audit_path,
                    shell_integration_enabled,
                },
            )
        });
        let provider = self.state.history_provider.clone();
        let triggers = app_settings(cx).triggers.clone();
        let block_override = self
            .state
            .catalog
            .sessions
            .iter()
            .find(|session| session.id == session_id)
            .map_or(transom_core::BlockViewOverride::FollowGlobal, |session| {
                session.block_view
            });
        let block_view = app_settings(cx).block_view.resolved(block_override);
        terminal.update(cx, |view, cx| {
            view.attach_history(provider);
            view.reload_triggers(&triggers);
            view.set_block_view(Some(block_view), cx);
        });
        terminal
    }
}

/// 终端创建参数：连接 IO、已合并的配置与审计上下文。
pub(super) struct TerminalSpawn {
    pub(super) io: transom_core::TerminalIo,
    pub(super) profile: transom_core::TerminalProfile,
    pub(super) interaction: transom_core::TerminalInteraction,
    pub(super) key_filter: transom_terminal::KeyFilter,
    pub(super) audit_path: Option<std::path::PathBuf>,
    pub(super) shell_integration_enabled: bool,
    pub(super) session_id: transom_core::SessionId,
}

/// 用全局终端外观覆盖会话里可能残留的旧默认值。
pub(super) fn apply_global_terminal_profile(profile: &mut transom_core::TerminalProfile, cx: &App) {
    // 光标外观与外观偏好（字体族、中文字体、字号、行高、关键词高亮）属于全局设置，
    // 覆盖会话里可能残留的旧默认值：界面上没有任何会话级字体入口，
    // 若这里不覆盖，用户在设置里改的字体只会作用于已打开的窗格。
    profile.cursor_shape = app_settings(cx).terminal_profile.cursor_shape;
    profile.cursor_blink = app_settings(cx).terminal_profile.cursor_blink;
    profile.font_family = app_settings(cx).terminal_profile.font_family.clone();
    profile.cjk_font_family = app_settings(cx).terminal_profile.cjk_font_family.clone();
    profile.font_size = app_settings(cx).terminal_profile.font_size;
    profile.line_height = app_settings(cx).terminal_profile.line_height;
    // 分隔符同样没有会话级入口，且老会话文件里存的是内核默认值；
    // 不覆盖的话，用户在设置里改的分隔符会被会话里的陈旧值压掉。
    profile.word_separators = app_settings(cx).terminal_profile.word_separators.clone();
    // 自定义按键映射属于全局终端偏好：会话文件里的旧副本不参与生效。
    profile.key_bindings = app_settings(cx).terminal_profile.key_bindings.clone();
    profile.highlight = app_settings(cx).terminal_profile.highlight.clone();
    profile.alternate_scroll = app_settings(cx).terminal_profile.alternate_scroll;
    // 终端配色同理：会话文件里残留的旧调色板会压掉全局选择，
    // 表现为"改了终端配色但重开会话不生效"。运行期程序仍可通过
    // OSC 4/10/11 动态改色，不受此静态基线影响。
    profile.theme_name = app_settings(cx).terminal_profile.theme_name.clone();
    profile.palette = app_settings(cx).terminal_profile.palette.clone();
    // 终端背景透明度跟随窗口整体透明：窗口半透明时终端区也要透出
    // 桌面，否则只有界面变透明、终端仍是一块实色。
    profile.background_transparency = app_settings(cx).window.window_transparency;
}
