//! RDP 会话单步循环：取消安全读取、帧刷新、输入处理与网络遥测。

use super::{MAX_FRAME_COALESCE_DELAY, NO_RTT_SENTINEL, SessionLoopRunner};
use crate::error::RdpError;
use crate::session::channel::{RdpInput, RdpOutput};
use crate::session::input::{InputBatchOutcome, InputProcessContext};
use crate::session::output::{
    LoopAction, dispatch_stage_outputs, flush_dirty_tiles, flush_network_responses,
};
use ironrdp_tokio::{FramedRead, FramedWrite};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

impl SessionLoopRunner {
    /// 取消安全地等待下一帧网络 PDU，期间并发处理停机、帧刷新、遥测上报、尾沿防抖与输入批处理。
    pub(super) async fn poll_next_step<R, W>(
        &mut self,
        reader: &mut ironrdp_tokio::Framed<R>,
        writer: &mut W,
        frame_tick: &mut tokio::time::Interval,
        stats_tick: &mut tokio::time::Interval,
    ) -> LoopAction
    where
        R: FramedRead,
        W: FramedWrite,
    {
        let pdu_result = {
            let read_fut = reader.read_pdu();
            tokio::pin!(read_fut);
            let resize_sleep = tokio::time::sleep(Duration::ZERO);
            tokio::pin!(resize_sleep);
            let mut armed_deadline: Option<Instant> = None;

            loop {
                let has_pending = self.tile_extractor.has_pending();
                let waiting_frame_end = self
                    .in_surface_frame_since
                    .is_some_and(|since| since.elapsed() < MAX_FRAME_COALESCE_DELAY);
                let next_deadline = self.resize_controller.next_deadline();
                if next_deadline != armed_deadline {
                    armed_deadline = next_deadline;
                    if let Some(deadline) = next_deadline {
                        resize_sleep
                            .as_mut()
                            .reset(tokio::time::Instant::from_std(deadline));
                    }
                }

                tokio::select! {
                    biased;
                    _ = self.shutdown_rx.changed() => {
                        self.send_graceful_shutdown(writer).await;
                        return LoopAction::Stop;
                    }
                    _ = frame_tick.tick(), if has_pending && !waiting_frame_end => {
                        self.in_surface_frame_since = None;
                        flush_dirty_tiles(
                            &mut self.tile_extractor,
                            &self.canvas,
                            &self.senders.frames,
                        );
                    }
                    _ = &mut resize_sleep, if armed_deadline.is_some() => {
                        armed_deadline = None;
                        self.flush_due_resize(writer).await;
                    }
                    _ = stats_tick.tick() => {
                        self.emit_network_stats();
                    }
                    maybe_input = self.input_rx.recv() => {
                        let Some(first_input) = maybe_input else {
                            return LoopAction::Stop;
                        };
                        if matches!(
                            self.handle_input_batch(first_input, writer).await,
                            LoopAction::Stop
                        ) {
                            return LoopAction::Stop;
                        }
                    }
                    Ok(()) = self.mouse_rx.changed() => {
                        let pos = *self.mouse_rx.borrow_and_update();
                        if let Some((x, y)) = pos {
                            self.input_batcher.prepare_mouse_move(x, y);
                            if matches!(self.flush_fastpath_ops(writer).await, LoopAction::Stop) {
                                return LoopAction::Stop;
                            }
                        }
                    }
                    res = &mut read_fut => {
                        break res;
                    }
                }
            }
        };

        match pdu_result {
            Ok((action, payload)) => {
                self.rx_bytes_accum = self
                    .rx_bytes_accum
                    .saturating_add((payload.len() as u64).saturating_add(4));
                self.handle_pdu(action, &payload, writer).await
            }
            Err(e) => {
                log::info!("RDP 远端断开连接或发生 I/O 错误: {e}");
                let _ = self
                    .senders
                    .events
                    .send(RdpOutput::Terminated(RdpError::Network {
                        address: "远端连接".to_string(),
                        source: e,
                    }))
                    .await;
                LoopAction::Stop
            }
        }
    }

