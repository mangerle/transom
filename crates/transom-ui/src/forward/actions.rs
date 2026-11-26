//! 端口转发规则管理与启停控制方法。

use super::{ForwardKindChoice, ForwardPanelState, forward_bind_host, is_public_bind_address};
use crate::app::{AppShell, AppState, TabStatus};
use crate::keymap;
use crate::palette::sftp_text_input;
use gpui_kit::component::input::InputEvent;
use gpui_kit::*;
use transom_core::{
    AppError, Connection, ConnectionId, ErrorCode, ForwardCommand, ForwardId, ForwardRule,
    ForwardState, ForwardStatus, SessionId, TabId,
};

impl AppState {
    /// 记录断线前仍处于活跃或启动中的转发规则，供重连成功后恢复。
    pub(crate) fn snapshot_active_forwards(&mut self, tab_id: TabId) {
        if let Some(pane) = self
            .workspace
            .tab_mut(tab_id)
            .and_then(|tab| tab.session_pane_mut())
        {
            for (rule_id, status) in &pane.forward_statuses {
                if matches!(
                    status.state,
                    ForwardState::Active { .. } | ForwardState::Starting
                ) && !pane.active_forwards_snapshot.contains(rule_id)
                {
                    pane.active_forwards_snapshot.push(*rule_id);
                }
            }
        }
    }

    pub(crate) fn mark_forwards_stopped(&mut self, tab_id: TabId) {
        self.snapshot_active_forwards(tab_id);
        if let Some(pane) = self
            .workspace
            .tab_mut(tab_id)
            .and_then(|tab| tab.session_pane_mut())
        {
            for status in pane.forward_statuses.values_mut() {
                if !matches!(status.state, ForwardState::Stopped) {
                    status.state = ForwardState::Stopped;
                    status.error = None;
                }
            }
        }
    }

    pub(crate) fn set_forward_status(
        &mut self,
        tab_id: TabId,
        rule_id: ForwardId,
        state: ForwardState,
        error: Option<AppError>,
    ) {
        if let Some(pane) = self
            .workspace
            .tab_mut(tab_id)
            .and_then(|tab| tab.session_pane_mut())
        {
            pane.forward_statuses.insert(
                rule_id,
                ForwardStatus {
                    rule_id,
                    state,
                    error,
                    meter: None,
                },
            );
        }
    }
}

/// 处理转发面板发来的意图。
///
/// 只用事件携带的数据，不回读面板：事件派发阶段面板仍被租借。
fn on_forward_panel_event(
    app_state: &mut AppState,
    event: &crate::forward::ForwardPanelEvent,
    cx: &mut Context<AppShell>,
) {
    use crate::forward::ForwardPanelEvent;
    match event {
        ForwardPanelEvent::SubmitRule { tab_id, rule } => {
            crate::forward::submit_forward_rule(app_state, *tab_id, rule.clone(), cx);
        }
        ForwardPanelEvent::StartRule { tab_id, rule_id } => {
            crate::forward::start_forward_rule(app_state, *tab_id, *rule_id, cx);
        }
        ForwardPanelEvent::StopRule { tab_id, rule_id } => {
            crate::forward::stop_forward_rule(app_state, *tab_id, *rule_id, cx);
        }
        ForwardPanelEvent::DeleteRule { tab_id, rule_id } => {
            crate::forward::delete_forward_rule(app_state, *tab_id, *rule_id, cx);
        }
        ForwardPanelEvent::UpdateRule { tab_id, rule } => {
            crate::forward::update_forward_rule(app_state, *tab_id, rule.clone(), cx);
        }
        ForwardPanelEvent::ProbeRule { tab_id, rule_id } => {
            crate::forward::probe_forward_rule(app_state, *tab_id, *rule_id, cx);
        }
        ForwardPanelEvent::CopyCli { tab_id, rule_id } => {
            crate::forward::copy_forward_cli(app_state, *tab_id, *rule_id, cx);
        }
        ForwardPanelEvent::OpenBrowser { tab_id, rule_id } => {
            crate::forward::open_forward_browser(app_state, *tab_id, *rule_id, cx);
        }
        ForwardPanelEvent::ClosePanel => {
            app_state.workspace.close_forward_panel();
            cx.notify();
        }
    }
}

pub(crate) fn auto_start_forwards(
    app_state: &mut AppState,
    tab_id: TabId,
    connection_id: ConnectionId,
    session_id: SessionId,
    cx: &mut Context<AppShell>,
) {
    let rules: Vec<ForwardRule> = app_state
        .session
        .catalog
        .sessions
        .iter()
        .find(|session| session.id == session_id)
        .map(|session| session.forwards.clone())
        .unwrap_or_default();
    let active_snapshot = app_state
        .workspace
        .tab_mut(tab_id)
        .and_then(|tab| tab.session_pane_mut())
        .map(|pane| std::mem::take(&mut pane.active_forwards_snapshot))
        .unwrap_or_default();
    for rule in rules
        .into_iter()
        .filter(|rule| rule.enabled || active_snapshot.contains(&rule.id))
    {
        // 自动启动不能成为“导入会话后静默开放公网端口”的通道：
        // 公网 bind 一律留待用户在转发面板手动启动并二次确认。
        if is_public_bind_address(forward_bind_host(&rule)) {
            app_state.set_forward_status(
                tab_id,
                rule.id,
                ForwardState::NeedsConfirmation,
                Some(AppError::with_detail(
                    ErrorCode::ForwardFailed,
                    "公网监听不会自动启动；请在转发面板手动启动并确认",
                )),
            );
            continue;
        }
        app_state.set_forward_status(tab_id, rule.id, ForwardState::Starting, None);
        app_state.send_command(ForwardCommand::ForwardStart {
            tab_id,
            connection_id,
            rule,
        });
    }
    cx.notify();
}

