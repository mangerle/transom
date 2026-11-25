//! RDP PDU 处理、帧边界识别与重激活闭环。

use super::{MAX_CONSECUTIVE_DECODE_ERRORS, SessionLoopRunner};
use crate::error::RdpError;
use crate::session::channel::RdpOutput;
use crate::session::cliprdr::drive_cliprdr_events;
use crate::session::output::{LoopAction, dispatch_stage_outputs, flush_dirty_tiles};
use crate::session::reactivate::execute_reactivation;
use ironrdp_core::{MonotonicInstant, ReadCursor, decode_cursor};
use ironrdp_graphics::image_processing::PixelFormat;
use ironrdp_pdu::fast_path::{FastPathHeader, FastPathUpdatePdu, Fragmentation, UpdateCode};
use ironrdp_pdu::surface_commands::{FrameAction, SurfaceCommand};
use ironrdp_session::image::DecodedImage;
use ironrdp_tokio::{FramedRead, FramedWrite};
use std::time::Instant;

impl SessionLoopRunner {
    /// 处理单条来自网络的 RDP PDU 报文。
    pub(super) async fn handle_pdu<W: FramedWrite>(
        &mut self,
        action: ironrdp_pdu::Action,
        payload: &[u8],
        writer: &mut W,
    ) -> LoopAction {
        let is_fastpath = matches!(action, ironrdp_pdu::Action::FastPath);
        let received_at =
            MonotonicInstant::from_millis(self.started_at.elapsed().as_millis() as u64);
        match self
            .active_stage
            .process(&mut self.canvas, action, payload, Some(received_at))
        {
            Ok(outputs) => {
                self.consecutive_decode_errors = 0;
                let action = dispatch_stage_outputs(writer, &self.senders.events, &outputs).await;
                if !matches!(action, LoopAction::Continue) {
                    return action;
                }

                let frame_marker = if is_fastpath {
                    inspect_fastpath_frame_marker(payload)
                } else {
                    None
                };

                let damages = self.active_stage.take_damage_regions();
                let new_size = (self.canvas.width(), self.canvas.height());
                if new_size != self.current_resolution && new_size.0 > 0 && new_size.1 > 0 {
                    self.current_resolution = new_size;
                    self.resize_controller.set_current(new_size);
                    self.tile_extractor.reset();
                    let _ = self
                        .senders
                        .events
                        .send(RdpOutput::Resized {
                            width: new_size.0,
                            height: new_size.1,
                        })
                        .await;
                }
                if !damages.is_empty() {
                    self.tile_extractor.mark_dirty(
                        self.canvas.width(),
                        self.canvas.height(),
                        &damages,
                    );
                }

                match frame_marker {
                    Some(FrameAction::Begin) => {
                        self.in_surface_frame_since.get_or_insert_with(Instant::now);
                    }
                    Some(FrameAction::End) => {
                        self.in_surface_frame_since = None;
                        if self.tile_extractor.has_pending() {
                            flush_dirty_tiles(
                                &mut self.tile_extractor,
                                &self.canvas,
                                &self.senders.frames,
                            );
                        }
                    }
                    None => {}
                }

                if let Err(e) = drive_cliprdr_events(&mut self.active_stage, writer).await {
                    log::info!("写入 CLIPRDR 响应时连接中断: {e}");
                    let _ = self
                        .senders
                        .events
                        .send(RdpOutput::Terminated(RdpError::Network {
                            address: "远端连接".to_string(),
                            source: e,
                        }))
                        .await;
                    return LoopAction::Stop;
                }
            }
            Err(e) => {
                self.consecutive_decode_errors = self.consecutive_decode_errors.saturating_add(1);
                log::error!(
                    "解码处理 RDP 报文失败（连续第 {} 次）: {e}",
                    self.consecutive_decode_errors
                );
                if self.consecutive_decode_errors >= MAX_CONSECUTIVE_DECODE_ERRORS {
                    let _ = self
                        .senders
                        .events
                        .send(RdpOutput::Terminated(RdpError::Protocol {
                            stage: "ActiveStage 报文解码",
                            detail: format!("连续解码失败超过上限: {e}"),
                        }))
                        .await;
                    return LoopAction::Stop;
                }
            }
        }
        LoopAction::Continue
    }

    /// 执行标准 Deactivation-Reactivation 序列并重建本地画布与分辨率状态。
    pub(super) async fn handle_deactivate_all<R, W>(
        &mut self,
        reader: &mut ironrdp_tokio::Framed<R>,
        writer: &mut ironrdp_tokio::Framed<W>,
    ) -> bool
    where
        R: FramedRead,
        W: FramedWrite,
    {
        let Some((width, height)) = execute_reactivation(
            &self.activation_factory,
            &mut self.active_stage,
            reader,
            writer,
            &self.senders.events,
            &mut self.shutdown_rx,
        )
        .await
        else {
            return false;
        };

        self.canvas = DecodedImage::new(PixelFormat::BgrX32, width, height);
        self.tile_extractor.reset();
        self.in_surface_frame_since = None;
        self.current_resolution = (width, height);
        self.resize_controller.set_current((width, height));

        let _ = self
            .senders
            .events
            .send(RdpOutput::Resized { width, height })
            .await;
        true
    }
}

/// 窥视未压缩 FastPath SurfaceCommands 中的帧边界标记（`Begin` / `End`），用于整帧合并提交。
fn inspect_fastpath_frame_marker(packet: &[u8]) -> Option<FrameAction> {
    let mut input = ReadCursor::new(packet);
    let header = decode_cursor::<FastPathHeader>(&mut input).ok()?;
    if !header.flags.is_empty() {
        return None;
    }

    let mut last_marker = None;
    while !input.is_empty() {
        let Ok(update) = decode_cursor::<FastPathUpdatePdu<'_>>(&mut input) else {
            break;
        };
        if update.update_code != UpdateCode::SurfaceCommands
            || update.fragmentation != Fragmentation::Single
            || update.compression_flags.is_some_and(|f| !f.is_empty())
        {
            continue;
        }
        let mut cmd_cur = ReadCursor::new(update.data);
        while !cmd_cur.is_empty() {
            match decode_cursor::<SurfaceCommand<'_>>(&mut cmd_cur) {
                Ok(SurfaceCommand::FrameMarker(marker)) => {
                    last_marker = Some(marker.frame_action);
                }
                Ok(SurfaceCommand::SetSurfaceBits(_) | SurfaceCommand::StreamSurfaceBits(_)) => {}
                Err(_) => break,
            }
        }
    }
    last_marker
}
