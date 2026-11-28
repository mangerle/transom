//! 智能运维 AI 助手 GPUI 面板模块。
//!
//! 设计原理（Why）：
//! - 独立实体 [`AiAssistantView`] 仅持有自身状态、全局设置读取与 `command_tx` 命令发送端，
//!   **严禁持有 `WeakEntity<AppShell>`**，严格遵守数据向下、意图向上的单向数据流；
//! - 按 `state` / `cards` / `approval` / `composer` / `config_drawer` / `view` 拆分为小文件。

mod actions;
mod app_actions;
mod cards;
mod chrome;
mod chrome_header;
mod composer;
mod drawers;
mod empty_state;
mod handoff_banner;
mod handoff_render;
pub(crate) mod inline_assist;
mod markdown;
mod message_stream;
mod metrics;
mod metrics_model;
pub(crate) mod panel_events;
mod state;
mod state_types;
mod task_bar;
mod turn_nav;
mod view;
mod view_events;

use cards::{
    approval, bulk_matrix, choice_card, diff_view, plan_card, queue_card, tool_card,
    tool_card_body, work_group, work_group_summary,
};
use composer::{
    acp_banner, attachments, composer_triggers, context_popup, image, mention_menu, popups,
    slash_menu,
};
use drawers::{
    config_drawer, config_drawer_general, config_form, memory_drawer, model_picker, model_table,
    policy_hooks_drawer, skills_drawer,
};
use markdown::{
    blocks as markdown_blocks, code_card as markdown_code_card, parser as markdown_parser,
};

pub(crate) use state::ActiveAiSessionInfo;
pub(crate) use view::AiAssistantView;

use gpui_kit::{App, Window};
use transom_core::{AiAppEvent, AiCommand, AiTerminalContextKind, AppCommand};

use crate::app::AppState;

fn send_ai_response(state: &mut AppState, cx: &mut App, command: AiCommand, context: &str) {
    if let Err(err) = state
        .runtime
        .services
        .command_tx
        .try_send(AppCommand::Ai(Box::new(command)))
    {
        log::warn!("发送 AI 控制平面响应失败（{context}）: {err}");
        let message = rust_i18n::t!("ai.command_send_failed", error = err.to_string()).to_string();
        state
            .features
            .ai_panel
            .update(cx, |view, cx| view.report_command_error(message, cx));
    }
}

/// 分发控制平面推送的 AI 事件，并处理反向终端上下文采集请求。
pub(crate) fn dispatch_ai_event(
    state: &mut AppState,
    event: AiAppEvent,
    window: &mut Window,
    cx: &mut App,
) {
    if let AiAppEvent::TerminalContextRequested {
        request_id,
        tab_id,
        kind,
    } = &event
    {
        let content = state
            .workspace
            .tab(*tab_id)
            .and_then(|tab| tab.terminal())
            .map(|view| {
                let v = view.read(cx);
                match kind {
                    AiTerminalContextKind::RecentBlocks { limit } => {
                        v.recent_blocks_summary(*limit)
                    }
                    AiTerminalContextKind::ScreenTail { lines } => v.screen_tail_text(*lines),
                }
            });
        send_ai_response(
            state,
            cx,
            AiCommand::RespondTerminalContext {
                request_id: request_id.clone(),
                content,
            },
            "terminal_context",
        );
        return;
    }
    if let AiAppEvent::AppActionRequested {
        request_id,
        domain,
        action,
        args_json,
    } = &event
    {
        let result = app_actions::handle_app_action(state, domain, action, args_json, cx);
        send_ai_response(
            state,
            cx,
            AiCommand::RespondAppAction {
                request_id: request_id.clone(),
                result,
            },
            "app_action_result",
        );
        return;
    }
    if let AiAppEvent::InlineCommandResult {
        request_id,
        command,
        explanation,
        error,
    } = event
    {
        inline_assist::apply_inline_result(state, &request_id, command, explanation, error);
        return;
    }
    state
        .features
        .ai_panel
        .update(cx, |panel, cx| panel.handle_app_event(event, window, cx));
}

/// AI 面板向应用外壳发出的意图事件（事件自带全部所需数据，外壳禁止回读面板）。
#[derive(Debug, Clone)]
#[allow(clippy::enum_variant_names)]
pub(crate) enum AiPanelEvent {
    /// 请求收起右侧 AI 面板。
    CloseRequested,
    /// 请求将执行过的命令保存为快捷命令（Snippet）。
    SaveSnippetRequested { name: String, command: String },
    /// 请求将命令文本安全填入当前活动终端（不自动回车执行）。
    SendToTerminalRequested { command: String },
    /// 请求聚焦活动终端以便用户手动完成接管操作，并自动填入待执行命令。
    FocusTerminalForHandoff { command: String },
    /// 请求在采集活动终端最新输出后恢复或取消终端接管。
    ResumeHandoffRequested { run_id: String, cancelled: bool },
    /// 请求工作区切换到指定主机名关联的终端 Tab。
    JumpToTerminal { host_label: String },
    /// 请求连接指定已保存会话并在就绪后自动锁定为 AI 主目标。
    ConnectAndPinSession { session_id: transom_core::SessionId },
}
