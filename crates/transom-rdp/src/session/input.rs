//! RDP 会话输入批处理与 FastPath 转换器。
//!
//! 设计原理（Why）：将高频输入批处理、`InputDatabase` 状态维护与控制指令分流
//! 从 `loop_runner` 主事件循环中剥离，保持主循环简洁聚焦于网络与定时器调度；
//! 同时复用 `ops_buf` 避免每次键鼠事件产生堆分配。

use crate::keyboard::KeyboardTranslator;
use crate::session::channel::{MouseBtn, RdpInput};
use crate::session::cliprdr::handle_local_clipboard_copy;
use crate::session::resize::ResizeController;
use ironrdp_input::{
    Database as InputDatabase, MouseButton, MousePosition, Operation, WheelRotations,
};
use ironrdp_session::image::DecodedImage;
use ironrdp_session::{ActiveStage, ActiveStageOutput, SessionResult};
use ironrdp_tokio::FramedWrite;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use tokio::sync::mpsc;

/// 单次批处理从输入通道排空的最大事件数。
const MAX_INPUT_BATCH_SIZE: usize = 32;

/// 输入处理后的会话控制指令。
pub(crate) enum InputBatchOutcome {
    Continue,
    Stop,
}

/// 键盘、鼠标与控制输入批处理器。
pub(crate) struct InputBatcher {
    keyboard: KeyboardTranslator,
    input_db: InputDatabase,
    ops_buf: Vec<Operation>,
}

impl Default for InputBatcher {
    fn default() -> Self {
        Self {
            keyboard: KeyboardTranslator::default(),
            input_db: InputDatabase::new(),
            ops_buf: Vec::with_capacity(MAX_INPUT_BATCH_SIZE),
        }
    }
}

impl InputBatcher {
    /// 批量排空相邻 UI 输入指令，立即响应控制指令并将连续 FastPath 键鼠事件合并。
    /// 若存在待同步的最新鼠标坐标，优先注入队列头部，保证点击事件前坐标严格对齐。
    pub(crate) async fn process_batch<W: FramedWrite>(
        &mut self,
        pending_mouse: Option<(u16, u16)>,
        first: RdpInput,
        input_rx: &mut mpsc::Receiver<RdpInput>,
        ctx: &mut InputProcessContext<'_, W>,
    ) -> InputBatchOutcome {
        self.ops_buf.clear();
        if let Some((x, y)) = pending_mouse {
            self.ops_buf
                .push(Operation::MouseMove(MousePosition { x, y }));
        }
        if matches!(
            self.enqueue_or_handle_control(first, ctx).await,
            InputBatchOutcome::Stop
        ) {
            return InputBatchOutcome::Stop;
        }

        let mut drained = 1;
        while drained < MAX_INPUT_BATCH_SIZE {
            let Ok(next) = input_rx.try_recv() else {
                break;
            };
            drained += 1;
            if matches!(
                self.enqueue_or_handle_control(next, ctx).await,
                InputBatchOutcome::Stop
            ) {
                return InputBatchOutcome::Stop;
            }
        }

        InputBatchOutcome::Continue
    }

    /// 单独压入一次鼠标移动操作（用于鼠标覆盖槽位独立唤醒时）。
    pub(crate) fn prepare_mouse_move(&mut self, x: u16, y: u16) {
        self.ops_buf.clear();
        self.ops_buf
            .push(Operation::MouseMove(MousePosition { x, y }));
    }

    /// 将当前累积的键鼠操作转换为 FastPath PDU 并交由状态机编码（零多余 `Vec` 克隆）。
    pub(crate) fn flush_to_stage(
        &mut self,
        active_stage: &mut ActiveStage,
        canvas: &mut DecodedImage,
    ) -> Option<SessionResult<Vec<ActiveStageOutput>>> {
        if self.ops_buf.is_empty() {
            return None;
        }
        let fastpath_events = self.input_db.apply(self.ops_buf.drain(..));
        if fastpath_events.is_empty() {
            return None;
        }
        Some(active_stage.process_fastpath_input(canvas, &fastpath_events))
    }

