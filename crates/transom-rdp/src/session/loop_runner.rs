//! RDP 数据会话主循环与事件驱动。
//!
//! 设计原理（Why）：会话循环运行在独立 OS 线程的 current-thread 运行时中，驱动 IronRDP
//! 的 `ActiveStage` 协议状态机：
//! 1. **100% 取消安全的 PDU 读取**：内层循环通过 `tokio::pin!` 固定 `reader.read_pdu()`，
//!    处理用户输入、帧刷新或防抖定时器时绝不中途丢弃正在读取的网络 Future；
//! 2. **固定空间脏瓦片合并与无反压帧投递**：由 [`TileExtractor`] 在恒定大小位图上合并损伤，
//!    视频帧通道满时保留脏位图延后合并，绝不反压阻塞网络读写与用户输入；
//! 3. **输入批处理委托**：由 `InputBatcher` 批量合并相邻 FastPath 事件并复用操作缓冲；
//! 4. **尾沿防抖动态分辨率与完整重激活闭环**：由 `ResizeController` 保证最终窗口尺寸
//!    偶数对齐且不丢尾，收到 `DeactivateAll` 后支持取消与失败上报。

mod cycle;
mod pdu;

use crate::session::channel::{RdpInput, RdpOutput, SessionSenders};
use crate::session::connector::{ErasedStream, EstablishedConnection};
use crate::session::input::InputBatcher;
use crate::session::output::LoopAction;
use crate::session::resize::ResizeController;
use crate::settings::DEFAULT_COMPRESSION_TYPE;
use crate::tiles::{TileBufferPool, TileExtractor};
use ironrdp_connector::connection_activation::ConnectionActivationFactory;
use ironrdp_graphics::image_processing::PixelFormat;
use ironrdp_session::image::DecodedImage;
use ironrdp_session::{ActiveStage, ActiveStageBuilder};
use ironrdp_tokio::{TokioFramed, split_tokio_framed};
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
            latest_rtt_ms: Arc::new(AtomicU32::new(initial_rtt_ms.unwrap_or(NO_RTT_SENTINEL))),
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
}
