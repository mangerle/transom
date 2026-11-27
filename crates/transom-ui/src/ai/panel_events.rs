//! AI 面板向上发出的工作区级事件分发处理器。

use gpui_kit::base::resizable_panel;
use gpui_kit::*;
use transom_core::{AiCommand, AppCommand, TabId};

use super::AiPanelEvent;
use crate::divider::h_resizable;
use crate::theme::tokens::{rem_px, size};

/// 同步当前活动 SSH 会话列表并把主工作区与 AI 面板装配为水平可拖拽分栏。
pub(crate) fn wrap_with_ai_panel_split(
    state: &crate::app::AppState,
    resizable_group: AnyElement,
    rem: Pixels,
    window: &mut Window,
    cx: &mut Context<crate::app::AppShell>,
) -> AnyElement {
    let to_ai_info = |tab_id: TabId| -> Option<super::ActiveAiSessionInfo> {
        let tab = state.workspace.tab(tab_id)?;
        let session = tab.session()?;
        let transom_core::Connection::Ssh(ssh) = &session.connection else {
            return None;
        };
        Some(super::ActiveAiSessionInfo {
            tab_id,
            session_id: session.id,
            host_name: session.name.clone(),
            login: format!("{}@{}:{}", ssh.username, ssh.host, ssh.port),
            platform: "linux".to_owned(),
            protection: ssh.protection,
            connected: matches!(tab.status, crate::app::TabStatus::Connected),
        })
    };
    let active_ai_session = state.workspace.active_tab.and_then(to_ai_info);
    let connected_ssh_sessions: Vec<_> = state
        .workspace
        .tabs
        .iter()
        .filter_map(|t| to_ai_info(t.id))
        .filter(|info| info.connected)
        .collect();
    let mention_candidates = build_mention_candidates(state);
    state.features.ai_panel.update(cx, |panel, cx| {
        panel.set_sessions(active_ai_session, connected_ssh_sessions, window, cx);
        panel.set_mention_candidates(mention_candidates, cx);
    });
    let max_w = (window.viewport_size().width.as_f32() * 0.72).clamp(400.0, 960.0);
    let ai_split = h_resizable("workspace-ai-split")
        .with_state(&state.ui.ai_panel_split)
        .child(resizable_panel().child(resizable_group))
        .child(
            resizable_panel()
                .size(rem_px(size::AI_PANEL_WIDTH, rem))
                .size_range(px(400.0)..px(max_w))
                .fixed()
                .child(state.features.ai_panel.clone()),
        );
    div()
        .flex_1()
        .min_w_0()
        .h_full()
        .overflow_hidden()
        .child(ai_split)
        .into_any_element()
}

fn mention_token(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.split_whitespace().collect::<Vec<_>>().join("-"))
}

fn build_mention_candidates(state: &crate::app::AppState) -> Vec<super::state::AiMentionCandidate> {
    use std::collections::HashSet;

    use super::state::{AiMentionCandidate, AiMentionKind};

    let mut out = Vec::new();
    let mut seen_sessions = HashSet::new();
    for tab in &state.workspace.tabs {
        let Some(session) = tab.session() else {
            continue;
        };
        let transom_core::Connection::Ssh(ssh) = &session.connection else {
            continue;
        };
        if !matches!(tab.status, crate::app::TabStatus::Connected) {
            continue;
        }
        if let Some(token) = mention_token(&session.name)
            && seen_sessions.insert(session.id)
        {
            out.push(AiMentionCandidate {
                token,
                label: session.name.clone(),
                detail: format!("{}@{}:{}", ssh.username, ssh.host, ssh.port),
                kind: AiMentionKind::SshHost { connected: true },
            });
        }
    }
    for session in &state.session.catalog.sessions {
        let transom_core::Connection::Ssh(ssh) = &session.connection else {
            continue;
        };
        if seen_sessions.contains(&session.id) {
            continue;
        }
        if let Some(token) = mention_token(&session.name) {
            seen_sessions.insert(session.id);
            out.push(AiMentionCandidate {
                token,
                label: session.name.clone(),
                detail: format!("{}@{}:{}", ssh.username, ssh.host, ssh.port),
                kind: AiMentionKind::SshHost { connected: false },
            });
        }
    }
    for engine in &state.session.catalog.engines {
        if let Some(token) = mention_token(&engine.name) {
            out.push(AiMentionCandidate {
                token,
                label: engine.name.clone(),
                detail: rust_i18n::t!("ai.mention_docker_desc").to_string(),
                kind: AiMentionKind::ContainerEngine,
            });
        }
    }
    out.push(AiMentionCandidate {
        token: "local".to_owned(),
        label: "local".to_owned(),
        detail: rust_i18n::t!("ai.mention_local_desc").to_string(),
        kind: AiMentionKind::Local,
    });
    out
}

