//! RDP 数据会话主循环与事件驱动。
//!
//! 设计原理（Why）：会话循环运行在独立 OS 线程的 current-thread 运行时中，驱动 IronRDP
//! 的 `ActiveStage` 协议状态机：
//! 1. **100% 取消安全的 PDU 读取**：内层循环通过 `tokio::pin!` 固定 `reader.read_pdu()`，
//!    处理用户输入、帧刷新或防抖定时器时绝不中途丢弃正在读取的网络 Future；
//! 2. **固定空间脏瓦片合并与无反压帧投递**：由 [`TileExtractor`] 在恒定大小位图上合并损伤，
//!    视频帧通道满时保留脏位图延后合并，绝不反压阻塞网络读写与用户输入；
//! 3. **输入事件批处理**：收到输入时通过 `try_recv` 批量合并相邻 FastPath 事件并复用
//!    `ops_buf`，降低高频鼠标与键盘的系统调用及堆分配开销；
//! 4. **尾沿防抖动态分辨率与完整重激活闭环**：由 [`ResizeController`] 保证最终窗口尺寸
//!    偶数对齐且不丢尾，收到 `DeactivateAll` 后支持取消与失败上报。

use crate::clipboard::CliprdrSyncContext;
use crate::error::RdpError;
use crate::keyboard::KeyboardTranslator;
use crate::pointer::{CursorUpdate, create_custom_cursor};
use crate::session::channel::{MouseBtn, RdpInput, RdpOutput, SessionSenders};
use crate::session::cliprdr::{drive_cliprdr_events, handle_local_clipboard_copy};
use crate::session::connector::{ErasedStream, EstablishedConnection};
use crate::session::reactivate::execute_reactivation;
use crate::session::resize::ResizeController;
use crate::tiles::TileExtractor;
use ironrdp_connector::connection_activation::ConnectionActivationFactory;
use ironrdp_graphics::image_processing::PixelFormat;
use ironrdp_input::{
    Database as InputDatabase, MouseButton, MousePosition, Operation, WheelRotations,
};
use ironrdp_session::image::DecodedImage;
use ironrdp_session::{ActiveStage, ActiveStageBuilder, ActiveStageOutput};
use ironrdp_tokio::{FramedRead, FramedWrite, TokioFramed, split_tokio_framed};
use parking_lot::Mutex;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};
use tokio::time::MissedTickBehavior;

/// 桌面图像增量帧最小聚合节流间隔（约 60 FPS）。
const MIN_FRAME_INTERVAL: Duration = Duration::from_millis(16);

/// 单次批处理从输入通道排空的最大事件数。
const MAX_INPUT_BATCH_SIZE: usize = 32;

/// 连续 PDU 解码失败上限，避免协议流错位后陷入无限错误循环。
const MAX_CONSECUTIVE_DECODE_ERRORS: u32 = 16;

/// 构造会话主循环的参数对象。
pub(crate) struct SessionLoopParams {
    pub(crate) connection: EstablishedConnection,
    pub(crate) input_rx: mpsc::Receiver<RdpInput>,
    pub(crate) senders: SessionSenders,
    pub(crate) shutdown_rx: watch::Receiver<bool>,
}

/// 单步事件处理后的循环控制指令。
enum LoopAction {
    Continue,
    Reactivate,
    Stop,
}

