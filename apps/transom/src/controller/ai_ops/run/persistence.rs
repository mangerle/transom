//! AI 轮次与工具记录的有序持久化转换。

use transom_ai::{
    AiAuditEntry, ChatRole, HookContext, HookEvent, SessionEntry, SessionEntryKind,
    ToolExecutionCard, TurnUsageRecord, compute_context_breakdown,
};
use transom_core::{AiAppEvent, AiToolCardDto, AiTurnMetricsDto, AppEvent};

use super::PumpContext;

pub(super) async fn handle_turn_end(
    ctx: &mut PumpContext,
    last_leaf_id: &mut Option<String>,
    turn: transom_ai::TurnOutput,
) -> Result<(), transom_ai::AiError> {
    let mut to_append = Vec::with_capacity(2);
    if !turn.reasoning.trim().is_empty() {
        let entry = SessionEntry::new(
            last_leaf_id.clone(),
            SessionEntryKind::Reasoning {
                text: turn.reasoning.clone(),
            },
        );
        *last_leaf_id = Some(entry.id.clone());
        to_append.push(entry.clone());
        ctx.history_entries.push(entry);
    }
    if !turn.text.trim().is_empty() {
        let entry = SessionEntry::new(
            last_leaf_id.clone(),
            SessionEntryKind::Message {
                role: ChatRole::Assistant,
                text: turn.text.clone(),
                images: Vec::new(),
                model: Some(ctx.model_name.clone()),
                tokens_per_sec: turn.tokens_per_sec,
                usage: Some(TurnUsageRecord {
                    input_tokens: turn.input_tokens,
                    cached_input_tokens: turn.cached_input_tokens,
                    output_tokens: turn.output_tokens,
                    model_duration_ms: turn.model_duration_ms,
                    ttft_ms: turn.ttft_ms,
                }),
            },
        );
        *last_leaf_id = Some(entry.id.clone());
        to_append.push(entry.clone());
        ctx.history_entries.push(entry);
    }
    let added = turn.input_tokens.saturating_add(turn.output_tokens);
    let (today_tokens, month_tokens) = ctx.recorder.append_turn(to_append, added).await?;
    let refs: Vec<&SessionEntry> = ctx.history_entries.iter().collect();
    let breakdown =
        compute_context_breakdown(&ctx.preamble, &ctx.memory, &refs, ctx.context_window);
    let metrics = AiTurnMetricsDto {
        input_tokens: turn.input_tokens,
        cached_input_tokens: turn.cached_input_tokens,
        output_tokens: turn.output_tokens,
        model_duration_ms: turn.model_duration_ms,
        ttft_ms: turn.ttft_ms,
        today_tokens,
        month_tokens,
        context_breakdown: Some(breakdown),
    };
    if let Err(err) = ctx
        .event_tx
        .send_async(AppEvent::Ai(Box::new(AiAppEvent::TurnEnd {
            run_id: ctx.run_id.clone(),
            conversation_id: ctx.conversation_id.clone(),
            reasoning: turn.reasoning,
            text: turn.text,
            tokens_per_sec: turn.tokens_per_sec,
            metrics,
        })))
        .await
    {
        log::warn!("发送 AI 轮次结束事件失败: {err}");
    }
    Ok(())
}

pub(super) async fn handle_tool_card(
    ctx: &mut PumpContext,
    last_leaf_id: &mut Option<String>,
    card: ToolExecutionCard,
) -> Result<(), transom_ai::AiError> {
    if card.running {
        let dto = AiToolCardDto {
            conversation_id: ctx.conversation_id.clone(),
            run_id: ctx.run_id.clone(),
            call_id: card.call_id,
            running: true,
            tool_name: card.tool_name,
            host_label: card.host_label,
            risk_label: card.risk.label().to_owned(),
            summary: card.summary,
            output: card.output,
            ok: card.ok,
            duration_ms: card.duration_ms,
            diff_preview: card.diff_preview,
            revert_target_path: card.revert_target_path,
            revert_backup_path: card.revert_backup_path,
        };
        if let Err(err) = ctx
            .event_tx
            .send_async(AppEvent::Ai(Box::new(AiAppEvent::ToolCard(dto))))
            .await
        {
            log::warn!("发送 AI 工具运行态卡片失败: {err}");
        }
        return Ok(());
    }

    let entry = SessionEntry::new(
        last_leaf_id.clone(),
        SessionEntryKind::ToolRecord {
            tool_name: card.tool_name.clone(),
            summary: card.summary.clone(),
            output: card.output.clone(),
            ok: card.ok,
            duration_ms: card.duration_ms,
            approved: Some(card.approved),
            diff_preview: card.diff_preview.clone(),
            revert_target_path: card.revert_target_path.clone(),
            revert_backup_path: card.revert_backup_path.clone(),
        },
    );
    *last_leaf_id = Some(entry.id.clone());
    let audit = AiAuditEntry {
        timestamp: chrono::Utc::now().to_rfc3339(),
        conversation_id: ctx.conversation_id.clone(),
        host_label: ctx.host_label.clone(),
        tool_name: card.tool_name.clone(),
        risk: card.risk,
        summary: card.summary.clone(),
        approved: card.approved,
        ok: card.ok,
        duration_ms: card.duration_ms,
    };
    ctx.history_entries.push(entry.clone());
    ctx.recorder.append_tool(entry, audit).await?;

    let card_tool_name = card.tool_name.clone();
    let card_summary_for_hook = card.summary.clone();
    let card_host_for_hook = card.host_label.clone();
    let card_ok = card.ok;
    let card_summary_text = format!("{} ({})", card.tool_name, card.summary);

    let dto = AiToolCardDto {
        conversation_id: ctx.conversation_id.clone(),
        run_id: ctx.run_id.clone(),
        call_id: card.call_id,
        running: false,
        tool_name: card.tool_name,
        host_label: card.host_label,
        risk_label: card.risk.label().to_owned(),
        summary: card.summary,
        output: card.output,
        ok: card.ok,
        duration_ms: card.duration_ms,
        diff_preview: card.diff_preview,
        revert_target_path: card.revert_target_path,
        revert_backup_path: card.revert_backup_path,
    };
    if let Err(err) = ctx
        .event_tx
        .send_async(AppEvent::Ai(Box::new(AiAppEvent::ToolCard(dto))))
        .await
    {
        log::warn!("发送 AI 工具完成态卡片失败: {err}");
    }

    if let Some(exec) = &ctx.hook_executor {
        let hook_ctx = HookContext {
            tool_name: Some(card_tool_name),
            args_json: Some(card_summary_for_hook),
            target_label: card_host_for_hook,
            tool_ok: Some(card_ok),
            summary: card_summary_text,
        };
        let outcome = exec.execute(HookEvent::PostToolUse, &hook_ctx).await;
        for note in outcome.notes {
            if let Err(err) = ctx
                .event_tx
                .send_async(AppEvent::Ai(Box::new(AiAppEvent::FocusUpdated {
                    run_id: ctx.run_id.clone(),
                    focus: format!("Hook: {note}"),
                })))
                .await
            {
                log::warn!("发送 AI Hook 焦点事件失败: {err}");
            }
        }
    }
    Ok(())
}
