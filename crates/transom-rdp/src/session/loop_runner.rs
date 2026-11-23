//! RDP 数据会话主循环与事件驱动。
//!
//! 设计原理（Why）：会话循环运行在独立的操作系统线程中，驱动带有 GAT 生命周期的
//! 异步状态机。通过 `tokio::select!` 多路复用同时监听网络入站 PDU 与 UI 端入站操作，
//! 实现双向零等待高吞吐响应，并控制单个循环轮次的时延。

use crate::audio::AudioPlayer;
use crate::clipboard::ClipboardManager;
use crate::error::RdpError;
use crate::graphics::GraphicsPipeline;
use crate::keyboard::KeyboardTranslator;
use crate::session::channel::{RdpInput, RdpOutput};
use crate::session::connector::ErasedStream;
use crate::settings::SessionConfig;
use crate::tiles::TiledDesktopFrame;
use ironrdp_connector::ConnectionResult;
use ironrdp_session::image::DecodedImage;
use ironrdp_session::{ActiveStage, ActiveStageBuilder};
use ironrdp_tokio::{TokioFramed, split_tokio_framed};
use std::sync::Arc;
use tokio::sync::mpsc;

/// 会话循环主运行器。
pub struct SessionLoopRunner {
    framed: TokioFramed<ErasedStream>,
    _active_stage: ActiveStage,
    canvas: DecodedImage,
    tiled_frame: TiledDesktopFrame,
    input_rx: mpsc::Receiver<RdpInput>,
    output_tx: mpsc::Sender<RdpOutput>,
    _audio_player: AudioPlayer,
    clipboard: ClipboardManager,
    keyboard: KeyboardTranslator,
    graphics: GraphicsPipeline,
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
            _active_stage: active_stage,
            canvas,
            tiled_frame: TiledDesktopFrame::default(),
            input_rx,
            output_tx,
            _audio_player: audio_player,
            clipboard: ClipboardManager::new(),
            keyboard: KeyboardTranslator::default(),
            graphics: GraphicsPipeline::new(width, height),
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

        let (mut reader, _writer) = split_tokio_framed(self.framed);
        let mut input_rx = self.input_rx;
        let output_tx = self.output_tx;
        let mut keyboard = self.keyboard;
        let mut clipboard = self.clipboard;
        let mut graphics = self.graphics;
        let mut tiled_frame = self.tiled_frame;
        let canvas = self.canvas;

        loop {
            tokio::select! {
                // 1. 处理来自界面的输入指令
                maybe_input = input_rx.recv() => {
                    match maybe_input {
                        Some(input) => {
                            match input {
                                RdpInput::Disconnect => break,
                                RdpInput::KeyDown { key } => {
                                    let _ = keyboard.handle_key_down(&key);
                                }
                                RdpInput::KeyUp { key } => {
                                    let _ = keyboard.handle_key_up(&key);
                                }
                                RdpInput::ReleaseAllKeys => {
                                    let _ = keyboard.release_all();
                                }
                                RdpInput::Resize { width, height } => {
                                    graphics.resize(width, height);
                                }
                                RdpInput::ClipboardText(text) => {
                                    let _ = clipboard.update_local_text(text);
                                }
                                _ => {}
                            }
                        }
                        None => break, // 输入端已关闭
                    }
                }

                // 2. 处理来自网络链路的 RDP PDU
                pdu_result = reader.read_pdu() => {
                    match pdu_result {
                        Ok((_action, _payload)) => {
                            if let Some(frame_event) = graphics.commit_frame() {
                                tiled_frame.update(&canvas, &frame_event.dirty_rectangles);
                                let frame_arc = Arc::new(tiled_frame.clone());
                                let _ = output_tx.send(RdpOutput::Frame(frame_arc)).await;
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
}