pub(crate) fn toggle_forward_panel(
    app_state: &mut AppState,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let Some(tab_id) = app_state.workspace.active_tab else {
        return;
    };
    let Some(index) = app_state.workspace.tab_index(tab_id) else {
        return;
    };
    if app_state.workspace.tabs[index].forwards_view().is_some() {
        if let Some(pane) = app_state.workspace.tabs[index].session_pane_mut() {
            pane.forwards_view = None;
        }
        cx.notify();
        return;
    }
    if !matches!(
        app_state.workspace.tabs[index]
            .session()
            .map(|session| &session.connection),
        Some(Connection::Ssh(_))
    ) {
        app_state.set_error(rust_i18n::t!("error.forward_requires_ssh").to_string());
        cx.notify();
        return;
    }
    if !matches!(app_state.workspace.tabs[index].status, TabStatus::Connected) {
        app_state.set_error(rust_i18n::t!("error.forward_failed").to_string());
        cx.notify();
        return;
    }
    let (name, bind_host, bind_port, target_host, target_port) = forward_form_inputs(window, cx);
    let view = cx.new(|_| {
        super::ForwardView::new(
            ForwardPanelState {
                name_input: name.clone(),
                bind_host_input: bind_host.clone(),
                bind_port_input: bind_port.clone(),
                target_host_input: target_host.clone(),
                target_port_input: target_port.clone(),
                kind: ForwardKindChoice::Local,
                error: None,
                editing_id: None,
                _subscriptions: Vec::new(),
            },
            tab_id,
        )
    });
    // 回车提交：名称与两个端口输入框触发表单校验与落库。
    let mut enter_subscriptions = Vec::new();
    for input in [
        ("forward-enter-name", name),
        ("forward-enter-bind-port", bind_port),
        ("forward-enter-target-port", target_port),
    ] {
        let (_id, input) = input;
        enter_subscriptions.push(cx.subscribe_in(
            &input,
            window,
            move |this, _, event: &InputEvent, _window, cx| {
                if !matches!(event, InputEvent::PressEnter { .. }) {
                    return;
                }
                if let Some(view) = this
                    .state
                    .workspace
                    .tab(tab_id)
                    .and_then(|tab| tab.forwards_view())
                {
                    view.update(cx, |panel, cx| panel.submit_pressed(cx));
                }
            },
        ));
    }
    let subscription = cx.subscribe(
        &view,
        |app_state, _view, event: &crate::forward::ForwardPanelEvent, cx| {
            on_forward_panel_event(&mut app_state.state, event, cx);
        },
    );
    app_state
        .runtime
        .tasks
        .subscriptions
        .forward
        .insert(tab_id, subscription);
    if let Some(pane) = app_state.workspace.tabs[index].session_pane_mut() {
        view.update(cx, |panel, _| {
            panel.state._subscriptions.extend(enter_subscriptions);
        });
        pane.forwards_view = Some(view);
    }
    cx.notify();
}

pub(crate) fn on_toggle_forward(
    app_state: &mut AppState,
    _: &keymap::ToggleForward,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    crate::forward::toggle_forward_panel(app_state, window, cx);
}

/// 转发表单的五个文本输入：名称、绑定地址/端口与目标地址/端口。
/// 转发表单五个输入框的类型别名。
type ForwardFormInputs = (
    gpui_kit::Entity<gpui_kit::component::input::InputState>,
    gpui_kit::Entity<gpui_kit::component::input::InputState>,
    gpui_kit::Entity<gpui_kit::component::input::InputState>,
    gpui_kit::Entity<gpui_kit::component::input::InputState>,
    gpui_kit::Entity<gpui_kit::component::input::InputState>,
);

fn forward_form_inputs(
    window: &mut Window,
    cx: &mut Context<crate::app::AppShell>,
) -> ForwardFormInputs {
    let name = sftp_text_input(window, cx, "", rust_i18n::t!("forward.name").as_ref());
    let bind_host = sftp_text_input(
        window,
        cx,
        "127.0.0.1",
        rust_i18n::t!("forward.bind_host").as_ref(),
    );
    let bind_port = sftp_text_input(
        window,
        cx,
        "8080",
        rust_i18n::t!("forward.bind_port").as_ref(),
    );
    let target_host = sftp_text_input(
        window,
        cx,
        "127.0.0.1",
        rust_i18n::t!("forward.target_host").as_ref(),
    );
    let target_port = sftp_text_input(
        window,
        cx,
        "80",
        rust_i18n::t!("forward.target_port").as_ref(),
    );
    (name, bind_host, bind_port, target_host, target_port)
}
