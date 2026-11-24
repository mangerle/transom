//! RDP 数据会话主循环与事件驱动。
//!
//! 设计原理（Why）：会话循环运行在独立 OS 线程的 current-thread 运行时中，驱动 IronRDP
//! 的 `ActiveStage` 协议状态机：
//! 1. **100% 取消安全的 PDU 读取**：内层循环通过 `tokio::pin!` 固定 `reader.read_pdu()`，
//!    处理用户输入、帧刷新或防抖定时器时绝不中途丢弃正在读取的网络 Future；
//! 2. **固定空间脏瓦片合并与无反压帧投递**：由 [`TileExtractor`] 在恒定大小位图上合并损伤，
//!    视频帧通道满时保留脏位图延后合并，绝不反压阻塞网络读写与用户输入；
//! 3. **输入批处理委托**：由 [`InputBatcher`] 批量合并相邻 FastPath 事件并复用操作缓冲；
//! 4. **尾沿防抖动态分辨率与完整重激活闭环**：由 [`ResizeController`] 保证最终窗口尺寸
//!    偶数对齐且不丢尾，收到 `DeactivateAll` 后支持取消与失败上报。

use crate::error::RdpError;
use crate::session::channel::{RdpInput, RdpOutput, SessionSenders};
use crate::session::cliprdr::drive_cliprdr_events;
use crate::session::connector::{ErasedStream, EstablishedConnection};
use crate::session::input::{InputBatchOutcome, InputBatcher, InputProcessContext};
use crate::session::output::{
    LoopAction, dispatch_stage_outputs, flush_dirty_tiles, flush_network_responses,
};
use crate::session::reactivate::execute_reactivation;
use crate::session::resize::ResizeController;
use crate::settings::DEFAULT_COMPRESSION_TYPE;
use crate::tiles::TileExtractor;
use ironrdp_connector::connection_activation::ConnectionActivationFactory;
use ironrdp_graphics::image_processing::PixelFormat;
use ironrdp_session::image::DecodedImage;
use ironrdp_session::{ActiveStage, ActiveStageBuilder};
use ironrdp_tokio::{FramedRead, FramedWrite, TokioFramed, split_tokio_framed};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};
use tokio::time::MissedTickBehavior;

/// 桌面图像增量帧最小聚合节流间隔（约 60 FPS）。
const MIN_FRAME_INTERVAL: Duration = Duration::from_millis(16);

/// 连续 PDU 解码失败上限，避免协议流错位后陷入无限错误循环。
const MAX_CONSECUTIVE_DECODE_ERRORS: u32 = 16;

/// 构造会话主循环的参数对象。
pub(crate) struct SessionLoopParams {
    pub(crate) connection: EstablishedConnection,
    pub(crate) input_rx: mpsc::Receiver<RdpInput>,
    pub(crate) mouse_rx: watch::Receiver<Option<(u16, u16)>>,
    pub(crate) senders: SessionSenders,
    pub(crate) shutdown_rx: watch::Receiver<bool>,
}

/// 会话循环主运行器。
pub(crate) struct SessionLoopRunner {
    framed: Option<TokioFramed<ErasedStream>>,
    active_stage: ActiveStage,
    activation_factory: ConnectionActivationFactory,
    canvas: DecodedImage,
    tile_extractor: TileExtractor,
    input_rx: mpsc::Receiver<RdpInput>,
    mouse_rx: watch::Receiver<Option<(u16, u16)>>,
    senders: SessionSenders,
    shutdown_rx: watch::Receiver<bool>,
    audio_muted: Arc<AtomicBool>,
    input_batcher: InputBatcher,
    resize_controller: ResizeController,
    current_resolution: (u16, u16),
    consecutive_decode_errors: u32,
}

