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
use crate::tiles::{TileBufferPool, TileExtractor};
use ironrdp_connector::connection_activation::ConnectionActivationFactory;
use ironrdp_core::{MonotonicInstant, ReadCursor, decode_cursor};
use ironrdp_graphics::image_processing::PixelFormat;
use ironrdp_pdu::fast_path::{FastPathHeader, FastPathUpdatePdu, Fragmentation, UpdateCode};
use ironrdp_pdu::surface_commands::{FrameAction, SurfaceCommand};
use ironrdp_session::image::DecodedImage;
use ironrdp_session::{ActiveStage, ActiveStageBuilder};
use ironrdp_tokio::{FramedRead, FramedWrite, TokioFramed, split_tokio_framed};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};
use tokio::time::MissedTickBehavior;

/// 桌面图像增量帧最小聚合节流间隔（约 60 FPS）。
const MIN_FRAME_INTERVAL: Duration = Duration::from_millis(16);

/// 等待 `SurfaceCommands` `EndFrame` 的最大容忍时长，防止异常分片一直阻塞刷帧。
const MAX_FRAME_COALESCE_DELAY: Duration = Duration::from_millis(32);

/// 连续 PDU 解码失败上限，避免协议流错位后陷入无限错误循环。
const MAX_CONSECUTIVE_DECODE_ERRORS: u32 = 16;

/// 表示尚未采集到有效 RTT 的哨兵值。
const NO_RTT_SENTINEL: u32 = u32::MAX;

/// 网络遥测采样与上报间隔（秒）。
const NETWORK_STATS_INTERVAL: Duration = Duration::from_secs(1);

/// 轻量 TCP RTT 探测间隔（秒）。
const RTT_PROBE_INTERVAL: Duration = Duration::from_secs(3);

/// 构造会话主循环的参数对象。
pub(crate) struct SessionLoopParams {
    pub(crate) connection: EstablishedConnection,
    pub(crate) input_rx: mpsc::Receiver<RdpInput>,
    pub(crate) mouse_rx: watch::Receiver<Option<(u16, u16)>>,
    pub(crate) buffer_pool: TileBufferPool,
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
    in_surface_frame_since: Option<Instant>,
    peer_addr: Option<SocketAddr>,
    latest_rtt_ms: Arc<AtomicU32>,
    rx_bytes_accum: u64,
    last_stats_at: Instant,
    started_at: Instant,
}

impl SessionLoopRunner {
    /// 基于握手结果与通信通道构造主循环驱动器。
    pub(crate) fn new(params: SessionLoopParams) -> Self {
        let EstablishedConnection {
            framed,
            result,
            audio_muted,
            scale_factor_percent,
            peer_addr,
            initial_rtt_ms,
        } = params.connection;

        let width = result.desktop_size.width;
        let height = result.desktop_size.height;
        let canvas = DecodedImage::new(PixelFormat::BgrX32, width, height);

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
            tile_extractor: TileExtractor::with_pool(params.buffer_pool),
            input_rx: params.input_rx,
            mouse_rx: params.mouse_rx,
            senders: params.senders,
            shutdown_rx: params.shutdown_rx,
            audio_muted,
            input_batcher: InputBatcher::default(),
            resize_controller: ResizeController::new((width, height), scale_factor_percent),
            current_resolution: (width, height),
            consecutive_decode_errors: 0,
            in_surface_frame_since: None,
            peer_addr,
            latest_rtt_ms: Arc::new(AtomicU32::new(
                initial_rtt_ms.unwrap_or(NO_RTT_SENTINEL),
            )),
            rx_bytes_accum: 0,
            last_stats_at: Instant::now(),
            started_at: Instant::now(),
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
        let mut stats_tick = tokio::time::interval(NETWORK_STATS_INTERVAL);
        stats_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

        let rtt_probe_handle = self.peer_addr.map(|addr| {
            let rtt_store = Arc::clone(&self.latest_rtt_ms);
            let mut probe_shutdown = self.shutdown_rx.clone();
            tokio::spawn(async move {
                let mut ticker = tokio::time::interval(RTT_PROBE_INTERVAL);
                ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
                loop {
                    tokio::select! {
                        _ = probe_shutdown.changed() => break,
                        _ = ticker.tick() => {
                            let started = Instant::now();
                            let Ok(Ok(stream)) = tokio::time::timeout(
                                Duration::from_secs(2),
                                tokio::net::TcpStream::connect(addr),
                            )
                            .await else {
                                continue;
                            };
                            drop(stream);
                            let ms = u32::try_from(started.elapsed().as_millis())
                                .unwrap_or(u32::MAX - 1)
                                .min(NO_RTT_SENTINEL - 1);
                            rtt_store.store(ms, Ordering::Relaxed);
                        }
                    }
                }
            })
        });

        loop {
            if *self.shutdown_rx.borrow_and_update() {
                self.send_graceful_shutdown(&mut writer).await;
                break;
            }
            match self
                .poll_next_step(&mut reader, &mut writer, &mut frame_tick, &mut stats_tick)
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

        if let Some(handle) = rtt_probe_handle {
            handle.abort();
            let _ = handle.await;
        }
    }

    /// 取消安全地等待下一帧网络 PDU，期间并发处理停机、帧刷新、遥测上报、尾沿防抖与输入批处理。
    async fn poll_next_step<R, W>(
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
    async fn send_graceful_shutdown<W: FramedWrite>(&self, writer: &mut W) {
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

    /// 处理单条来自网络的 RDP PDU 报文。
    async fn handle_pdu<W: FramedWrite>(
        &mut self,
        action: ironrdp_pdu::Action,
        payload: &[u8],
        writer: &mut W,
    ) -> LoopAction {
        let is_fastpath = matches!(action, ironrdp_pdu::Action::FastPath);
        let received_at =
            MonotonicInstant::from_millis(self.started_at.elapsed().as_millis() as u64);
        match self.active_stage.process(
            &mut self.canvas,
            action,
            payload,
            Some(received_at),
        ) {
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
