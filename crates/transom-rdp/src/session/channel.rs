//! 会话输入输出有界通信通道与背压契约。
//!
//! 设计原理（Why）：严格遵循 AGENTS.md「完全禁用无界通道」红线。UI 交互线程与
//! RDP 协议循环之间通过固定容量（Bounded Channel）传递事件，显式定义背压流控，
//! 杜绝网络偶发停顿或重绘延迟导致的内存无限积压与 OOM。

use crate::clipboard::ClipboardEvent;
use crate::error::RdpError;
use crate::pointer::CursorUpdate;
use crate::tiles::TiledDesktopFrame;
use std::sync::Arc;
use tokio::sync::mpsc;

/// 输入事件通道容量上限（鼠标、按键与控制指令）。
pub const INPUT_CHANNEL_CAPACITY: usize = 128;

/// 输出帧与事件通道容量上限（图像帧、光标与状态）。
pub const OUTPUT_CHANNEL_CAPACITY: usize = 64;

/// 从 UI 界面发送给 RDP 会话线程的输入指令。
#[derive(Debug)]
pub enum RdpInput {
    /// 鼠标移动。
    MouseMove { x: u16, y: u16 },
    /// 鼠标按键。
    MouseButton { button: MouseBtn, pressed: bool },
    /// 鼠标滚轮滚动（垂直/水平）。
    MouseWheel { vertical: bool, delta: i16 },
    /// 键盘按键按下。
    KeyDown { key: String },
    /// 键盘按键抬起。
    KeyUp { key: String },
    /// 释放全部按键。
    ReleaseAllKeys,
    /// 请求动态重设桌面分辨率。
    Resize { width: u16, height: u16 },
    /// 设置音频静音状态。
    MuteAudio(bool),
    /// 本地剪贴板文本更新。
    ClipboardText(String),
    /// 请求断开会话并退出。
    Disconnect,
}

/// 鼠标按键枚举。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseBtn {
    Left,
    Middle,
    Right,
    X1,
    X2,
}

/// 从 RDP 会话循环发往 UI 界面层的输出通知。
pub enum RdpOutput {
    /// 会话连接成功，协商完成的分辨率。
    Connected { width: u16, height: u16 },
    /// 桌面瓦片图像帧增量更新。
    Frame(Arc<TiledDesktopFrame>),
    /// 远端光标形状更新。
    Cursor(CursorUpdate),
    /// 剪贴板事件。
    Clipboard(ClipboardEvent),
    /// 会话遇到错误已终止。
    Terminated(RdpError),
}

/// 输入通道发送/接收对。
pub type InputChannelPair = (mpsc::Sender<RdpInput>, mpsc::Receiver<RdpInput>);

/// 输出通道发送/接收对。
pub type OutputChannelPair = (mpsc::Sender<RdpOutput>, mpsc::Receiver<RdpOutput>);

/// 创建配对的有界通信通道。
pub fn create_session_channels() -> (InputChannelPair, OutputChannelPair) {
    let input_ch = mpsc::channel(INPUT_CHANNEL_CAPACITY);
    let output_ch = mpsc::channel(OUTPUT_CHANNEL_CAPACITY);
    (input_ch, output_ch)
}
