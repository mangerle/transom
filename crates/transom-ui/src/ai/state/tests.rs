//! AI 面板状态与运行事件路由单元测试。

use super::AiPanelState;
use crate::ai::state_types::UiChatItem;
use transom_core::{AiAppEvent, AiApprovalDto, AiToolCardDto, HostProtection};

#[test]
fn tool_card_updates_in_place_by_call_id() {
    let mut st = AiPanelState {
        active_conversation_id: Some("c1".to_owned()),
        ..AiPanelState::default()
    };
    st.apply_app_event(AiAppEvent::ToolCard(AiToolCardDto {
        conversation_id: "c1".to_owned(),
        run_id: String::new(),
        call_id: "call-1".to_owned(),
        running: true,
        tool_name: "remote_exec".to_owned(),
        host_label: "web-01".to_owned(),
        risk_label: "只读".to_owned(),
        summary: "tail -n 5 /var/log/syslog".to_owned(),
        output: "line 1".to_owned(),
        ok: true,
        duration_ms: 300,
        diff_preview: None,
        revert_target_path: None,
        revert_backup_path: None,
    }));
    assert_eq!(st.items.len(), 1);
    assert_eq!(st.metrics.steps, 0);

    st.apply_app_event(AiAppEvent::ToolCard(AiToolCardDto {
        conversation_id: "c1".to_owned(),
        run_id: String::new(),
        call_id: "call-1".to_owned(),
        running: false,
        tool_name: "remote_exec".to_owned(),
        host_label: "web-01".to_owned(),
        risk_label: "只读".to_owned(),
        summary: "tail -n 5 /var/log/syslog".to_owned(),
        output: "line 1\nline 2".to_owned(),
        ok: true,
        duration_ms: 820,
        diff_preview: None,
        revert_target_path: None,
        revert_backup_path: None,
    }));
    assert_eq!(st.items.len(), 1);
    assert_eq!(st.metrics.steps, 1);
    assert_eq!(st.metrics.tool_duration_ms, 820);
    match &st.items[0] {
        UiChatItem::ToolCard {
            running,
            output,
            duration_ms,
            ..
        } => {
            assert!(!*running);
            assert_eq!(output, "line 1\nline 2");
            assert_eq!(*duration_ms, 820);
        }
        _ => panic!("期望为 ToolCard"),
    }
}

#[test]
fn run_started_resets_prior_view_state_with_real_user_entry() {
    use transom_core::AiPlanDto;

    let mut state = AiPanelState {
        active_conversation_id: Some("conv-1".to_owned()),
        active_plan: Some(AiPlanDto {
            title: Some("旧计划".to_owned()),
            steps: Vec::new(),
            focus: Some("旧焦点".to_owned()),
        }),
        active_focus: Some("旧焦点".to_owned()),
        status_panel_expanded: false,
        ..AiPanelState::default()
    };
    state.begin_run("run-1".to_owned(), Some("conv-1".to_owned()));
    state.items.push(UiChatItem::UserText {
        id: "local-u-run-1".to_owned(),
        text: "新问题".to_owned(),
        timestamp: None,
    });

    state.apply_app_event(AiAppEvent::RunStarted {
        run_id: "run-1".to_owned(),
        conversation_id: "conv-1".to_owned(),
        title: "新对话".to_owned(),
        user_entry_id: "entry-1".to_owned(),
    });

    assert!(state.active_plan.is_none());
    assert!(state.active_focus.is_none());
    assert!(state.status_panel_expanded);
    assert_eq!(state.metrics.turns, 1);
    assert!(matches!(
        &state.items[0],
        UiChatItem::UserText { id, .. } if id == "entry-1"
    ));
}

