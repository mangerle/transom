//! AI 运行期 UI 事件发送与增量合并。

use transom_core::{AiAppEvent, AppEvent};

use super::PumpContext;

pub(super) async fn flush_deltas(
    ctx: &PumpContext,
    text_buf: &mut String,
    reasoning_buf: &mut String,
) {
    if !reasoning_buf.is_empty() {
        let delta = std::mem::take(reasoning_buf);
        emit_ui_event(
            ctx,
            AiAppEvent::ReasoningDelta {
                run_id: ctx.run_id.clone(),
                conversation_id: ctx.conversation_id.clone(),
                delta,
            },
            "发送 AI 推理增量事件",
        )
        .await;
    }
    if !text_buf.is_empty() {
        let delta = std::mem::take(text_buf);
        emit_ui_event(
            ctx,
            AiAppEvent::TextDelta {
                run_id: ctx.run_id.clone(),
                conversation_id: ctx.conversation_id.clone(),
                delta,
            },
            "发送 AI 文本增量事件",
        )
        .await;
    }
}

pub(super) async fn emit_ui_event(ctx: &PumpContext, event: AiAppEvent, operation: &str) {
    if let Err(err) = ctx.event_tx.send_async(AppEvent::Ai(Box::new(event))).await {
        log::warn!("{operation}失败: {err}");
    }
}