/// 处理 AI 面板向上发出的工作区级事件（关闭、保存片段、填终端、接管聚焦与回采）。
pub(crate) fn on_ai_panel_event(
    state: &mut crate::app::AppState,
    event: &AiPanelEvent,
    cx: &mut Context<crate::app::AppShell>,
) {
    match event {
        AiPanelEvent::CloseRequested => {
            state.features.ai_panel_open = false;
        }
        AiPanelEvent::SaveSnippetRequested { name, command } => {
            crate::snippets::add_quick_snippet(state, name.clone(), command.clone(), cx);
        }
        AiPanelEvent::SendToTerminalRequested { command } => {
            if let Some(view) = state
                .workspace
                .active_tab
                .and_then(|tid| state.workspace.tab(tid))
                .and_then(|tab| tab.terminal())
                && view.update(cx, |v, cx| v.send_text(command, cx))
            {
                state
                    .ui
                    .window_requests
                    .push_toast(rust_i18n::t!("ai.sent_to_terminal").to_string(), true);
            }
        }
        AiPanelEvent::JumpToTerminal { host_label } => {
            let target_tid = state
                .workspace
                .tabs
                .iter()
                .find(|t| {
                    t.terminal().is_some() && t.session().is_some_and(|s| s.name == *host_label)
                })
                .map(|t| t.id);
            if let Some(tid) = target_tid {
                state.workspace.active_tab = Some(tid);
                state.ui.window_requests.push_toast(
                    rust_i18n::t!("ai.jump_terminal_ok", host = host_label.as_str()).to_string(),
                    true,
                );
            } else {
                state.ui.window_requests.push_toast(
                    rust_i18n::t!("ai.jump_terminal_missing", host = host_label.as_str())
                        .to_string(),
                    false,
                );
            }
        }
        AiPanelEvent::FocusTerminalForHandoff { command } => {
            if let Some(view) = state
                .workspace
                .active_tab
                .and_then(|tid| state.workspace.tab(tid))
                .and_then(|tab| tab.terminal())
            {
                let cmd = command.trim().to_owned();
                if !cmd.is_empty() {
                    let _ = view.update(cx, |v, cx| v.send_text(&cmd, cx));
                }
                state
                    .ui
                    .window_requests
                    .push_toast(rust_i18n::t!("ai.handoff_toast").to_string(), true);
            }
        }
        AiPanelEvent::ResumeHandoffRequested { run_id, cancelled } => {
            let terminal_output = if *cancelled {
                None
            } else {
                state
                    .workspace
                    .active_tab
                    .and_then(|tid| state.workspace.tab(tid))
                    .and_then(|tab| tab.terminal())
                    .map(|term| {
                        let tv = term.read(cx);
                        let blocks = tv.recent_blocks_summary(2);
                        if blocks.trim().is_empty() {
                            tv.screen_tail_text(30)
                        } else {
                            blocks
                        }
                    })
            };
            let _ = state
                .runtime
                .services
                .command_tx
                .try_send(AppCommand::Ai(Box::new(AiCommand::ResumeHandoff {
                    run_id: run_id.clone(),
                    terminal_output,
                    cancelled: *cancelled,
                })));
        }
    }
}
