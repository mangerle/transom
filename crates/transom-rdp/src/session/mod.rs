//! RDP 会话入口与外部生命周期控制。
//!
//! 设计原理（Why）：由于 IronRDP 内部的 `Framed` 与状态机使用了携带高阶生命周期
//! 的 GAT 绑定，在多线程 Tokio Runtime 上无法通过 rustc 的跨线程 `Send` 判定。
//! 因此采用专用独立操作系统线程 + current-thread runtime 的经典并发架构，
//! 对外只暴露纯消息通道与生命周期句柄，彻底解耦 UI 渲染层与协议栈生命周期。
//! 严禁生产路径裸 `expect`，线程派生失败时优雅降级并通知上层。

pub mod channel;
pub mod connector;
pub mod loop_runner;

use crate::error::RdpError;
use crate::settings::SessionConfig;
pub use channel::{MouseBtn, RdpInput, RdpOutput};
use std::thread;
use tokio::sync::mpsc;

/// RDP 桌面会话句柄。
pub struct RdpSession {
    pub(crate) input_tx: mpsc::Sender<RdpInput>,
}

impl RdpSession {
    /// 异步拨号并启动独立的会话工作线程。
    pub fn connect(mut config: SessionConfig) -> (Self, mpsc::Receiver<RdpOutput>) {
        let (input_ch, output_ch) = channel::create_session_channels();
        let (input_tx, input_rx) = input_ch;
        let (output_tx, output_rx) = output_ch;

        let output_tx_err = output_tx.clone();
        let host_for_err = config.host.clone();

        let spawn_result = thread::Builder::new()
            .name("transom-rdp-session".to_string())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        log::error!("创建 RDP 专用运行时失败: {e}");
                        let _ = output_tx.try_send(RdpOutput::Terminated(RdpError::Network {
                            address: config.host.clone(),
                            source: e,
                        }));
                        return;
                    }
                };

                runtime.block_on(async move {
                    match connector::establish_connection(&config, &output_tx).await {
                        Ok(conn) => {
                            // 握手连接建立成功后，即刻清空擦除内存中的密码字符串
                            config.clear_password();
                            let runner = loop_runner::SessionLoopRunner::new(
                                conn.framed,
                                conn.result,
                                conn.audio_muted,
                                conn.cliprdr_context,
                                input_rx,
                                output_tx,
                            );
                            runner.run().await;
                        }
                        Err(e) => {
                            config.clear_password();
                            log::warn!("RDP 连接建立失败: {e}");
                            let _ = output_tx.send(RdpOutput::Terminated(e)).await;
                        }
                    }
                });
            });

        if let Err(e) = spawn_result {
            log::error!("派生 RDP 独立会话线程失败: {e}");
            let _ = output_tx_err.try_send(RdpOutput::Terminated(RdpError::Network {
                address: host_for_err,
                source: std::io::Error::other(e.to_string()),
            }));
        }

        (Self { input_tx }, output_rx)
    }

    /// 发送输入指令。若通道已满则等待；对端关闭时返回错误。
    pub async fn send_input(&self, input: RdpInput) -> Result<(), RdpError> {
        self.input_tx
            .send(input)
            .await
            .map_err(|_| RdpError::ChannelClosed {
                channel: "UI -> RDP Session",
            })
    }

    /// 尝试以非阻塞方式发送输入指令。精确区分通道满与已关闭。
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

    /// 请求断开会话。
    pub fn disconnect(&self) {
        let _ = self.try_send_input(RdpInput::Disconnect);
    }
}

impl Drop for RdpSession {
    fn drop(&mut self) {
        // 会话句柄销毁时自动尝试派发断开指令，实现确定性 RAII 资源回收与平滑停机
        let _ = self.input_tx.try_send(RdpInput::Disconnect);
    }
}