#[test]
fn stale_run_and_conversation_events_are_rejected() {
    let mut state = AiPanelState::default();
    state.begin_run("run-1".to_owned(), Some("conv-1".to_owned()));
    state.apply_app_event(AiAppEvent::RunStarted {
        run_id: "run-1".to_owned(),
        conversation_id: "conv-1".to_owned(),
        title: "对话".to_owned(),
        user_entry_id: "entry-1".to_owned(),
    });
    state.apply_app_event(AiAppEvent::TextDelta {
        run_id: "run-1".to_owned(),
        conversation_id: "conv-1".to_owned(),
        delta: "当前内容".to_owned(),
    });
    assert_eq!(state.items.len(), 1);

    state.apply_app_event(AiAppEvent::TextDelta {
        run_id: "run-2".to_owned(),
        conversation_id: "conv-1".to_owned(),
        delta: "旧运行内容".to_owned(),
    });
    state.apply_app_event(AiAppEvent::TextDelta {
        run_id: "run-1".to_owned(),
        conversation_id: "conv-2".to_owned(),
        delta: "其他对话内容".to_owned(),
    });
    assert_eq!(state.items.len(), 1);
    assert!(state.active_run.is_some());
}

#[test]
fn approval_routing_accepts_active_or_external_owner_only() {
    let mut state = AiPanelState::default();
    state.begin_run("run-1".to_owned(), Some("conv-1".to_owned()));
    state.active_conversation_id = Some("conv-1".to_owned());
    let approval = |owner: &str, run_id: &str| AiApprovalDto {
        request_id: "request-1".to_owned(),
        owner_run_id: owner.to_owned(),
        conversation_id: Some("conv-1".to_owned()),
        run_id: run_id.to_owned(),
        tool_name: "remote_exec".to_owned(),
        host_label: "host-1".to_owned(),
        protection: HostProtection::Normal,
        risk_label: "变更操作".to_owned(),
        is_destructive: false,
        title: "执行命令".to_owned(),
        detail: "uptime".to_owned(),
        diff_preview: None,
        requires_fresh_approval: false,
        suggested_prefix: None,
        impact_note: None,
    };
    assert!(state.accepts_event(&AiAppEvent::ApprovalRequested(approval("run-1", "run-1"))));
    assert!(
        !state.accepts_event(&AiAppEvent::ApprovalRequested(approval(
            "old-run", "old-run"
        )))
    );
    assert!(state.accepts_event(&AiAppEvent::ApprovalRequested(approval(
        "mcp-external",
        "mcp-external"
    ))));
}

#[test]
fn cancel_keeps_run_identity_until_finished() {
    let mut state = AiPanelState::default();
    state.begin_run("run-1".to_owned(), Some("conv-1".to_owned()));
    state.active_conversation_id = Some("conv-1".to_owned());
    assert_eq!(state.request_current_run_cancel().as_deref(), Some("run-1"));
    assert_eq!(
        state.active_run.as_ref().map(|run| run.cancel_requested),
        Some(true)
    );
    assert!(state.finish_run("run-1"));
    assert!(state.active_run.is_none());
}

#[test]
fn background_finish_does_not_clear_current_conversation_view() {
    let mut state = AiPanelState::default();
    state.begin_run("run-1".to_owned(), Some("conv-1".to_owned()));
    state.apply_app_event(AiAppEvent::RunStarted {
        run_id: "run-1".to_owned(),
        conversation_id: "conv-1".to_owned(),
        title: "后台对话".to_owned(),
        user_entry_id: "entry-1".to_owned(),
    });
    state.active_conversation_id = Some("conv-2".to_owned());
    state.active_focus = Some("当前对话焦点".to_owned());

    state.apply_app_event(AiAppEvent::RunFinished {
        run_id: "run-1".to_owned(),
        conversation_id: "conv-1".to_owned(),
        error: Some("后台错误".to_owned()),
        cancelled: false,
    });

    assert!(state.active_run.is_none());
    assert_eq!(state.active_focus.as_deref(), Some("当前对话焦点"));
    assert!(state.status_error.is_none());
    assert_eq!(
        state
            .background_run_errors
            .get("conv-1")
            .map(String::as_str),
        Some("后台错误")
    );
}
