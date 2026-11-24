//! 会话输入输出有界通信通道与背压契约。
//!
//! 设计原理（Why）：
//! 1. 严格遵循「完全禁用无界通道」红线，所有通道显式设置容量上限；
//! 2. 将低频控制事件通道（连接、分辨率变更、光标、剪贴板、断开）与高频桌面帧通道
//!    （[`FrameUpdate`]）物理隔离：控制事件绝不被海量视频帧淹没，视频帧通道满时
//!    后台保留损伤区域并在下一拍合并，绝不反压阻塞 RDP 网络协议主循环。

use crate::clipboard::ClipboardEvent;
use crate::error::RdpError;
use crate::keyboard::{KeyChord, ModifierState};
use crate::tiles::{FrameUpdate, TileBufferPool};
use tokio::sync::{mpsc, watch};

/// 输入事件通道容量上限（按键、鼠标按键与控制指令；高频鼠标移动走独立覆盖槽位）。
pub const INPUT_CHANNEL_CAPACITY: usize = 128;

/// 控制事件通道容量上限（连接状态、光标、剪贴板与终止通知）。
pub const OUTPUT_CHANNEL_CAPACITY: usize = 64;

/// 桌面视频帧增量通道容量上限（保持极低延迟与极小内存驻留）。
pub const FRAME_CHANNEL_CAPACITY: usize = 2;

/// 从 UI 界面发送给 RDP 会话线程的输入指令（不含高频可覆盖的鼠标移动）。
#[derive(Debug)]
pub enum RdpInput {
    /// 鼠标按键。
    MouseButton { button: MouseBtn, pressed: bool },
    /// 鼠标滚轮滚动（垂直/水平）。
    MouseWheel { vertical: bool, delta: i16 },
    /// 键盘按键按下（附带当前修饰键快照与可选 Unicode 字符）。
    KeyDown {
        key: String,
        key_char: Option<String>,
        modifiers: ModifierState,
    },
    /// 键盘按键抬起。
    KeyUp {
        key: String,
        key_char: Option<String>,
    },
    /// 独立修饰键状态变更（同步单独按下/抬起 Ctrl、Alt、Shift、Win）。
    ModifiersChanged(ModifierState),
    /// 原子发送预设组合键（如 Ctrl+Alt+Del、Win），杜绝通道满时半截卡键。
    SendKeyChord(KeyChord),
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

/// 从 RDP 会话循环发往 UI 界面层的控制事件通知。
#[derive(Debug)]
pub enum RdpOutput {
    /// 会话连接成功，协商完成的初始分辨率。
    Connected { width: u16, height: u16 },
    /// 远端桌面分辨率已通过重激活序列完成变更。
    Resized { width: u16, height: u16 },
    /// 剪贴板事件。
    Clipboard(ClipboardEvent),
    /// 远端或本地正常断开会话。
    Disconnected { reason: String },
    /// 会话遇到错误已终止。
    Terminated(RdpError),
}

/// 交付给 UI 层的双轨接收端句柄（控制事件与视频帧物理隔离）。
pub struct RdpReceivers {
    /// 低频控制事件接收端。
    pub events: mpsc::Receiver<RdpOutput>,
    /// 高频增量瓦片帧接收端。
    pub frames: mpsc::Receiver<FrameUpdate>,
    /// 跨线程共享的像素缓冲池（用于 UI 侧覆盖未上传补丁时归还 `Vec<u8>`）。
    pub buffer_pool: TileBufferPool,
}

/// 内部会话通道发送端集合。
pub(crate) struct SessionSenders {
    pub(crate) events: mpsc::Sender<RdpOutput>,
    pub(crate) frames: mpsc::Sender<FrameUpdate>,
}

/// 内部会话通道集合。
pub(crate) struct SessionChannels {
    pub(crate) input_tx: mpsc::Sender<RdpInput>,
    pub(crate) input_rx: mpsc::Receiver<RdpInput>,
    pub(crate) mouse_tx: watch::Sender<Option<(u16, u16)>>,
    pub(crate) mouse_rx: watch::Receiver<Option<(u16, u16)>>,
    pub(crate) senders: SessionSenders,
    pub(crate) receivers: RdpReceivers,
}

/// 创建配对的有界通信通道。
pub(crate) fn create_session_channels() -> SessionChannels {
    let (input_tx, input_rx) = mpsc::channel(INPUT_CHANNEL_CAPACITY);
    let (mouse_tx, mouse_rx) = watch::channel(None);
    let (events_tx, events_rx) = mpsc::channel(OUTPUT_CHANNEL_CAPACITY);
    let (frames_tx, frames_rx) = mpsc::channel(FRAME_CHANNEL_CAPACITY);

    SessionChannels {
        input_tx,
        input_rx,
        mouse_tx,
        mouse_rx,
        senders: SessionSenders {
            events: events_tx,
            frames: frames_tx,
        },
        receivers: RdpReceivers {
            events: events_rx,
            frames: frames_rx,
            buffer_pool: TileBufferPool::default(),
        },
    }
}
