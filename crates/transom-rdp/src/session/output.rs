//! RDP 会话状态机输出分发与帧投递辅助模块。
//!
//! 设计原理（Why）：将 `ActiveStageOutput` 协议响应刷写、终止/重激活识别与增量瓦片
//! 帧非阻塞投递从主事件循环 `loop_runner.rs` 中剥离，保持主循环聚焦于 `select!` 调度。

use crate::session::channel::RdpOutput;
use crate::tiles::{FrameUpdate, TileExtractor};
use ironrdp_session::ActiveStageOutput;
use ironrdp_session::image::DecodedImage;
use ironrdp_tokio::FramedWrite;
use tokio::sync::mpsc;

/// 单步事件处理后的循环控制指令。
pub(crate) enum LoopAction {
    Continue,
    Reactivate,
    Stop,
}

/// 尝试向 UI 非阻塞投递增量瓦片帧；若帧通道已满则回收缓冲并保留脏位图等待下一拍合并。
pub(crate) fn flush_dirty_tiles(
    tile_extractor: &mut TileExtractor,
    canvas: &DecodedImage,
    frames_tx: &mpsc::Sender<FrameUpdate>,
) {
    if frames_tx.capacity() == 0 {
        return;
    }
    if let Some(update) = tile_extractor.extract_pending(canvas)
        && let Err(send_err) = frames_tx.try_send(update)
    {
        tile_extractor.recycle_frame(send_err.into_inner());
        return;
    }
    tile_extractor.commit_flushed();
}

/// 将状态机产出写回网络并通知 UI，同时识别重激活与远端终止信号。
pub(crate) async fn dispatch_stage_outputs<W: FramedWrite>(
    writer: &mut W,
    events_tx: &mpsc::Sender<RdpOutput>,
    outputs: &[ActiveStageOutput],
) -> LoopAction {
    if flush_network_responses(writer, outputs).await.is_err() {
        return LoopAction::Stop;
    }

    let mut need_reactivate = false;
    for out in outputs {
        match out {
            ActiveStageOutput::DeactivateAll => {
                need_reactivate = true;
            }
            ActiveStageOutput::Terminate(reason) => {
                log::info!("远端服务器发起终止会话: {reason:?}");
                let _ = events_tx
                    .send(RdpOutput::Disconnected {
                        reason: format!("{reason:?}"),
                    })
                    .await;
                return LoopAction::Stop;
            }
            _ => {}
        }
    }

    if need_reactivate {
        LoopAction::Reactivate
    } else {
        LoopAction::Continue
    }
}

/// 将生成的响应帧批量刷入网络套接字。
pub(crate) async fn flush_network_responses<W: FramedWrite>(
    writer: &mut W,
    outputs: &[ActiveStageOutput],
) -> Result<(), std::io::Error> {
    for out in outputs {
        if let ActiveStageOutput::ResponseFrame(bytes) = out {
            writer.write_all(bytes).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironrdp_graphics::image_processing::PixelFormat;
    use ironrdp_session::GracefulDisconnectReason;
    use std::future::Future;
    use std::pin::Pin;

    struct DummyWriter;

    impl FramedWrite for DummyWriter {
        type WriteAllFut<'a>
            = Pin<Box<dyn Future<Output = std::io::Result<()>> + 'a>>
        where
            Self: 'a;

        fn write_all<'a>(&'a mut self, _buf: &'a [u8]) -> Self::WriteAllFut<'a> {
            Box::pin(async { Ok(()) })
        }
    }

    #[tokio::test]
    async fn dispatch_recognizes_reactivate_and_terminate() {
        let mut writer = DummyWriter;
        let (events_tx, mut events_rx) = mpsc::channel(8);

        let action =
            dispatch_stage_outputs(&mut writer, &events_tx, &[ActiveStageOutput::DeactivateAll])
                .await;
        assert!(matches!(action, LoopAction::Reactivate));

        let term_action = dispatch_stage_outputs(
            &mut writer,
            &events_tx,
            &[ActiveStageOutput::Terminate(
                GracefulDisconnectReason::UserInitiated,
            )],
        )
        .await;
        assert!(matches!(term_action, LoopAction::Stop));
        assert!(matches!(
            events_rx.try_recv(),
            Ok(RdpOutput::Disconnected { .. })
        ));
    }

    #[test]
    fn flush_dirty_tiles_recycles_on_backpressure() {
        let canvas = DecodedImage::new(PixelFormat::RgbA32, 256, 256);
        let mut extractor = TileExtractor::default();
        let (frames_tx, _frames_rx) = mpsc::channel(1);

        // 首次发送成功占用唯一槽位
        flush_dirty_tiles(&mut extractor, &canvas, &frames_tx);
        assert!(!extractor.has_pending());

        // 再次触发整帧重置（通道满），应保留脏状态且不递增 seq
        extractor.reset();
        flush_dirty_tiles(&mut extractor, &canvas, &frames_tx);
        assert!(extractor.has_pending());
        let retry = extractor.extract_pending(&canvas).expect("应可重试提取");
        assert_eq!(retry.seq, 2);
    }
}
