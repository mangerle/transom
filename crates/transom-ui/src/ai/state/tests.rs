//! AI 面板状态与运行事件路由单元测试。

use super::AiPanelState;
use crate::ai::state_types::UiChatItem;
use transom_core::{AiAppEvent, AiApprovalDto, AiToolCardDto, HostProtection};

#[test]
fn tool_card_updates_in_place_by_call_id() {
    let mut st = AiPanelState::default();
    st.conversation.active_conversation_id = Some("c1".to_owned());
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
    assert_eq!(st.conversation.items.len(), 1);
    assert_eq!(st.conversation.metrics.steps, 0);

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
    assert_eq!(st.conversation.items.len(), 1);
    assert_eq!(st.conversation.metrics.steps, 1);
    assert_eq!(st.conversation.metrics.tool_duration_ms, 820);
    match &st.conversation.items[0] {
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

    let mut state = AiPanelState::default();
    state.conversation.active_conversation_id = Some("conv-1".to_owned());
    state.run.active_plan = Some(AiPlanDto {
        title: Some("旧计划".to_owned()),
        steps: Vec::new(),
        focus: Some("旧焦点".to_owned()),
    });
    state.run.active_focus = Some("旧焦点".to_owned());
    state.view.status_panel_expanded = false;
    state.begin_run("run-1".to_owned(), Some("conv-1".to_owned()));
    state.conversation.items.push(UiChatItem::UserText {
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

    assert!(state.run.active_plan.is_none());
    assert!(state.run.active_focus.is_none());
    assert!(state.view.status_panel_expanded);
    assert_eq!(state.conversation.metrics.turns, 1);
    assert!(matches!(
        &state.conversation.items[0],
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
    assert_eq!(state.conversation.items.len(), 1);

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
    assert_eq!(state.conversation.items.len(), 1);
    assert!(state.run.active_run.is_some());
}

#[test]
fn approval_routing_accepts_active_or_external_owner_only() {
    let mut state = AiPanelState::default();
    state.begin_run("run-1".to_owned(), Some("conv-1".to_owned()));
    state.conversation.active_conversation_id = Some("conv-1".to_owned());
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
    state.conversation.active_conversation_id = Some("conv-1".to_owned());
    assert_eq!(state.request_current_run_cancel().as_deref(), Some("run-1"));
    assert_eq!(
        state
            .run
            .active_run
            .as_ref()
            .map(|run| run.cancel_requested),
        Some(true)
    );
    assert!(state.finish_run("run-1"));
    assert!(state.run.active_run.is_none());
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
    state.conversation.active_conversation_id = Some("conv-2".to_owned());
    state.run.active_focus = Some("当前对话焦点".to_owned());

    state.apply_app_event(AiAppEvent::RunFinished {
        run_id: "run-1".to_owned(),
        conversation_id: "conv-1".to_owned(),
        error: Some("后台错误".to_owned()),
        cancelled: false,
    });

    assert!(state.run.active_run.is_none());
    assert_eq!(state.run.active_focus.as_deref(), Some("当前对话焦点"));
    assert!(state.view.status_error.is_none());
    assert_eq!(
        state
            .run
            .background_run_errors
            .get("conv-1")
            .map(String::as_str),
        Some("后台错误")
    );
}

#[test]
fn stale_settings_save_result_does_not_clear_current_request() {
    let mut state = AiPanelState::default();
    state.config.pending_settings_save = Some("new-save".to_owned());

    state.apply_app_event(AiAppEvent::SettingsSaved {
        request_id: "old-save".to_owned(),
        revision: 1,
        error: Some("旧错误".to_owned()),
    });
    assert_eq!(
        state.config.pending_settings_save.as_deref(),
        Some("new-save")
    );
    assert!(state.view.status_error.is_none());

    state.apply_app_event(AiAppEvent::SettingsSaved {
        request_id: "new-save".to_owned(),
        revision: 2,
        error: Some("磁盘不可写".to_owned()),
    });
    assert!(state.config.pending_settings_save.is_none());
    assert!(
        state
            .view
            .status_error
            .as_deref()
            .is_some_and(|error| error.contains("磁盘不可写"))
    );
    assert!(
        rust_i18n::t!(
            "ai.save_config_failed",
            locale = "zh-CN",
            error = "磁盘不可写"
        )
        .contains("保存 AI 配置失败")
    );
}

#[test]
fn settings_snapshot_replaces_runtime_defaults() {
    let settings = transom_ai::AiSettings {
        default_model: "provider:model-1".to_owned(),
        execution_mode: transom_ai::ExecutionMode::AllowAll,
        ..transom_ai::AiSettings::default()
    };
    let mut state = AiPanelState::default();
    state.config.pending_settings_load = Some("load-1".to_owned());

    let outcome = state.apply_app_event(AiAppEvent::SettingsSnapshot {
        request_id: "load-1".to_owned(),
        settings_json: Some(serde_json::to_string(&settings).expect("序列化设置失败")),
        revision: 3,
        error: None,
    });

    assert_eq!(outcome.settings_snapshot, Some(settings.clone()));
    assert!(state.config.pending_settings_load.is_none());
    assert_eq!(state.config.settings, settings);
    assert_eq!(state.config.selected_model_id, "provider:model-1");
    assert_eq!(
        state.config.execution_mode,
        transom_ai::ExecutionMode::AllowAll
    );
    assert!(state.view.status_error.is_none());
}

#[test]
fn settings_load_failure_applies_default_snapshot_and_reports_error() {
    let settings = transom_ai::AiSettings::default();
    let mut state = AiPanelState::default();
    state.config.pending_settings_load = Some("load-1".to_owned());

    let outcome = state.apply_app_event(AiAppEvent::SettingsSnapshot {
        request_id: "load-1".to_owned(),
        settings_json: Some(serde_json::to_string(&settings).expect("序列化设置失败")),
        revision: 0,
        error: Some("config.json 内容损坏".to_owned()),
    });

    assert_eq!(outcome.settings_snapshot, Some(settings));
    assert!(state.config.pending_settings_load.is_none());
    assert!(
        state
            .view
            .status_error
            .as_deref()
            .is_some_and(|error| error.contains("config.json 内容损坏"))
    );
    assert!(
        rust_i18n::t!(
            "ai.load_config_failed",
            locale = "zh-CN",
            error = "config.json 内容损坏"
        )
        .contains("加载 AI 配置失败")
    );
}

#[test]
fn stale_settings_snapshot_is_ignored() {
    let mut state = AiPanelState::default();
    state.config.pending_settings_load = Some("new-load".to_owned());

    let outcome = state.apply_app_event(AiAppEvent::SettingsSnapshot {
        request_id: "old-load".to_owned(),
        settings_json: Some("{}".to_owned()),
        revision: 1,
        error: Some("旧加载错误".to_owned()),
    });

    assert!(outcome.settings_snapshot.is_none());
    assert_eq!(
        state.config.pending_settings_load.as_deref(),
        Some("new-load")
    );
    assert!(state.view.status_error.is_none());
}
