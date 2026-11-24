//! RDP 数据会话主循环与事件驱动。
//!
//! 设计原理（Why）：会话循环运行在独立的异步任务中，驱动 IronRDP 的 `ActiveStage`
//! 协议状态机。通过 `tokio::select!` 多路复用监听网络入站 PDU、UI 输入指令、16ms 帧合并
//! 定时器与停机取消信号：
//! 1. **无反压阻塞帧投递**：视频帧通道满时跳过本次提取并保留损伤矩形，在下一帧自动合并，
//!    确保 UI 渲染卡顿或后台标签不消费时绝不阻塞网络读包与用户输入；
//! 2. **完整重激活闭环**：收到 `ActiveStageOutput::DeactivateAll` 后执行标准
//!    Deactivation-Reactivation 序列，重建画布并向 UI 通知新分辨率；
//! 3. **平滑停机**：监听 `shutdown_rx` 或 `RdpInput::Disconnect`，发送关闭请求后安全退出。

use crate::clipboard::CliprdrSyncContext;
use crate::error::RdpError;
use crate::keyboard::KeyboardTranslator;
use crate::pointer::{CursorUpdate, create_custom_cursor};
use crate::session::channel::{MouseBtn, RdpInput, RdpOutput, SessionSenders};
use crate::session::cliprdr::{drive_cliprdr_events, handle_local_clipboard_copy};
use crate::session::connector::{ErasedStream, EstablishedConnection};
use crate::session::reactivate::execute_reactivation;
use crate::tiles::TileExtractor;
use ironrdp_connector::connection_activation::ConnectionActivationFactory;
use ironrdp_graphics::image_processing::PixelFormat;
use ironrdp_input::{
    Database as InputDatabase, MouseButton, MousePosition, Operation, WheelRotations,
};
use ironrdp_pdu::geometry::InclusiveRectangle;
use ironrdp_session::image::DecodedImage;
use ironrdp_session::{ActiveStage, ActiveStageBuilder, ActiveStageOutput};
use ironrdp_tokio::{FramedRead, FramedWrite, TokioFramed, split_tokio_framed};
use parking_lot::Mutex;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};

/// 动态调整分辨率最小防抖冷却间隔（毫秒）。
const RESIZE_DEBOUNCE_INTERVAL: Duration = Duration::from_millis(250);

/// 桌面图像增量帧最小聚合节流间隔（约 60 FPS）。
const MIN_FRAME_INTERVAL: Duration = Duration::from_millis(16);

