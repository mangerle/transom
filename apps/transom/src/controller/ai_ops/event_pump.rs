//! 主 Agent 与子 Agent 共用的统一异步事件泵驱动器。
//!
//! 设计原理（Why）：
//! - 消除 `run.rs` 与 `subagent.rs` 中重复的 `tokio::select!` 泵送循环；
//! - 在 `runner_done` 时同时检查 `event_rx`、`card_rx` 与 `plan_rx` 是否已全部排空，
//!   并在循环结束后补排残余 `card_rx` 与 `plan_rx`，彻底避免末尾 `PlanUpdated` 或工具卡片丢失。

use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use transom_ai::{AgentEvent, AiError, PlanSnapshot, ToolExecutionCard};

/// 统一事件泵的消费回调契约。
#[async_trait]
pub(super) trait AgentPumpSink: Send {
    /// 定期节流刷盘回调（例如主 Agent 每 25ms 合并推送流式文本增量）。
    async fn on_tick(&mut self) {}

    /// 消费单张工具执行卡片。
    async fn on_card(&mut self, card: ToolExecutionCard);

    /// 消费单次计划更新快照。
    async fn on_plan(&mut self, _plan: PlanSnapshot) {}

    /// 消费来自运行时的 [`AgentEvent`]。
    async fn on_event(&mut self, event: AgentEvent);
}

/// 驱动 Agent 运行任务并在统一循环中泵送事件、工具卡片与计划更新，返回最终错误摘要（若失败）。
pub(super) async fn drive_agent_pump<S: AgentPumpSink>(
    mut runner_handle: Option<JoinHandle<Result<(), AiError>>>,
    mut event_rx: mpsc::Receiver<AgentEvent>,
    mut card_rx: mpsc::Receiver<ToolExecutionCard>,
    mut plan_rx: Option<mpsc::Receiver<PlanSnapshot>>,
    tick_interval: Option<Duration>,
    runner_panic_prefix: &str,
    sink: &mut S,
) -> Option<String> {
    let mut flush_tick = tick_interval.map(|d| {
        let mut interval = tokio::time::interval(d);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        interval
    });

    let mut runner_done = runner_handle.is_none();
    let mut runner_err: Option<String> = None;

    loop {
        tokio::select! {
            _ = async {
                if let Some(t) = flush_tick.as_mut() {
                    t.tick().await;
                } else {
                    std::future::pending::<()>().await;
                }
            }, if flush_tick.is_some() => {
                sink.on_tick().await;
            }
            maybe_card = card_rx.recv() => {
                if let Some(card) = maybe_card {
                    sink.on_card(card).await;
                }
            }
            maybe_plan = async {
                if let Some(rx) = plan_rx.as_mut() {
                    rx.recv().await
                } else {
                    std::future::pending().await
                }
            }, if plan_rx.is_some() => {
                if let Some(plan) = maybe_plan {
                    sink.on_plan(plan).await;
                } else {
                    plan_rx = None;
                }
            }
            maybe_evt = event_rx.recv() => {
                match maybe_evt {
                    Some(evt) => sink.on_event(evt).await,
                    None if runner_done => break,
                    None => {}
                }
            }
            res = async {
                if let Some(h) = runner_handle.as_mut() {
                    h.await
                } else {
                    std::future::pending().await
                }
            }, if !runner_done => {
                runner_done = true;
                runner_handle = None;
                match res {
                    Ok(Ok(())) => {}
                    Ok(Err(err)) => runner_err = Some(err.to_string()),
                    Err(join_err) => runner_err = Some(format!("{runner_panic_prefix}: {join_err}")),
                }
                let plan_empty = plan_rx.as_ref().is_none_or(mpsc::Receiver::is_empty);
                if event_rx.is_empty() && card_rx.is_empty() && plan_empty {
                    break;
                }
            }
        }
    }

    sink.on_tick().await;
    while let Ok(card) = card_rx.try_recv() {
        sink.on_card(card).await;
    }
    if let Some(rx) = plan_rx.as_mut() {
        while let Ok(plan) = rx.try_recv() {
            sink.on_plan(plan).await;
        }
    }

    runner_err
}
