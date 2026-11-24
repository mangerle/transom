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
use tokio::sync::{mpsc, watch};

/// 重激活序列超时上限（秒）。
const REACTIVATION_TIMEOUT_SECS: u64 = 15;

/// 成功重激活后的新桌面分辨率 `(width, height)`；若失败或被停机取消则返回 `None`。
///
/// 设计原理（Why）：
/// 1. 监听 `shutdown_rx`，确保用户在重激活卡顿期间关闭标签能立即取消而无需干等 15 秒；
/// 2. 除主动停机外，所有失败分支一律向 UI 发送 `RdpOutput::Terminated`，杜绝界面永久黑屏僵死。
pub(crate) async fn execute_reactivation<R, W>(
    activation_factory: &ConnectionActivationFactory,
    active_stage: &mut ActiveStage,
    reader: &mut Framed<R>,
    writer: &mut Framed<W>,
    events_tx: &mpsc::Sender<RdpOutput>,
    shutdown_rx: &mut watch::Receiver<bool>,
) -> Option<(u16, u16)>
where
    R: FramedRead,
    W: FramedWrite,
{
    if *shutdown_rx.borrow() {
        return None;
    }

    log::info!("收到服务端 DeactivateAll PDU，开始执行重激活序列");
    let mut activation = activation_factory.create();
    let mut buf = WriteBuf::new();

    let timed_steps = tokio::time::timeout(Duration::from_secs(REACTIVATION_TIMEOUT_SECS), async {
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
    });

    let step_result = tokio::select! {
        _ = shutdown_rx.changed() => {
            log::info!("RDP 重激活序列收到停机信号，中止重激活");
            return None;
        }
        res = timed_steps => res,
    };

    let err_msg = match step_result {
        Ok(Ok(())) => None,
        Ok(Err(e)) => Some(format!("重激活序列执行失败: {e}")),
        Err(_) => Some("重激活序列等待超时".to_string()),
    };
    if let Some(detail) = err_msg {
        notify_reactivation_failure(events_tx, detail).await;
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
        notify_reactivation_failure(
            events_tx,
            "重激活序列结束但未进入 Finalized 状态".to_string(),
        )
        .await;
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
        notify_reactivation_failure(
            events_tx,
            "ActiveStage 重激活失败（静态虚拟通道分片尺寸非法）".to_string(),
        )
        .await;
        return None;
    }

    log::info!(
        "RDP 会话重激活完成，新分辨率: {}x{}",
        desktop_size.width,
        desktop_size.height
    );
    Some((desktop_size.width, desktop_size.height))
}

async fn notify_reactivation_failure(events_tx: &mpsc::Sender<RdpOutput>, detail: String) {
    log::warn!("RDP {detail}");
    let _ = events_tx
        .send(RdpOutput::Terminated(RdpError::Protocol {
            stage: "Deactivation-Reactivation 重激活",
            detail,
        }))
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironrdp_connector::DesktopSize;
    use std::io;
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

    struct EofStream;

    impl AsyncRead for EofStream {
        fn poll_read(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            _buf: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    impl AsyncWrite for EofStream {
        fn poll_write(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<io::Result<usize>> {
            Poll::Ready(Ok(buf.len()))
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    fn build_test_stage() -> (ConnectionActivationFactory, ActiveStage) {
        let mut config = ironrdp_connector::Config {
            credentials: ironrdp_connector::Credentials::UsernamePassword {
                username: "u".to_string(),
                password: "p".to_string(),
            },
            domain: None,
            enable_tls: false,
            enable_credssp: false,
            enable_standard_rdp_security: true,
            keyboard_type: ironrdp_pdu::gcc::KeyboardType::IBM_ENHANCED,
            keyboard_subtype: 0,
            keyboard_layout: 0,
            keyboard_functional_keys_count: 12,
            ime_file_name: String::new(),
            dig_product_id: String::new(),
            desktop_size: DesktopSize {
                width: 800,
                height: 600,
            },
            desktop_scale_factor: 0,
            monitor_layout: None,
            desktop_physical_size: None,
            desktop_orientation: None,
            device_scale_factor: None,
            monitor_extended_layout: None,
            relative_mouse_input: false,
            connection_type: ironrdp_pdu::gcc::ConnectionType::NotUsed,
            bitmap: None,
            client_build: 0,
            client_name: String::new(),
            client_dir: String::new(),
            platform: ironrdp_pdu::rdp::capability_sets::MajorPlatformType::UNIX,
            hardware_id: None,
            request_data: None,
            autologon: false,
            enable_audio_playback: false,
            enable_audio_capture: false,
            performance_flags: ironrdp_pdu::rdp::client_info::PerformanceFlags::empty(),
            license_cache: None,
            timezone_info: ironrdp_pdu::rdp::client_info::TimezoneInfo::default(),
            compression_type: None,
            enable_server_pointer: false,
            pointer_software_rendering: false,
            multitransport_flags: None,
            alternate_shell: String::new(),
            work_dir: String::new(),
            remote_application_mode: false,
            rail_support_level: ironrdp_pdu::rdp::capability_sets::RailSupportLevel::empty(),
            support_dyn_vc_gfx_protocol: false,
        };
        let _ = &mut config;
        let factory = ConnectionActivationFactory::new(config, 1002, 1001);
        let stage = ironrdp_session::ActiveStageBuilder {
            static_channels: ironrdp_svc::StaticChannelSet::new(),
            user_channel_id: 1001,
            io_channel_id: 1002,
            message_channel_id: Some(1003),
            share_id: 1,
            compression_type: None,
            enable_server_pointer: false,
            pointer_software_rendering: false,
        }
        .build();
        (factory, stage)
    }

    #[tokio::test]
    async fn reactivation_aborts_immediately_on_shutdown_and_notifies_on_eof() {
        let (factory, mut stage) = build_test_stage();
        let (events_tx, mut events_rx) = mpsc::channel(4);

        // 1. 已触发停机信号时立即返回 None，不发 Terminated
        let (_shutdown_tx, mut shutdown_rx) = watch::channel(true);
        let mut reader: ironrdp_tokio::TokioFramed<EofStream> =
            ironrdp_tokio::TokioFramed::new(EofStream);
        let mut writer: ironrdp_tokio::TokioFramed<EofStream> =
            ironrdp_tokio::TokioFramed::new(EofStream);
        let res = execute_reactivation(
            &factory,
            &mut stage,
            &mut reader,
            &mut writer,
            &events_tx,
            &mut shutdown_rx,
        )
        .await;
        assert!(res.is_none());
        assert!(events_rx.try_recv().is_err());

        // 2. 流意外 EOF 时必须发送 Terminated 防止界面黑屏僵死
        let (_shutdown_tx2, mut shutdown_rx2) = watch::channel(false);
        let res_eof = execute_reactivation(
            &factory,
            &mut stage,
            &mut reader,
            &mut writer,
            &events_tx,
            &mut shutdown_rx2,
        )
        .await;
        assert!(res_eof.is_none());
        assert!(matches!(
            events_rx.try_recv(),
            Ok(RdpOutput::Terminated(RdpError::Protocol { .. }))
        ));
    }
}