    async fn enqueue_or_handle_control<W: FramedWrite>(
        &mut self,
        input: RdpInput,
        ctx: &mut InputProcessContext<'_, W>,
    ) -> InputBatchOutcome {
        match input {
            RdpInput::Disconnect => InputBatchOutcome::Stop,
            RdpInput::MuteAudio(muted) => {
                ctx.audio_muted.store(muted, Ordering::Relaxed);
                InputBatchOutcome::Continue
            }
            RdpInput::ClipboardText(text) => {
                if let Err(e) =
                    handle_local_clipboard_copy(ctx.active_stage, ctx.writer, text).await
                {
                    log::info!("写入 CLIPRDR 本地复制通告时连接中断: {e}");
                    return InputBatchOutcome::Stop;
                }
                InputBatchOutcome::Continue
            }
            RdpInput::Resize {
                width,
                height,
                scale_factor_percent,
            } => {
                ctx.resize_controller
                    .schedule(width, height, scale_factor_percent, Instant::now());
                InputBatchOutcome::Continue
            }
            fastpath => {
                append_fastpath_operations(fastpath, &mut self.keyboard, &mut self.ops_buf);
                InputBatchOutcome::Continue
            }
        }
    }
}

/// 输入批处理所需的外部上下文引用（参数对象模式收敛入参）。
pub(crate) struct InputProcessContext<'a, W> {
    pub(crate) active_stage: &'a mut ActiveStage,
    pub(crate) writer: &'a mut W,
    pub(crate) audio_muted: &'a AtomicBool,
    pub(crate) resize_controller: &'a mut ResizeController,
}

/// 将 UI 键鼠输入追加到复用的底层原子操作缓冲（避免每条事件堆分配 `Vec`）。
fn append_fastpath_operations(
    input: RdpInput,
    keyboard: &mut KeyboardTranslator,
    out: &mut Vec<Operation>,
) {
    match input {
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
            if let Some(Operation::WheelRotations(prev)) = out.last_mut()
                && prev.is_vertical == vertical
            {
                prev.rotation_units = prev.rotation_units.saturating_add(delta);
            } else {
                out.push(Operation::WheelRotations(WheelRotations {
                    is_vertical: vertical,
                    rotation_units: delta,
                }));
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::pin::Pin;
    use std::time::Instant;

    struct DummyWriter;

    impl FramedWrite for DummyWriter {
        type WriteAllFut<'a>
            = Pin<Box<dyn Future<Output = std::io::Result<()>> + 'a>>
        where
            Self: 'a;

        fn write_all<'a>(&'a mut self, _buf: &'a [u8]) -> Self::WriteAllFut<'a> {
            Box::pin(async { Ok(()) })
        }
    }

    #[tokio::test]
    async fn batcher_orders_pending_mouse_before_click_and_handles_controls() {
        let mut writer = DummyWriter;
        let audio_muted = AtomicBool::new(false);
        let mut resize_controller = ResizeController::new((1920, 1080), 100);
        let mut active_stage = ironrdp_session::ActiveStageBuilder {
            static_channels: ironrdp_svc::StaticChannelSet::new(),
            user_channel_id: 1001,
            io_channel_id: 1002,
            message_channel_id: Some(1003),
            share_id: 1,
            compression_type: None,
            enable_server_pointer: false,
            pointer_software_rendering: false,
        }
        .build();

        let (input_tx, mut input_rx) = mpsc::channel(16);
        input_tx
            .send(RdpInput::MuteAudio(true))
            .await
            .expect("发送静音指令");
        input_tx
            .send(RdpInput::Resize {
                width: 1281,
                height: 721,
                scale_factor_percent: 200,
            })
            .await
            .expect("发送分辨率指令");

        let mut batcher = InputBatcher::default();
        let mut ctx = InputProcessContext {
            active_stage: &mut active_stage,
            writer: &mut writer,
            audio_muted: &audio_muted,
            resize_controller: &mut resize_controller,
        };

        let outcome = batcher
            .process_batch(
                Some((320, 240)),
                RdpInput::MouseButton {
                    button: MouseBtn::Left,
                    pressed: true,
                },
                &mut input_rx,
                &mut ctx,
            )
            .await;

        assert!(matches!(outcome, InputBatchOutcome::Continue));
        assert!(audio_muted.load(Ordering::Relaxed));
        assert!(ctx.resize_controller.next_deadline().is_some());
        assert!(matches!(
            batcher.ops_buf.first(),
            Some(Operation::MouseMove(MousePosition { x: 320, y: 240 }))
        ));
        assert!(matches!(
            batcher.ops_buf.get(1),
            Some(Operation::MouseButtonPressed(MouseButton::Left))
        ));

        // 遇到 Disconnect 立即返回 Stop
        let stop_outcome = batcher
            .process_batch(None, RdpInput::Disconnect, &mut input_rx, &mut ctx)
            .await;
        assert!(matches!(stop_outcome, InputBatchOutcome::Stop));
        let _ = Instant::now();
    }
}
