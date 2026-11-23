//! RDP 数据会话主循环与事件驱动。
//!
//! 设计原理（Why）：会话循环运行在独立的异步任务中，驱动 IronRDP 的 ActiveStage
//! 协议状态机。通过 `tokio::select!` 多路复用同时监听网络入站 PDU 与 UI 端入站操作，
//! 实现双向零等待高吞吐响应，并控制单个循环轮次的时延。所有响应帧实时写回网络，
//! 损伤矩形实时提取并更新瓦片帧，远端光标动态转换为系统光标。

use crate::audio::AudioPlayer;
use crate::clipboard::ClipboardManager;
use crate::error::RdpError;
use crate::keyboard::KeyboardTranslator;
use crate::pointer::{CursorUpdate, create_custom_cursor};
use crate::session::channel::{MouseBtn, RdpInput, RdpOutput};
use crate::session::connector::ErasedStream;
use crate::settings::SessionConfig;
use crate::tiles::TiledDesktopFrame;
use ironrdp_connector::ConnectionResult;
use ironrdp_input::{
    Database as InputDatabase, MouseButton, MousePosition, Operation, WheelRotations,
};
use ironrdp_session::image::DecodedImage;
use ironrdp_session::{ActiveStage, ActiveStageBuilder, ActiveStageOutput};
use ironrdp_tokio::{FramedWrite, TokioFramed, split_tokio_framed};
use std::sync::Arc;
use tokio::sync::mpsc;

/// 会话循环主运行器。
pub struct SessionLoopRunner {
    framed: TokioFramed<ErasedStream>,
    active_stage: ActiveStage,
    canvas: DecodedImage,
    tiled_frame: TiledDesktopFrame,
    input_rx: mpsc::Receiver<RdpInput>,
    output_tx: mpsc::Sender<RdpOutput>,
    _audio_player: AudioPlayer,
    clipboard: ClipboardManager,
    keyboard: KeyboardTranslator,
    input_db: InputDatabase,
}

impl SessionLoopRunner {
    /// 构造新的主循环驱动器。
    pub fn new(
        framed: TokioFramed<ErasedStream>,
        conn_result: ConnectionResult,
        config: &SessionConfig,
        input_rx: mpsc::Receiver<RdpInput>,
        output_tx: mpsc::Sender<RdpOutput>,
    ) -> Self {
        let width = conn_result.desktop_size.width;
        let height = conn_result.desktop_size.height;

        let canvas = DecodedImage::new(
            ironrdp_graphics::image_processing::PixelFormat::BgrX32,
            width,
            height,
        );

        let active_stage = ActiveStageBuilder {
            static_channels: conn_result.static_channels,
            user_channel_id: conn_result.user_channel_id,
            io_channel_id: conn_result.io_channel_id,
            message_channel_id: conn_result.message_channel_id,
            share_id: conn_result.share_id,
            compression_type: None,
            enable_server_pointer: conn_result.enable_server_pointer,
            pointer_software_rendering: conn_result.pointer_software_rendering,
        }
        .build();

        let audio_player = AudioPlayer::new();
        if !config.audio {
            audio_player.set_muted(true);
        }

        Self {
            framed,
            active_stage,
            canvas,
            tiled_frame: TiledDesktopFrame::default(),
            input_rx,
            output_tx,
            _audio_player: audio_player,
            clipboard: ClipboardManager::new(),
            keyboard: KeyboardTranslator::default(),
            input_db: InputDatabase::new(),
        }
    }

