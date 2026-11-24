//! RDP 会话入口与外部生命周期控制。
//!
//! 设计原理（Why）：由于 IronRDP 内部的 `Framed` 与状态机使用了携带高阶生命周期
//! 的 GAT 绑定，在多线程 Tokio Runtime 上无法通过 rustc 的跨线程 `Send` 判定。
//! 因此采用专用独立操作系统线程 + current-thread runtime 的并发架构，对外只暴露
//! 有界消息通道与基于 `watch` 通道的确定性取消停机令牌，保证无论处于握手阶段还是
//! 主循环阶段，销毁句柄即可立即中断并平滑回收后台线程。

pub mod channel;
pub mod cliprdr;
pub mod connector;
pub mod loop_runner;
mod reactivate;
mod resize;

use crate::error::RdpError;
use crate::settings::SessionConfig;
pub use channel::{MouseBtn, RdpInput, RdpOutput, RdpReceivers};
use loop_runner::{SessionLoopParams, SessionLoopRunner};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::thread;
use tokio::sync::{mpsc, watch};

/// RDP 桌面会话句柄。
///
/// 设计原理（Why）：显式持有后台工作线程 `JoinHandle` 并在线程入口包裹 `catch_unwind`
/// 隔离 panic，杜绝脱缰孤儿线程；由于 `Drop` 可能发生在 UI 主线程，析构时只发送确定性
/// 停机信号而不阻塞等待网络回收。
pub struct RdpSession {
    pub(crate) input_tx: mpsc::Sender<RdpInput>,
    pub(crate) shutdown_tx: watch::Sender<bool>,
    pub(crate) worker: Option<thread::JoinHandle<()>>,
}

impl RdpSession {
    /// 异步拨号并启动独立的会话工作线程，返回会话句柄与双轨接收端（控制事件 + 增量帧）。
    pub fn connect(config: SessionConfig) -> (Self, RdpReceivers) {
        let channels = channel::create_session_channels();
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let events_tx_err = channels.senders.events.clone();

        let worker = match thread::Builder::new()
            .name("transom-rdp-session".to_string())
            .spawn(move || {
                run_worker_thread(config, channels.input_rx, channels.senders, shutdown_rx)
            }) {
            Ok(handle) => Some(handle),
            Err(e) => {
                log::error!("派生 RDP 独立会话线程失败: {e}");
                let _ = events_tx_err.try_send(RdpOutput::Terminated(RdpError::Runtime {
                    detail: format!("派生会话线程失败: {e}"),
                }));
                None
            }
        };

        (
            Self {
                input_tx: channels.input_tx,
                shutdown_tx,
                worker,
            },
            channels.receivers,
        )
    }

    /// 发送输入指令。若通道已满则等待；对端关闭时返回错误。
    ///
    /// # Errors
    /// 当后台会话已关闭时返回 [`RdpError::ChannelClosed`]。
    pub async fn send_input(&self, input: RdpInput) -> Result<(), RdpError> {
        self.input_tx
            .send(input)
            .await
            .map_err(|_| RdpError::ChannelClosed {
                channel: "UI -> RDP Session",
            })
    }

    /// 尝试以非阻塞方式发送输入指令，精确区分通道满与已关闭。
    ///
    /// # Errors
    /// 通道已满返回 [`RdpError::ChannelFull`]，已关闭返回 [`RdpError::ChannelClosed`]。
    pub fn try_send_input(&self, input: RdpInput) -> Result<(), RdpError> {
        self.input_tx.try_send(input).map_err(|err| match err {
            mpsc::error::TrySendError::Full(_) => RdpError::ChannelFull {
                channel: "UI -> RDP Session",
            },
            mpsc::error::TrySendError::Closed(_) => RdpError::ChannelClosed {
                channel: "UI -> RDP Session",
            },
        })
    }

    /// 克隆底层的有界输入通道发送端（用于无需借用视图实体的场景非阻塞投递输入）。
    pub fn input_sender(&self) -> mpsc::Sender<RdpInput> {
        self.input_tx.clone()
    }

    /// 请求断开会话并触发确定性停机信号（即使输入通道已满或正在握手中也能立即中止）。
    pub fn disconnect(&self) {
        let _ = self.shutdown_tx.send(true);
        let _ = self.try_send_input(RdpInput::Disconnect);
    }
}

impl Drop for RdpSession {
    fn drop(&mut self) {
        self.disconnect();
        // 后台线程若已自然结束则顺手回收句柄；若仍在清理网络套接字则避免阻塞 UI 线程
        if self
            .worker
            .as_ref()
            .is_some_and(thread::JoinHandle::is_finished)
            && let Some(handle) = self.worker.take()
        {
            let _ = handle.join();
        }
    }
}

/// 独立会话工作线程入口：构建单线程运行时并捕获意外 panic 通知上层 UI。
fn run_worker_thread(
    mut config: SessionConfig,
    input_rx: mpsc::Receiver<RdpInput>,
    senders: channel::SessionSenders,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let panic_events_tx = senders.events.clone();
    let outcome = catch_unwind(AssertUnwindSafe(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                log::error!("创建 RDP 专用运行时失败: {e}");
                let _ = senders
                    .events
                    .try_send(RdpOutput::Terminated(RdpError::Runtime {
                        detail: format!("创建异步运行时失败: {e}"),
                    }));
                return;
            }
        };

        runtime.block_on(async move {
            match connector::establish_connection(&config, &senders.events, &mut shutdown_rx).await
            {
                Ok(connection) => {
                    // 握手建立后立即主动擦除密码，不等待线程退出
                    config.clear_password();
                    let runner = SessionLoopRunner::new(SessionLoopParams {
                        connection,
                        input_rx,
                        senders,
                        shutdown_rx,
                    });
                    runner.run().await;
                }
                Err(e) => {
                    config.clear_password();
                    if !*shutdown_rx.borrow() {
                        log::warn!("RDP 连接建立失败: {e}");
                        let _ = senders.events.send(RdpOutput::Terminated(e)).await;
                    }
                }
            }
        });
    }));

    if outcome.is_err() {
        log::error!("RDP 会话工作线程发生未捕获 panic，已隔离并通知界面");
        let _ = panic_events_tx.try_send(RdpOutput::Terminated(RdpError::Runtime {
            detail: "RDP 会话后台线程异常中断".to_string(),
        }));
    }
}