impl SessionLoopRunner {
    /// 基于握手结果与通信通道构造主循环驱动器。
    pub(crate) fn new(params: SessionLoopParams) -> Self {
        let EstablishedConnection {
            framed,
            result,
            audio_muted,
        } = params.connection;

        let width = result.desktop_size.width;
        let height = result.desktop_size.height;
        let canvas = DecodedImage::new(PixelFormat::RgbA32, width, height);

        let active_stage = ActiveStageBuilder {
            static_channels: result.static_channels,
            user_channel_id: result.user_channel_id,
            io_channel_id: result.io_channel_id,
            message_channel_id: result.message_channel_id,
            share_id: result.share_id,
            compression_type: Some(DEFAULT_COMPRESSION_TYPE),
            enable_server_pointer: result.enable_server_pointer,
            pointer_software_rendering: result.pointer_software_rendering,
        }
        .build();

        Self {
            framed: Some(framed),
            active_stage,
            activation_factory: result.activation_factory,
            canvas,
            tile_extractor: TileExtractor::default(),
            input_rx: params.input_rx,
            mouse_rx: params.mouse_rx,
            senders: params.senders,
            shutdown_rx: params.shutdown_rx,
            audio_muted,
            input_batcher: InputBatcher::default(),
            resize_controller: ResizeController::new((width, height)),
            current_resolution: (width, height),
            consecutive_decode_errors: 0,
        }
    }

    /// 执行会话事件循环直到被中止、远端断开或发生网络错误。
    pub(crate) async fn run(mut self) {
        if *self.shutdown_rx.borrow_and_update() {
            return;
        }
        let Some(framed) = self.framed.take() else {
            return;
        };
        let (width, height) = self.current_resolution;
        let _ = self
            .senders
            .events
            .send(RdpOutput::Connected { width, height })
            .await;

        let (mut reader, mut writer) = split_tokio_framed(framed);
        let mut frame_tick = tokio::time::interval(MIN_FRAME_INTERVAL);
        frame_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            if *self.shutdown_rx.borrow_and_update() {
                self.send_graceful_shutdown(&mut writer).await;
                break;
            }
            match self
                .poll_next_step(&mut reader, &mut writer, &mut frame_tick)
                .await
            {
                LoopAction::Continue => {}
                LoopAction::Reactivate => {
                    if !self.handle_deactivate_all(&mut reader, &mut writer).await {
                        break;
                    }
                }
                LoopAction::Stop => break,
            }
        }
    }

    /// 取消安全地等待下一帧网络 PDU，期间并发处理停机、帧刷新、尾沿防抖与 UI 输入批处理。
    async fn poll_next_step<R, W>(
        &mut self,
        reader: &mut ironrdp_tokio::Framed<R>,
        writer: &mut W,
        frame_tick: &mut tokio::time::Interval,
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
                    _ = self.shutdown_rx.changed() => {
                        self.send_graceful_shutdown(writer).await;
                        return LoopAction::Stop;
                    }
                    _ = frame_tick.tick(), if has_pending => {
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
            Ok((action, payload)) => self.handle_pdu(action, &payload, writer).await,
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
        let Some((width, height)) = self.resize_controller.take_due(now) else {
            return;
        };

        match self
            .active_stage
            .encode_resize(u32::from(width), u32::from(height), None, None)
        {
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
                    .requeue_if_channel_not_ready((width, height), now);
            }
        }
    }

    /// 发送协议层优雅停机请求。
    async fn send_graceful_shutdown<W: FramedWrite>(&self, writer: &mut W) {
        if let Ok(outputs) = self.active_stage.graceful_shutdown() {
            let _ = flush_network_responses(writer, &outputs).await;
        }
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

    /// 处理单条来自网络的 RDP PDU 报文。
    async fn handle_pdu<W: FramedWrite>(
        &mut self,
        action: ironrdp_pdu::Action,
        payload: &[u8],
        writer: &mut W,
    ) -> LoopAction {
        match self
            .active_stage
            .process(&mut self.canvas, action, payload, None)
        {
            Ok(outputs) => {
                self.consecutive_decode_errors = 0;
                let action = dispatch_stage_outputs(writer, &self.senders.events, &outputs).await;
                if !matches!(action, LoopAction::Continue) {
                    return action;
                }

                let damages = self.active_stage.take_damage_regions();
                if !damages.is_empty() {
                    self.tile_extractor.mark_dirty(
                        self.canvas.width(),
                        self.canvas.height(),
                        &damages,
                    );
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
    async fn handle_deactivate_all<R, W>(
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

        self.canvas = DecodedImage::new(PixelFormat::RgbA32, width, height);
        self.tile_extractor.reset();
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