    /// 执行会话事件循环直到被中止或网络断开。
    pub async fn run(self) {
        let width = self.canvas.width();
        let height = self.canvas.height();
        let _ = self
            .output_tx
            .send(RdpOutput::Connected { width, height })
            .await;

        let (mut reader, mut writer) = split_tokio_framed(self.framed);
        let mut input_rx = self.input_rx;
        let output_tx = self.output_tx;
        let mut keyboard = self.keyboard;
        let mut clipboard = self.clipboard;
        let mut input_db = self.input_db;
        let mut tiled_frame = self.tiled_frame;
        let mut active_stage = self.active_stage;
        let mut canvas = self.canvas;

        loop {
            tokio::select! {
                // 1. 处理来自界面的用户输入指令
                maybe_input = input_rx.recv() => {
                    let Some(input) = maybe_input else {
                        break;
                    };

                    if matches!(input, RdpInput::Disconnect) {
                        if let Ok(outputs) = active_stage.graceful_shutdown() {
                            let _ = Self::flush_network_responses(&mut writer, &outputs).await;
                        }
                        break;
                    }

                    if let RdpInput::ClipboardText(text) = input {
                        let _ = clipboard.update_local_text(text);
                        continue;
                    }

                    let ops = Self::convert_input_to_operations(input, &mut keyboard);
                    if ops.is_empty() {
                        continue;
                    }

                    let fastpath_events = input_db.apply(ops);
                    if !fastpath_events.is_empty() {
                        match active_stage.process_fastpath_input(&mut canvas, &fastpath_events) {
                            Ok(outputs) => {
                                if Self::flush_network_responses(&mut writer, &outputs).await.is_err() {
                                    break;
                                }
                                if Self::notify_ui_outputs(&outputs, &output_tx).await {
                                    break;
                                }
                            }
                            Err(e) => {
                                log::warn!("处理 FastPath 输入异常: {e}");
                            }
                        }
                    }
                }

                // 2. 处理来自网络链路的 RDP PDU 报文
                pdu_result = reader.read_pdu() => {
                    match pdu_result {
                        Ok((action, payload)) => {
                            match active_stage.process(&mut canvas, action, &payload, None) {
                                Ok(outputs) => {
                                    if Self::flush_network_responses(&mut writer, &outputs).await.is_err() {
                                        break;
                                    }
                                    if Self::notify_ui_outputs(&outputs, &output_tx).await {
                                        break;
                                    }

                                    // 提取最新损伤矩形并提交瓦片更新
                                    let damages = active_stage.take_damage_regions();
                                    if !damages.is_empty() {
                                        tiled_frame.update(&canvas, &damages);
                                        let frame_arc = Arc::new(tiled_frame.clone());
                                        let _ = output_tx.send(RdpOutput::Frame(frame_arc)).await;
                                    }
                                }
                                Err(e) => {
                                    log::error!("解码处理 RDP 报文失败: {e}");
                                }
                            }
                        }
                        Err(e) => {
                            log::info!("RDP 远端断开连接或发生 I/O 错误: {e}");
                            let _ = output_tx.send(RdpOutput::Terminated(RdpError::Network {
                                address: "远端连接".to_string(),
                                source: e,
                            })).await;
                            break;
                        }
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

    /// 将光标与状态机事件广播给 UI 呈现层。
    async fn notify_ui_outputs(
        outputs: &[ActiveStageOutput],
        output_tx: &mpsc::Sender<RdpOutput>,
    ) -> bool {
        for out in outputs {
            match out {
                ActiveStageOutput::PointerDefault => {
                    let _ = output_tx
                        .send(RdpOutput::Cursor(CursorUpdate::Default))
                        .await;
                }
                ActiveStageOutput::PointerHidden => {
                    let _ = output_tx
                        .send(RdpOutput::Cursor(CursorUpdate::Hidden))
                        .await;
                }
                ActiveStageOutput::PointerBitmap(ptr) => {
                    if let Some(cursor) = create_custom_cursor(
                        ptr.width,
                        ptr.height,
                        ptr.hotspot_x,
                        ptr.hotspot_y,
                        &ptr.bitmap_data,
                    ) {
                        let _ = output_tx.send(RdpOutput::Cursor(cursor)).await;
                    }
                }
                ActiveStageOutput::Terminate(reason) => {
                    log::info!("远端服务器发起终止会话: {reason:?}");
                    return true;
                }
                _ => {}
            }
        }
        false
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
}
