//! RDP 桌面视口核心组件与事件处理。
//!
//! 设计原理（Why）：作为 GPUI 自绘树中的一等公民视图，`RdpView` 将来自后台会话
//! 线程的增量瓦片帧（`TiledDesktopFrame`）拼装呈现，并捕获键盘与鼠标操作直接
//! 送回会话输入通道。

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::*;
use std::sync::Arc;
use transom_core::SessionId;
use transom_rdp::{RdpOutput, RdpSession, SessionConfig, TiledDesktopFrame};

use super::toolbar::{RdpToolbarParams, render_rdp_toolbar};

/// 会话连接显示状态。
#[derive(Debug, Clone)]
pub enum RdpViewState {
    Connecting,
    Connected { width: u16, height: u16 },
    Disconnected,
    Failed(String),
}

/// RDP 远程桌面视口视图。
pub struct RdpView {
    pub session_id: SessionId,
    pub title: String,
    state: RdpViewState,
    frame: Option<Arc<TiledDesktopFrame>>,
    session: Option<RdpSession>,
    is_muted: bool,
    is_scaled: bool,
    focus_handle: FocusHandle,
}

impl RdpView {
    /// 构造新的 RDP 桌面视图。
    pub fn new(
        session_id: SessionId,
        title: String,
        config: SessionConfig,
        cx: &mut Context<Self>,
    ) -> Self {
        let (session, mut output_rx) = RdpSession::connect(config);

        // 监听来自后台会话线程的输出通知
        cx.spawn(async move |this, cx| {
            while let Some(output) = output_rx.recv().await {
                let res = this.update(cx, |view, cx| {
                    match output {
                        RdpOutput::Connected { width, height } => {
                            view.state = RdpViewState::Connected { width, height };
                        }
                        RdpOutput::Frame(f) => {
                            view.frame = Some(f);
                        }
                        RdpOutput::Terminated(e) => {
                            view.state = RdpViewState::Failed(e.to_string());
                        }
                        _ => {}
                    }
                    cx.notify();
                });
                if res.is_err() {
                    break;
                }
            }
        })
        .detach();

        Self {
            session_id,
            title,
            state: RdpViewState::Connecting,
            frame: None,
            session: Some(session),
            is_muted: false,
            is_scaled: true,
            focus_handle: cx.focus_handle(),
        }
    }

    /// 发送断开请求。
    pub fn disconnect(&mut self) {
        if let Some(session) = &self.session {
            session.disconnect();
        }
        self.state = RdpViewState::Disconnected;
    }
}

impl Focusable for RdpView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for RdpView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let mut root = div()
            .track_focus(&self.focus_handle)
            .size_full()
            .relative()
            .bg(gpui_kit::rgb(0x101010))
            .overflow_hidden();

        match &self.state {
            RdpViewState::Connecting => {
                root = root.child(
                    v_flex()
                        .size_full()
                        .items_center()
                        .justify_center()
                        .text_color(gpui_kit::rgb(0xaaaaaa))
                        .child("正在建立远程桌面连接..."),
                );
            }
            RdpViewState::Failed(msg) => {
                root = root.child(
                    v_flex()
                        .size_full()
                        .items_center()
                        .justify_center()
                        .gap(px(8.0))
                        .text_color(gpui_kit::rgb(0xff5555))
                        .child(format!("远程桌面连接失败: {msg}")),
                );
            }
            RdpViewState::Disconnected => {
                root = root.child(
                    v_flex()
                        .size_full()
                        .items_center()
                        .justify_center()
                        .text_color(gpui_kit::rgb(0x888888))
                        .child("远程桌面会话已断开"),
                );
            }
            RdpViewState::Connected { .. } => {
                // 渲染桌面瓦片集合
                if let Some(frame) = &self.frame {
                    let mut grid = v_flex().flex_none();
                    let cols = frame.cols as usize;
                    let rows = frame.rows as usize;

                    for r in 0..rows {
                        let mut row = h_flex().flex_none();
                        for c in 0..cols {
                            let idx = r * cols + c;
                            if let Some(tile_img) = frame.tiles.get(idx) {
                                row = row
                                    .child(img(ImageSource::Render(tile_img.clone())).flex_none());
                            }
                        }
                        grid = grid.child(row);
                    }
                    root = root.child(grid);
                }

                // 悬浮工具条
                let toolbar = render_rdp_toolbar(RdpToolbarParams {
                    is_muted: self.is_muted,
                    is_scaled: self.is_scaled,
                    on_send_cad: Box::new(|_, _, _| {}),
                    on_toggle_mute: Box::new(|_, _, _| {}),
                    on_toggle_scale: Box::new(|_, _, _| {}),
                    on_disconnect: Box::new(|_, _, _| {}),
                });

                root = root.child(
                    div()
                        .absolute()
                        .top(px(8.0))
                        .left_0()
                        .right_0()
                        .flex()
                        .justify_center()
                        .child(toolbar),
                );
            }
        }

        root
    }
}