/// 会话循环主运行器。
pub(crate) struct SessionLoopRunner {
    framed: Option<TokioFramed<ErasedStream>>,
    active_stage: ActiveStage,
    activation_factory: ConnectionActivationFactory,
    canvas: DecodedImage,
    tile_extractor: TileExtractor,
    input_rx: mpsc::Receiver<RdpInput>,
    senders: SessionSenders,
    shutdown_rx: watch::Receiver<bool>,
    audio_muted: Arc<AtomicBool>,
    cliprdr_context: Arc<Mutex<CliprdrSyncContext>>,
    keyboard: KeyboardTranslator,
    input_db: InputDatabase,
    ops_buf: Vec<Operation>,
    last_frame_flushed: Instant,
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
            cliprdr_context,
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
            compression_type: None,
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
            senders: params.senders,
            shutdown_rx: params.shutdown_rx,
            audio_muted,
            cliprdr_context,
            keyboard: KeyboardTranslator::default(),
            input_db: InputDatabase::new(),
            ops_buf: Vec::with_capacity(MAX_INPUT_BATCH_SIZE),
            last_frame_flushed: Instant::now(),
            resize_controller: ResizeController::new((width, height)),
            current_resolution: (width, height),
            consecutive_decode_errors: 0,
        }
    }

    /// 执行会话事件循环直到被中止、远端断开或发生网络错误。
    pub(crate) async fn run(mut self) {
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
                        self.flush_dirty_tiles();
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

    /// 尝试向 UI 非阻塞投递增量瓦片帧；若帧通道已满则保留脏位图等待下一拍合并。
    fn flush_dirty_tiles(&mut self) {
        if self.senders.frames.capacity() == 0 {
            self.last_frame_flushed = Instant::now();
            return;
        }
        if let Some(update) = self.tile_extractor.extract_pending(&self.canvas)
            && self.senders.frames.try_send(update).is_err()
        {
            self.last_frame_flushed = Instant::now();
            return;
        }
        self.tile_extractor.commit_flushed();
        self.last_frame_flushed = Instant::now();
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
                // DisplayControl 动态虚拟通道尚未就绪，顺延重试避免首帧丢弃
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

    /// 批量排空相邻 UI 输入指令，将连续 FastPath 键鼠事件合并为单次网络写入。
    async fn handle_input_batch<W: FramedWrite>(
        &mut self,
        first: RdpInput,
        writer: &mut W,
    ) -> LoopAction {
        self.ops_buf.clear();
        let mut action = self.enqueue_or_handle_control(first, writer).await;
        if matches!(action, LoopAction::Stop) {
            return LoopAction::Stop;
        }

        let mut drained = 1;
        while drained < MAX_INPUT_BATCH_SIZE {
            let Ok(next) = self.input_rx.try_recv() else {
                break;
            };
            drained += 1;
            action = self.enqueue_or_handle_control(next, writer).await;
            if matches!(action, LoopAction::Stop) {
                return LoopAction::Stop;
            }
        }

        self.flush_fastpath_ops(writer).await
    }

    /// 立即处理控制类输入，或将键鼠事件压入复用操作缓冲等待批量刷出。
    async fn enqueue_or_handle_control<W: FramedWrite>(
        &mut self,
        input: RdpInput,
        writer: &mut W,
    ) -> LoopAction {
        match input {
            RdpInput::Disconnect => {
                let _ = self.flush_fastpath_ops(writer).await;
                self.send_graceful_shutdown(writer).await;
                LoopAction::Stop
            }
            RdpInput::MuteAudio(muted) => {
                self.audio_muted.store(muted, Ordering::Relaxed);
                LoopAction::Continue
            }
            RdpInput::ClipboardText(text) => {
                let _ = self.flush_fastpath_ops(writer).await;
                handle_local_clipboard_copy(
                    &mut self.active_stage,
                    writer,
                    &self.cliprdr_context,
                    text,
                )
                .await;
                LoopAction::Continue
            }
            RdpInput::Resize { width, height } => {
                self.resize_controller
                    .schedule(width, height, Instant::now());
                LoopAction::Continue
            }
            fastpath => {
                append_fastpath_operations(fastpath, &mut self.keyboard, &mut self.ops_buf);
                LoopAction::Continue
            }
        }
    }

    /// 将当前累积的键鼠原子操作一次性转换为 FastPath PDU 并写入网络。
    async fn flush_fastpath_ops<W: FramedWrite>(&mut self, writer: &mut W) -> LoopAction {
        if self.ops_buf.is_empty() {
            return LoopAction::Continue;
        }
        let fastpath_events = self.input_db.apply(self.ops_buf.drain(..));
        if fastpath_events.is_empty() {
            return LoopAction::Continue;
        }
        match self
            .active_stage
            .process_fastpath_input(&mut self.canvas, &fastpath_events)
        {
            Ok(outputs) => self.dispatch_stage_outputs(writer, &outputs).await,
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
                let action = self.dispatch_stage_outputs(writer, &outputs).await;
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
                if self.tile_extractor.has_pending()
                    && self.last_frame_flushed.elapsed() >= MIN_FRAME_INTERVAL
                {
                    self.flush_dirty_tiles();
                }

                drive_cliprdr_events(&mut self.active_stage, writer, &self.cliprdr_context).await;
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

    /// 将状态机产出写回网络并通知 UI，同时识别重激活与远端终止信号。
    async fn dispatch_stage_outputs<W: FramedWrite>(
        &self,
        writer: &mut W,
        outputs: &[ActiveStageOutput],
    ) -> LoopAction {
        if flush_network_responses(writer, outputs).await.is_err() {
            return LoopAction::Stop;
        }

        let mut need_reactivate = false;
        for out in outputs {
            match out {
                ActiveStageOutput::PointerDefault => {
                    let _ = self
                        .senders
                        .events
                        .try_send(RdpOutput::Cursor(CursorUpdate::Default));
                }
                ActiveStageOutput::PointerHidden => {
                    let _ = self
                        .senders
                        .events
                        .try_send(RdpOutput::Cursor(CursorUpdate::Hidden));
                }
                ActiveStageOutput::PointerBitmap(ptr) => {
                    if let Some(cursor) = create_custom_cursor(
                        ptr.width,
                        ptr.height,
                        ptr.hotspot_x,
                        ptr.hotspot_y,
                        &ptr.bitmap_data,
                    ) {
                        let _ = self.senders.events.try_send(RdpOutput::Cursor(cursor));
                    }
                }
                ActiveStageOutput::DeactivateAll => {
                    need_reactivate = true;
                }
                ActiveStageOutput::Terminate(reason) => {
                    log::info!("远端服务器发起终止会话: {reason:?}");
                    let _ = self
                        .senders
                        .events
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

/// 将生成的响应帧批量刷入网络套接字。
async fn flush_network_responses<W: FramedWrite>(
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

/// 将 UI 键鼠输入追加到复用的底层原子操作缓冲（避免每条事件堆分配 `Vec`）。
fn append_fastpath_operations(
    input: RdpInput,
    keyboard: &mut KeyboardTranslator,
    out: &mut Vec<Operation>,
) {
    match input {
        RdpInput::MouseMove { x, y } => out.push(Operation::MouseMove(MousePosition { x, y })),
        RdpInput::MouseButton { button, pressed } => {
            let btn = match button {
                MouseBtn::Left => MouseButton::Left,
                MouseBtn::Middle => MouseButton::Middle,
                MouseBtn::Right => MouseButton::Right,
                MouseBtn::X1 => MouseButton::X1,
                MouseBtn::X2 => MouseButton::X2,
            };
            if pressed {
                out.push(Operation::MouseButtonPressed(btn));
            } else {
                out.push(Operation::MouseButtonReleased(btn));
            }
        }
        RdpInput::MouseWheel { vertical, delta } => {
            out.push(Operation::WheelRotations(WheelRotations {
                is_vertical: vertical,
                rotation_units: delta,
            }));
        }
        RdpInput::KeyDown {
            key,
            key_char,
            modifiers,
        } => keyboard.handle_key_down(&key, key_char.as_deref(), modifiers, out),
        RdpInput::KeyUp { key, key_char } => {
            keyboard.handle_key_up(&key, key_char.as_deref(), out);
        }
        RdpInput::ModifiersChanged(modifiers) => keyboard.sync_modifiers(modifiers, out),
        RdpInput::SendKeyChord(chord) => keyboard.handle_key_chord(chord, out),
        RdpInput::ReleaseAllKeys => keyboard.release_all(out),
        _ => {}
    }
}
