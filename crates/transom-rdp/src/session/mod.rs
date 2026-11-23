//! RDP 会话入口与外部生命周期控制。
//!
//! 设计原理（Why）：由于 IronRDP 内部的 `Framed` 与状态机使用了携带高阶生命周期
//! 的 GAT 绑定，在多线程 Tokio Runtime 上无法通过 rustc 的跨线程 `Send` 判定。
//! 因此采用专用独立操作系统线程 + current-thread runtime 的经典并发架构，
//! 对外只暴露纯消息通道与生命周期句柄，彻底解耦 UI 渲染层与协议栈生命周期。

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
    input_tx: mpsc::Sender<RdpInput>,
}

impl RdpSession {
    /// 异步拨号并启动独立的会话工作线程。
    pub fn connect(config: SessionConfig) -> (Self, mpsc::Receiver<RdpOutput>) {
        let (input_ch, output_ch) = channel::create_session_channels();
        let (input_tx, input_rx) = input_ch;
        let (output_tx, output_rx) = output_ch;

        thread::Builder::new()
            .name("transom-rdp-session".to_string())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        log::error!("创建 RDP 专用运行时失败: {e}");
                        return;
                    }
                };

                runtime.block_on(async move {
                    match connector::establish_connection(&config).await {
                        Ok(conn) => {
                            let runner = loop_runner::SessionLoopRunner::new(
                                conn.framed,
                                conn.result,
                                &config,
                                input_rx,
                                output_tx,
                            );
                            runner.run().await;
                        }
                        Err(e) => {
                            log::warn!("RDP 连接建立失败: {e}");
                            let _ = output_tx.send(RdpOutput::Terminated(e)).await;
                        }
                    }
                });
            })
            .expect("派生 RDP 会话线程必须成功");

        (Self { input_tx }, output_rx)
    }

    /// 发送输入指令。若通道已满或对端关闭，返回错误。
    pub async fn send_input(&self, input: RdpInput) -> Result<(), RdpError> {
        self.input_tx
            .send(input)
            .await
            .map_err(|_| RdpError::ChannelClosed {
                channel: "UI -> RDP Session",
            })
    }

    /// 尝试以非阻塞方式发送输入指令。
    pub fn try_send_input(&self, input: RdpInput) -> Result<(), RdpError> {
        self.input_tx
            .try_send(input)
            .map_err(|_| RdpError::ChannelClosed {
                channel: "UI -> RDP Session",
            })
    }

    /// 请求断开会话。
    pub fn disconnect(&self) {
        let _ = self.try_send_input(RdpInput::Disconnect);
    }
}
