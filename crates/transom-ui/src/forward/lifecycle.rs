//! 端口转发规则的生命周期：落库、启停、公网确认、删除与就地更新。

use super::{PendingForwardConfirm, forward_bind_host, is_public_bind_address};
use crate::app::{AppShell, AppState};
use gpui_kit::*;
use transom_core::{ForwardCommand, ForwardId, ForwardRule, ForwardState, TabId};

/// 面板校验通过后提交规则：落库、必要时要求公网确认，否则直接下发启动命令。
pub(crate) fn submit_forward_rule(
    app_state: &mut AppState,
    tab_id: TabId,
    rule: ForwardRule,
    cx: &mut Context<AppShell>,
) {
    let Some(tab) = app_state.workspace.tab(tab_id) else {
        return;
    };
    let Some(connection_id) = tab.connection_id() else {
        return;
    };
    let Some(session_id) = tab.session_id() else {
        return;
    };

    if let Some(session) = app_state
        .session
        .catalog
        .sessions
        .iter_mut()
        .find(|session| session.id == session_id)
    {
        session.forwards.push(rule.clone());
    }
    app_state.persist_catalog();

    // 公网监听必须二次确认：不能因为点了一次「添加」就把端口开到公网。
    if is_public_bind_address(forward_bind_host(&rule)) {
        app_state.ui.modals.tools.confirm_forward_any = Some(PendingForwardConfirm {
            tab_id,
            connection_id,
            rule,
        });
        cx.notify();
        return;
    }

    app_state.set_forward_status(tab_id, rule.id, ForwardState::Starting, None);
    app_state.send_command(ForwardCommand::ForwardStart {
        tab_id,
        connection_id,
        rule,
    });
    cx.notify();
}

pub(crate) fn start_forward_rule(
    app_state: &mut AppState,
    tab_id: TabId,
    rule_id: ForwardId,
    cx: &mut Context<AppShell>,
) {
    let Some(tab) = app_state.workspace.tab(tab_id) else {
        return;
    };
    let Some(connection_id) = tab.connection_id() else {
        return;
    };
    let Some(session_id) = tab.session_id() else {
        return;
    };
    let Some(rule) = app_state
        .session
        .catalog
        .sessions
        .iter()
        .find(|session| session.id == session_id)
        .and_then(|session| session.forwards.iter().find(|rule| rule.id == rule_id))
        .cloned()
    else {
        return;
    };

    let bind_host = forward_bind_host(&rule);

    if is_public_bind_address(bind_host) {
        app_state.ui.modals.tools.confirm_forward_any = Some(PendingForwardConfirm {
            tab_id,
            connection_id,
            rule,
        });
        cx.notify();
        return;
    }

    if let Some(session) = app_state
        .session
        .catalog
        .sessions
        .iter_mut()
        .find(|session| session.id == session_id)
        && let Some(rule) = session.forwards.iter_mut().find(|rule| rule.id == rule_id)
    {
        rule.enabled = true;
    }
    app_state.persist_catalog();
    app_state.set_forward_status(tab_id, rule_id, ForwardState::Starting, None);
    app_state.send_command(ForwardCommand::ForwardStart {
        tab_id,
        connection_id,
        rule,
    });
    cx.notify();
}

pub(crate) fn confirm_forward_start(app_state: &mut AppState, cx: &mut Context<AppShell>) {
    let Some(pending) = app_state.ui.modals.tools.confirm_forward_any.take() else {
        return;
    };
    let PendingForwardConfirm {
        tab_id,
        connection_id,
        rule,
    } = pending;
    let session_id = app_state.workspace.tab(tab_id).and_then(|t| t.session_id());
    if let Some(session_id) = session_id {
        if let Some(session) = app_state
            .session
            .catalog
            .sessions
            .iter_mut()
            .find(|s| s.id == session_id)
            && let Some(r) = session.forwards.iter_mut().find(|r| r.id == rule.id)
        {
            r.enabled = true;
        }
        app_state.persist_catalog();
    }
    app_state.set_forward_status(tab_id, rule.id, ForwardState::Starting, None);
    app_state.send_command(ForwardCommand::ForwardStart {
        tab_id,
        connection_id,
        rule,
    });
    cx.notify();
}

