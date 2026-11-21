//! 转发规则的检查类动作：连通性探测、复制 CLI 与浏览器打开。

use crate::app::{AppShell, AppState};
use gpui_kit::*;
use transom_core::{ForwardCommand, ForwardId, ForwardRule, TabId};

pub(crate) fn probe_forward_rule(
    app_state: &mut AppState,
    tab_id: TabId,
    rule_id: ForwardId,
    cx: &mut Context<AppShell>,
) {
    let Some(rule) = find_forward_rule(app_state, tab_id, rule_id) else {
        return;
    };
    let Some(connection_id) = app_state
        .workspace
        .tab(tab_id)
        .and_then(|t| t.connection_id())
    else {
        return;
    };
    app_state.send_command(ForwardCommand::ForwardProbe {
        tab_id,
        connection_id,
        rule_id,
        rule,
    });
    cx.notify();
}

/// 复制与本规则等价的 `ssh -N` 命令行到剪贴板，并给出状态条提示。
pub(crate) fn copy_forward_cli(
    app_state: &mut AppState,
    tab_id: TabId,
    rule_id: ForwardId,
    cx: &mut Context<AppShell>,
) {
    let Some(rule) = find_forward_rule(app_state, tab_id, rule_id) else {
        return;
    };
    let Some(session) = tab_session(app_state, tab_id) else {
        return;
    };
    let command = rule.ssh_command(&session);
    cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(command));
    app_state.set_transient_status(rust_i18n::t!("forward.cli_copied").to_string(), cx);
}

/// 在系统浏览器打开 Local 转发的本机入口；打开动作放后台执行器。
pub(crate) fn open_forward_browser(
    app_state: &mut AppState,
    tab_id: TabId,
    rule_id: ForwardId,
    cx: &mut Context<AppShell>,
) {
    let Some(rule) = find_forward_rule(app_state, tab_id, rule_id) else {
        return;
    };
    let Some(url) = rule.browser_url() else {
        return;
    };
    cx.spawn(async move |this, cx| {
        let opened = cx
            .background_executor()
            .spawn(async move { transom_platform::opener::open_with_default_app(url) });
        let result = opened.await;
        let _ = this.update(cx, |this, cx| {
            if let Err(error) = result {
                this.state
                    .set_error(rust_i18n::t!("forward.browser_failed", error = error).to_string());
            }
            cx.notify();
        });
    })
    .detach();
}

/// 从目录中找到指定标签会话的转发规则。
fn find_forward_rule(
    app_state: &AppState,
    tab_id: TabId,
    rule_id: ForwardId,
) -> Option<ForwardRule> {
    let session_id = app_state.workspace.tab(tab_id)?.session_id()?;
    app_state
        .session
        .catalog
        .sessions
        .iter()
        .find(|session| session.id == session_id)?
        .forwards
        .iter()
        .find(|rule| rule.id == rule_id)
        .cloned()
}

/// 标签对应会话的克隆快照。
fn tab_session(app_state: &AppState, tab_id: TabId) -> Option<transom_core::Session> {
    let session_id = app_state.workspace.tab(tab_id)?.session_id()?;
    app_state
        .session
        .catalog
        .sessions
        .iter()
        .find(|session| session.id == session_id)
        .cloned()
}