/// 动态分辨率安全限制范围。
const MIN_DESKTOP_WIDTH: u16 = 640;
const MAX_DESKTOP_WIDTH: u16 = 4096;
const MIN_DESKTOP_HEIGHT: u16 = 480;
const MAX_DESKTOP_HEIGHT: u16 = 2560;

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
    pending_damages: Vec<InclusiveRectangle>,
    last_frame_flushed: Instant,
    last_resize_at: Instant,
    current_resolution: (u16, u16),
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
        let canvas = DecodedImage::new(PixelFormat::BgrX32, width, height);

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
            pending_damages: Vec::new(),
            last_frame_flushed: Instant::now(),
            last_resize_at: Instant::now() - RESIZE_DEBOUNCE_INTERVAL,
            current_resolution: (width, height),
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

        loop {
            let wait_time = MIN_FRAME_INTERVAL.saturating_sub(self.last_frame_flushed.elapsed());
            let has_pending = !self.pending_damages.is_empty();

            tokio::select! {
                _ = self.shutdown_rx.changed() => {
                    self.send_graceful_shutdown(&mut writer).await;
                    break;
                }
                _ = tokio::time::sleep(wait_time), if has_pending => {
                    self.flush_dirty_tiles();
                }
                maybe_input = self.input_rx.recv() => {
                    let Some(input) = maybe_input else {
                        break;
                    };
                    if matches!(self.handle_input(input, &mut writer).await, LoopAction::Stop) {
                        break;
                    }
                }
                pdu_result = reader.read_pdu() => {
                    let action = match pdu_result {
                        Ok((action, payload)) => self.handle_pdu(action, &payload, &mut writer).await,
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
                    };
                    match action {
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
        }
    }

    /// 尝试向 UI 非阻塞投递增量瓦片帧；若帧通道已满则保留损伤等待下一帧合并。
    fn flush_dirty_tiles(&mut self) {
        if self.senders.frames.capacity() == 0 {
            // UI 端尚未消费上一帧：保留 pending_damages 延后合并，避免无效像素转换与反压阻塞
            self.last_frame_flushed = Instant::now();
            return;
        }
        if let Some(update) = self
            .tile_extractor
            .extract(&self.canvas, &self.pending_damages)
            && self.senders.frames.try_send(update).is_err()
        {
            self.last_frame_flushed = Instant::now();
            return;
        }
        self.pending_damages.clear();
        self.last_frame_flushed = Instant::now();
    }

    /// 发送协议层优雅停机请求。
    async fn send_graceful_shutdown<W: FramedWrite>(&self, writer: &mut W) {
        if let Ok(outputs) = self.active_stage.graceful_shutdown() {
            let _ = flush_network_responses(writer, &outputs).await;
        }
    }

    /// 处理单条来自 UI 层的输入指令。
    async fn handle_input<W: FramedWrite>(
        &mut self,
        input: RdpInput,
        writer: &mut W,
    ) -> LoopAction {
        match input {
            RdpInput::Disconnect => {
                self.send_graceful_shutdown(writer).await;
                return LoopAction::Stop;
            }
            RdpInput::MuteAudio(muted) => {
                self.audio_muted.store(muted, Ordering::Relaxed);
            }
            RdpInput::ClipboardText(text) => {
                handle_local_clipboard_copy(
                    &mut self.active_stage,
                    writer,
                    &self.cliprdr_context,
                    text,
                )
                .await;
            }
            RdpInput::Resize { width, height } => {
                self.handle_resize(writer, width, height).await;
            }
            other => {
                let ops = convert_input_to_operations(other, &mut self.keyboard);
                if ops.is_empty() {
                    return LoopAction::Continue;
                }
                let fastpath_events = self.input_db.apply(ops);
                if fastpath_events.is_empty() {
                    return LoopAction::Continue;
                }
                match self
                    .active_stage
                    .process_fastpath_input(&mut self.canvas, &fastpath_events)
                {
                    Ok(outputs) => {
                        return self.dispatch_stage_outputs(writer, &outputs).await;
                    }
                    Err(e) => {
                        log::warn!("处理 FastPath 输入异常: {e}");
                    }
                }
            }
        }
        LoopAction::Continue
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
                let action = self.dispatch_stage_outputs(writer, &outputs).await;
                if !matches!(action, LoopAction::Continue) {
                    return action;
                }

                let damages = self.active_stage.take_damage_regions();
                if !damages.is_empty() {
                    self.pending_damages.extend(damages);
                }
                if !self.pending_damages.is_empty()
                    && self.last_frame_flushed.elapsed() >= MIN_FRAME_INTERVAL
                {
                    self.flush_dirty_tiles();
                }

                drive_cliprdr_events(&mut self.active_stage, writer, &self.cliprdr_context).await;
            }
            Err(e) => {
                log::error!("解码处理 RDP 报文失败: {e}");
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
        )
        .await
        else {
            return false;
        };

        self.canvas = DecodedImage::new(PixelFormat::BgrX32, width, height);
        self.tile_extractor.reset();
        self.pending_damages.clear();
        self.current_resolution = (width, height);

        let _ = self
            .senders
            .events
            .send(RdpOutput::Resized { width, height })
            .await;
        true
    }

    /// 执行防抖后的动态重设分辨率请求。
    async fn handle_resize<W: FramedWrite>(&mut self, writer: &mut W, width: u16, height: u16) {
        let clamped_w = width.clamp(MIN_DESKTOP_WIDTH, MAX_DESKTOP_WIDTH);
        let clamped_h = height.clamp(MIN_DESKTOP_HEIGHT, MAX_DESKTOP_HEIGHT);
        if (clamped_w, clamped_h) == self.current_resolution
            || self.last_resize_at.elapsed() < RESIZE_DEBOUNCE_INTERVAL
        {
            return;
        }
        self.last_resize_at = Instant::now();
        if let Some(res) =
            self.active_stage
                .encode_resize(u32::from(clamped_w), u32::from(clamped_h), None, None)
        {
            match res {
                Ok(response_frame) => {
                    if let Err(e) = writer.write_all(&response_frame).await {
                        log::warn!("写入动态重设分辨率响应帧失败: {e}");
                    }
                }
                Err(e) => {
                    log::warn!("编码动态重设分辨率失败: {e}");
                }
            }
        }
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

/// 将 UI 操作指令映射为底层原子状态操作集合。
fn convert_input_to_operations(
    input: RdpInput,
    keyboard: &mut KeyboardTranslator,
) -> Vec<Operation> {
    match input {
        RdpInput::MouseMove { x, y } => vec![Operation::MouseMove(MousePosition { x, y })],
        RdpInput::MouseButton { button, pressed } => {
            let btn = match button {
                MouseBtn::Left => MouseButton::Left,
                MouseBtn::Middle => MouseButton::Middle,
                MouseBtn::Right => MouseButton::Right,
                MouseBtn::X1 => MouseButton::X1,
                MouseBtn::X2 => MouseButton::X2,
            };
            if pressed {
                vec![Operation::MouseButtonPressed(btn)]
            } else {
                vec![Operation::MouseButtonReleased(btn)]
            }
        }
        RdpInput::MouseWheel { vertical, delta } => {
            vec![Operation::WheelRotations(WheelRotations {
                is_vertical: vertical,
                rotation_units: delta,
            })]
        }
        RdpInput::KeyDown { key } => keyboard.handle_key_down(&key),
        RdpInput::KeyUp { key } => keyboard.handle_key_up(&key),
        RdpInput::ReleaseAllKeys => keyboard.release_all(),
        _ => Vec::new(),
    }
}