pub(crate) fn cancel_forward_confirm(app_state: &mut AppState, cx: &mut Context<AppShell>) {
    app_state.ui.modals.tools.confirm_forward_any = None;
    cx.notify();
}

pub(crate) fn stop_forward_rule(
    app_state: &mut AppState,
    tab_id: TabId,
    rule_id: ForwardId,
    cx: &mut Context<AppShell>,
) {
    let Some(tab) = app_state.workspace.tab(tab_id) else {
        return;
    };
    let Some(connection_id) = tab.connection_id() else {
        return;
    };
    let Some(session_id) = tab.session_id() else {
        return;
    };
    if let Some(session) = app_state
        .session
        .catalog
        .sessions
        .iter_mut()
        .find(|session| session.id == session_id)
        && let Some(rule) = session.forwards.iter_mut().find(|rule| rule.id == rule_id)
    {
        rule.enabled = false;
    }
    app_state.persist_catalog();
    app_state.set_forward_status(tab_id, rule_id, ForwardState::Stopped, None);
    app_state.send_command(ForwardCommand::ForwardStop {
        tab_id,
        connection_id,
        rule_id,
    });
    cx.notify();
}

pub(crate) fn delete_forward_rule(
    app_state: &mut AppState,
    tab_id: TabId,
    rule_id: ForwardId,
    cx: &mut Context<AppShell>,
) {
    let Some(tab) = app_state.workspace.tab(tab_id) else {
        return;
    };
    let connection_id = tab.connection_id();
    let Some(session_id) = tab.session_id() else {
        return;
    };
    if let (Some(connection_id), Some(status)) = (
        connection_id,
        app_state
            .workspace
            .tab(tab_id)
            .and_then(|tab| tab.session_pane())
            .and_then(|pane| pane.forward_statuses.get(&rule_id)),
    ) && matches!(
        status.state,
        ForwardState::Active { .. } | ForwardState::Starting
    ) {
        app_state.send_command(ForwardCommand::ForwardStop {
            tab_id,
            connection_id,
            rule_id,
        });
    }
    if let Some(pane) = app_state
        .workspace
        .tab_mut(tab_id)
        .and_then(|tab| tab.session_pane_mut())
    {
        pane.forward_statuses.remove(&rule_id);
        pane.forward_probe_results.remove(&rule_id);
    }
    if let Some(session) = app_state
        .session
        .catalog
        .sessions
        .iter_mut()
        .find(|session| session.id == session_id)
    {
        session.forwards.retain(|rule| rule.id != rule_id);
    }
    app_state.persist_catalog();
    cx.notify();
}

/// 保存对既有规则的修改：保序替换并按意图重新拉起。
///
/// 运行中的规则不允许就地编辑（界面已禁用），因此这里无需先停旧隧道；
/// `enabled` 保持规则的既有意图——开启的规则替换后立即按新参数重启。
pub(crate) fn update_forward_rule(
    app_state: &mut AppState,
    tab_id: TabId,
    rule: ForwardRule,
    cx: &mut Context<AppShell>,
) {
    let Some(session_id) = app_state.workspace.tab(tab_id).and_then(|t| t.session_id()) else {
        return;
    };
    let mut restart = false;
    {
        if let Some(session) = app_state
            .session
            .catalog
            .sessions
            .iter_mut()
            .find(|session| session.id == session_id)
            && let Some(existing) = session.forwards.iter_mut().find(|r| r.id == rule.id)
        {
            // 记住替换前的启用意图：开启的规则替换后按新参数重新拉起。
            restart = existing.enabled;
            *existing = rule.clone();
        }
    }
    app_state.persist_catalog();
    if restart {
        start_forward_rule(app_state, tab_id, rule.id, cx);
        return;
    }
    app_state.set_forward_status(tab_id, rule.id, ForwardState::Stopped, None);
    cx.notify();
}
