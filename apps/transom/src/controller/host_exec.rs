//! 主机批量动作执行：有界并发跑 TCP 拨测，逐条回报结果。
//!
//! 设计原理（Why）：
//! - 实现初衷：拨测的耗时来自"不可达主机要等超时"，串行跑 50 台最坏要分钟级；
//!   并发又会被"全选几百台"放大成 FD 与网络风暴，因此用一份跨批次共享的信号量
//!   把在途动作数钉死，无论界面一次派发多少条命令。
//! - 核心优势：并发上限放在控制器侧而不是界面侧，用户连点两次"拨测"也不会翻倍
//!   占用；每条动作恰好回报一次结果（连任务 panic 也补一条失败），界面不会
//!   因为漏报而永远转圈。
//! - 代价与局限：批次任务本身若异常退出，日志会记录但无法为整批补发结果——
//!   批次体只做取许可、发包、发事件三件事，没有可 panic 的业务分支。

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use transom_core::{AppError, AppEvent, ErrorCode, HostAction, HostOutcome, SessionId};
use transom_net::probe_tcp;

/// 跨批次共享的在途动作上限。
///
/// 16 路并发在局域网里足以让上百台主机的拨测在数秒内跑完；再高不会更快
/// （瓶颈是对端超时而非本机），却会成倍占用文件描述符。
pub(super) const MAX_CONCURRENT_ACTIONS: usize = 16;

/// 单次 TCP 握手超时：局域网正常是毫秒级，2 秒还没有 SYN-ACK 基本等价不可达。
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// 并发执行一批动作，逐条把结果送回事件通道。
pub(super) async fn run_batch(
    actions: Vec<HostAction>,
    event_tx: flume::Sender<AppEvent>,
    permits: Arc<Semaphore>,
) {
    // 未回报集合：任务 panic 或被取消时 `join_next` 给不回会话 id，
    // 批次排空后按这张表补齐失败结果，界面计数因此一定能归零。
    let mut pending: HashSet<SessionId> = actions.iter().map(HostAction::session_id).collect();
    let mut tasks = JoinSet::new();
    for action in actions {
        let action_permits = Arc::clone(&permits);
        tasks.spawn(async move {
            // 许可只在信号量被关闭时取不到，此时宁可少一层限流也要执行，
            // 保证每条动作都有回报。
            let _permit = action_permits.acquire_owned().await.ok();
            execute(action).await
        });
    }

    while let Some(joined) = tasks.join_next().await {
        match joined {
            Ok((session_id, outcome)) => {
                pending.remove(&session_id);
                report(&event_tx, session_id, outcome).await;
            }
            // 任务 panic/被取消：会话归属未知，交给下面的补齐逻辑结算。
            Err(error) => tracing::error!("主机动作任务异常退出，将在批次结束时补齐结果: {error}"),
        }
    }

    for session_id in pending {
        tracing::error!("主机动作未产生结果，会话 {session_id:?} 按失败结算");
        report(
            &event_tx,
            session_id,
            failed_outcome("动作未产生结果".to_string()),
        )
        .await;
    }
}

/// 把单条结果投到事件通道；通道满时反压等待，绝不丢弃。
async fn report(event_tx: &flume::Sender<AppEvent>, session_id: SessionId, outcome: HostOutcome) {
    let event = AppEvent::HostActionResult {
        session_id,
        outcome,
    };
    if let Err(error) = event_tx.send_async(event).await {
        tracing::warn!("主机动作结果未能送达界面，事件通道已关闭: {error}");
    }
}

/// 执行单条动作；网络错误一律结算成可展示的失败结果，不向上抛。
async fn execute(action: HostAction) -> (SessionId, HostOutcome) {
    // 用不可解构模式取字段：`HostAction` 目前只有拨测一个变体，写成 match 反而
    // 暗示还有别的分支。
    let HostAction::Probe {
        session_id,
        host,
        port,
    } = action;
    match probe_tcp(&host, port, PROBE_TIMEOUT).await {
        Ok(latency) => (
            session_id,
            HostOutcome::Reachable {
                // 饱和转换：外部耗时无上界，不能让它回绕成小数值。
                latency_ms: latency.as_millis().min(u64::MAX as u128) as u64,
            },
        ),
        Err(error) => (
            session_id,
            HostOutcome::Unreachable {
                error: AppError::from(error),
            },
        ),
    }
}

fn failed_outcome(reason: String) -> HostOutcome {
    HostOutcome::Unreachable {
        error: AppError::with_detail(ErrorCode::Io, reason),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `run_batch` 必须为每条动作发恰好一个结果事件：会话行的"拨测中"状态点
    /// 靠它落定，少发一次点就永远挂着，多发一次会把已经到期的点重新点亮。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn every_action_reports_exactly_once() {
        let open = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let open_port = open.local_addr().unwrap().port();
        let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let closed_port = closed.local_addr().unwrap().port();
        drop(closed);

        let actions = vec![
            HostAction::Probe {
                session_id: SessionId::new(),
                host: "127.0.0.1".to_string(),
                port: open_port,
            },
            HostAction::Probe {
                session_id: SessionId::new(),
                host: "127.0.0.1".to_string(),
                port: closed_port,
            },
        ];
        let (tx, rx) = flume::bounded::<AppEvent>(16);
        let permits = Arc::new(Semaphore::new(MAX_CONCURRENT_ACTIONS));
        run_batch(actions.clone(), tx, permits).await;

        // 完成顺序不保证与派发顺序一致，因此按结局种类计数而不是按下标断言。
        let mut reachable = 0;
        let mut unreachable = 0;
        let mut reported: Vec<SessionId> = Vec::new();
        while let Ok(event) = rx.try_recv() {
            match event {
                AppEvent::HostActionResult {
                    session_id,
                    outcome,
                } => {
                    reported.push(session_id);
                    match outcome {
                        HostOutcome::Reachable { .. } => reachable += 1,
                        HostOutcome::Unreachable { .. } => unreachable += 1,
                    }
                }
                // AppEvent 没有 Debug：只能按变体判定，不能打印现场。
                _ => panic!("只应收到主机动作结果事件"),
            }
        }
        assert_eq!(reachable, 1, "开放端口应判可达");
        assert_eq!(unreachable, 1, "已关闭端口应判不可达");
        assert_eq!(reported.len(), actions.len());
        // 每条动作归属唯一会话：结果串行化到行全靠这个 id。
        assert_eq!(
            reported
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            actions.len()
        );
        drop(open);
    }
}
