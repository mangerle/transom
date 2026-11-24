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
