//! 主 Agent 与子 Agent 共用的统一有序事件泵。
//!
//! 设计原理（Why）：
//! - 文本增量、工具卡片、计划与运行收尾共用一条有界事件流，跨类别顺序由发送端等待背压保证；
//! - 25ms 节流仅合并高频文本，不改变最终工具记录与轮次终态的先后关系；
//! - runner 完成后再等待事件通道关闭，确保已成功发送的事件全部消费完毕。

use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use transom_ai::{AgentEvent, AiError};

/// 统一事件泵的消费回调契约。
#[async_trait]
pub(super) trait AgentPumpSink: Send {
    /// 定期节流刷盘回调（例如主 Agent 每 25ms 合并推送流式文本增量）。
    async fn on_tick(&mut self) {}

    /// 消费来自运行时的有序事件。
    async fn on_event(&mut self, event: AgentEvent);
}

/// 驱动 Agent 运行任务并消费有序事件，返回最终错误摘要（若失败）。
pub(super) async fn drive_agent_pump<S: AgentPumpSink>(
    mut runner_handle: Option<JoinHandle<Result<(), AiError>>>,
    mut event_rx: mpsc::Receiver<AgentEvent>,
    tick_interval: Option<Duration>,
    runner_panic_prefix: &str,
    sink: &mut S,
) -> Option<String> {
    let mut flush_tick = tick_interval.map(|duration| {
        let mut interval = tokio::time::interval(duration);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        interval
    });

    let mut runner_done = runner_handle.is_none();
    let mut events_closed = false;
    let mut runner_err: Option<String> = None;

    loop {
        if runner_done && events_closed {
            break;
        }
        tokio::select! {
            _ = async {
                if let Some(tick) = flush_tick.as_mut() {
                    tick.tick().await;
                } else {
                    std::future::pending::<()>().await;
                }
            }, if flush_tick.is_some() => {
                sink.on_tick().await;
            }
            maybe_event = event_rx.recv(), if !events_closed => {
                match maybe_event {
                    Some(event) => sink.on_event(event).await,
                    None => events_closed = true,
                }
            }
            result = async {
                if let Some(handle) = runner_handle.as_mut() {
                    handle.await
                } else {
                    std::future::pending().await
                }
            }, if !runner_done => {
                runner_done = true;
                runner_handle = None;
                match result {
                    Ok(Ok(())) => {}
                    Ok(Err(err)) => runner_err = Some(err.to_string()),
                    Err(join_err) => runner_err = Some(format!("{runner_panic_prefix}: {join_err}")),
                }
            }
        }
    }

    sink.on_tick().await;
    runner_err
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use tokio::sync::mpsc;
    use transom_ai::{AgentEvent, ToolExecutionCard, TurnOutput};

    use super::{AgentPumpSink, drive_agent_pump};

    struct OrderSink {
        kinds: Vec<&'static str>,
    }

    #[async_trait]
    impl AgentPumpSink for OrderSink {
        async fn on_event(&mut self, event: AgentEvent) {
            self.kinds.push(match event {
                AgentEvent::ToolCard(_) => "tool",
                AgentEvent::TurnEnd(_) => "turn",
                _ => "other",
            });
        }
    }

    #[tokio::test]
    async fn preserves_tool_card_before_turn_end_on_one_stream() {
        let (tx, rx) = mpsc::channel(8);
        tx.send(AgentEvent::ToolCard(ToolExecutionCard::default()))
            .await
            .expect("发送工具卡片失败");
        tx.send(AgentEvent::TurnEnd(TurnOutput::default()))
            .await
            .expect("发送轮次终态失败");
        drop(tx);

        let mut sink = OrderSink { kinds: Vec::new() };
        let error = drive_agent_pump(None, rx, None, "runner", &mut sink).await;
        assert!(error.is_none());
        assert_eq!(sink.kinds, ["tool", "turn"]);
    }
}
