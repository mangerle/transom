//! 会话 Deactivation-Reactivation 重激活序列驱动。
//!
//! 设计原理（Why）：当服务端触发动态分辨率变更、会话切换或锁屏时会发送
//! `DeactivateAll` PDU，要求客户端执行标准 [MS-RDPBCGR] Deactivation-Reactivation 序列。
//! 将重激活流程独立为专职模块，避免主事件循环 `loop_runner.rs` 膨胀超限。

use crate::error::RdpError;
use crate::session::channel::RdpOutput;
use ironrdp_connector::connection_activation::{
    ConnectionActivationFactory, ConnectionActivationState,
};
use ironrdp_connector::{Sequence, Written};
use ironrdp_core::WriteBuf;
use ironrdp_session::ActiveStage;
use ironrdp_tokio::{Framed, FramedRead, FramedWrite, single_sequence_step_read};
use std::time::Duration;
use tokio::sync::mpsc;

/// 重激活序列超时上限（秒）。
const REACTIVATION_TIMEOUT_SECS: u64 = 15;

/// 成功重激活后的新桌面分辨率 `(width, height)`。
pub(crate) async fn execute_reactivation<R, W>(
    activation_factory: &ConnectionActivationFactory,
    active_stage: &mut ActiveStage,
    reader: &mut Framed<R>,
    writer: &mut Framed<W>,
    events_tx: &mpsc::Sender<RdpOutput>,
) -> Option<(u16, u16)>
where
    R: FramedRead,
    W: FramedWrite,
{
    log::info!("收到服务端 DeactivateAll PDU，开始执行重激活序列");
    let mut activation = activation_factory.create();
    let mut buf = WriteBuf::new();

    let step_result = tokio::time::timeout(Duration::from_secs(REACTIVATION_TIMEOUT_SECS), async {
        while !activation.state().is_terminal() {
            let written = single_sequence_step_read(reader, &mut activation, &mut buf).await?;
            if let Written::Size(size) = written {
                writer
                    .write_all(&buf[..size.get()])
                    .await
                    .map_err(|e| ironrdp_connector::custom_err!("写入重激活帧失败", e))?;
            }
        }
        Ok::<(), ironrdp_connector::ConnectorError>(())
    })
    .await;

    let err_msg = match step_result {
        Ok(Ok(())) => None,
        Ok(Err(e)) => Some(format!("重激活序列执行失败: {e}")),
        Err(_) => Some("重激活序列等待超时".to_string()),
    };
    if let Some(detail) = err_msg {
        log::warn!("RDP {detail}");
        let _ = events_tx
            .send(RdpOutput::Terminated(RdpError::Protocol {
                stage: "Deactivation-Reactivation 重激活",
                detail,
            }))
            .await;
        return None;
    }

    let ConnectionActivationState::Finalized {
        desktop_size,
        share_id,
        static_channel_chunk_size,
        enable_server_pointer,
        pointer_software_rendering,
        ..
    } = activation.connection_activation_state()
    else {
        log::warn!("RDP 重激活序列结束但未进入 Finalized 状态");
        return None;
    };

    let reactivated = active_stage.reactivate(
        activation.io_channel_id(),
        activation.user_channel_id(),
        share_id,
        enable_server_pointer,
        pointer_software_rendering,
        static_channel_chunk_size,
    );
    if !reactivated {
        log::warn!("RDP ActiveStage 重激活失败（静态虚拟通道分片尺寸非法）");
        return None;
    }

    log::info!(
        "RDP 会话重激活完成，新分辨率: {}x{}",
        desktop_size.width,
        desktop_size.height
    );
    Some((desktop_size.width, desktop_size.height))
}
