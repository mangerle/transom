//! 智能运维 AI 助手 GPUI 面板模块。
//!
//! 设计原理（Why）：
//! - 独立实体 [`AiAssistantView`] 仅持有自身状态、全局设置读取与 `command_tx` 命令发送端，
//!   **严禁持有 `WeakEntity<AppShell>`**，严格遵守数据向下、意图向上的单向数据流；
//! - 按 `state` / `cards` / `approval` / `composer` / `config_drawer` / `view` 拆分为小文件。

mod acp_banner;
mod actions;
mod app_actions;
mod approval;
mod attachments;
mod bulk_matrix;
mod cards;
mod choice_card;
mod chrome;
mod chrome_header;
mod composer;
mod composer_triggers;
mod config_drawer;
mod config_drawer_general;
mod config_form;
mod context_popup;
mod diff_view;
mod empty_state;
mod handoff_banner;
mod handoff_render;
mod history_drawer;
pub(crate) mod inline_assist;
mod markdown_blocks;
mod memory_drawer;
mod mention_menu;
mod metrics;
mod metrics_model;
mod model_picker;
mod model_table;
pub(crate) mod panel_events;
mod plan_card;
mod policy_hooks_actions;
mod policy_hooks_drawer;
mod popups;
mod queue_card;
mod settings_actions;
mod skills_drawer;
mod skills_drawer_render;
mod slash_menu;
mod state;
mod state_turn;
mod state_types;
mod task_bar;
mod tool_card;
mod tool_card_body;
mod turn_nav;
mod view;
mod view_events;
mod work_group;
mod work_group_summary;

pub(crate) use state::ActiveAiSessionInfo;
pub(crate) use view::AiAssistantView;

use gpui_kit::{App, Window};
use transom_core::{AiAppEvent, AiCommand, AiTerminalContextKind, AppCommand};

use crate::app::AppState;

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
        let _ = state
            .runtime
            .services
            .command_tx
            .try_send(AppCommand::Ai(Box::new(
                AiCommand::RespondTerminalContext {
                    request_id: request_id.clone(),
                    content,
                },
            )));
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
        let _ = state
            .runtime
            .services
            .command_tx
            .try_send(AppCommand::Ai(Box::new(AiCommand::RespondAppAction {
                request_id: request_id.clone(),
                result,
            })));
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
}