    /// 触发已到期的动态分辨率请求（若通道尚未就绪则有限次数重试）。
    async fn flush_due_resize<W: FramedWrite>(&mut self, writer: &mut W) {
        let now = Instant::now();
        let Some((width, height, scale_percent)) = self.resize_controller.take_due(now) else {
            return;
        };

        match self.active_stage.encode_resize(
            u32::from(width),
            u32::from(height),
            Some(scale_percent),
            None,
        ) {
            Some(Ok(response_frame)) => {
                if let Err(e) = writer.write_all(&response_frame).await {
                    log::warn!("写入动态重设分辨率帧失败: {e}");
                }
            }
            Some(Err(e)) => {
                log::warn!("编码动态重设分辨率失败: {e}");
            }
            None => {
                self.resize_controller
                    .requeue_if_channel_not_ready((width, height, scale_percent), now);
            }
        }
    }

    /// 发送协议层优雅停机请求。
    pub(super) async fn send_graceful_shutdown<W: FramedWrite>(&self, writer: &mut W) {
        if let Ok(outputs) = self.active_stage.graceful_shutdown() {
            let _ = flush_network_responses(writer, &outputs).await;
        }
    }

    /// 计算最近采样周期的下行速率并向 UI 非阻塞推送实时网络质量遥测。
    fn emit_network_stats(&mut self) {
        let elapsed = self.last_stats_at.elapsed().as_secs_f64();
        let bytes = self.rx_bytes_accum;
        self.rx_bytes_accum = 0;
        self.last_stats_at = Instant::now();
        let rx_bytes_per_sec = if elapsed > 0.05 {
            ((bytes as f64) / elapsed).round() as u64
        } else {
            0
        };
        let raw_rtt = self.latest_rtt_ms.load(Ordering::Relaxed);
        let rtt_ms = (raw_rtt != NO_RTT_SENTINEL).then_some(raw_rtt);
        let _ = self.senders.events.try_send(RdpOutput::NetworkStats {
            rx_bytes_per_sec,
            rtt_ms,
        });
    }

    /// 委托 [`InputBatcher`] 批量处理输入并一次性刷出 FastPath 报文。
    async fn handle_input_batch<W: FramedWrite>(
        &mut self,
        first: RdpInput,
        writer: &mut W,
    ) -> LoopAction {
        let pending_mouse = if self.mouse_rx.has_changed().unwrap_or(false) {
            *self.mouse_rx.borrow_and_update()
        } else {
            None
        };
        let mut ctx = InputProcessContext {
            active_stage: &mut self.active_stage,
            writer,
            audio_muted: &self.audio_muted,
            resize_controller: &mut self.resize_controller,
        };
        let outcome = self
            .input_batcher
            .process_batch(pending_mouse, first, &mut self.input_rx, &mut ctx)
            .await;

        let action = self.flush_fastpath_ops(writer).await;
        if matches!(outcome, InputBatchOutcome::Stop) {
            self.send_graceful_shutdown(writer).await;
            return LoopAction::Stop;
        }
        action
    }

    /// 将当前累积的键鼠原子操作一次性转换为 FastPath PDU 并写入网络。
    async fn flush_fastpath_ops<W: FramedWrite>(&mut self, writer: &mut W) -> LoopAction {
        let Some(result) = self
            .input_batcher
            .flush_to_stage(&mut self.active_stage, &mut self.canvas)
        else {
            return LoopAction::Continue;
        };
        match result {
            Ok(outputs) => dispatch_stage_outputs(writer, &self.senders.events, &outputs).await,
            Err(e) => {
                log::warn!("处理 FastPath 输入异常: {e}");
                LoopAction::Continue
            }
        }
    }
}
