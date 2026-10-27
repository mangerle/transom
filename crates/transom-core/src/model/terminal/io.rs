//! 独立于传输层的终端控制句柄与 IO 流模型。

use serde::{Deserialize, Serialize};
use std::fmt;
use std::io::{Read, Write};
use tokio::sync::mpsc::{Receiver, Sender};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalSize {
    pub cols: u16,
    pub rows: u16,
}

impl TerminalSize {
    pub const fn new(cols: u16, rows: u16) -> Self {
        Self { cols, rows }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalControl {
    Resize(TerminalSize),
    Close,
}

#[derive(Clone)]
pub struct TerminalControlHandle {
    sender: Sender<TerminalControl>,
}

impl TerminalControlHandle {
    pub fn channel(capacity: usize) -> (Self, Receiver<TerminalControl>) {
        let (sender, receiver) = tokio::sync::mpsc::channel(capacity.max(1));
        (Self { sender }, receiver)
    }

    /// 同步投递控制消息：快路径无锁 try_send，缓冲满（连续拖拽改尺寸）
    /// 时退化为阻塞反压，与此前 flume 的语义一致。调用方均在 UI 或专用
    /// 线程上下文，不会进入异步运行时，blocking_send 的运行时限制不触发。
    pub fn send(&self, control: TerminalControl) -> bool {
        match self.sender.try_send(control) {
            Ok(()) => true,
            Err(tokio::sync::mpsc::error::TrySendError::Full(control)) => {
                self.sender.blocking_send(control).is_ok()
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => false,
        }
    }

    pub fn resize(&self, size: TerminalSize) -> bool {
        self.send(TerminalControl::Resize(size))
    }

    pub fn close(&self) -> bool {
        self.send(TerminalControl::Close)
    }
}

impl fmt::Debug for TerminalControlHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TerminalControlHandle")
            .finish_non_exhaustive()
    }
}

/// A transport-independent terminal byte stream.
///
/// The terminal widget owns `reader`/`writer`; the transport keeps the receiving
/// half of `control` and reacts to resize and close requests.
pub struct TerminalIo {
    pub reader: Box<dyn Read + Send>,
    pub writer: Box<dyn Write + Send>,
    pub control: TerminalControlHandle,
}

impl TerminalIo {
    pub fn new(
        reader: Box<dyn Read + Send>,
        writer: Box<dyn Write + Send>,
        control: TerminalControlHandle,
    ) -> Self {
        Self {
            reader,
            writer,
            control,
        }
    }
}

impl fmt::Debug for TerminalIo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TerminalIo")
            .field("control", &self.control)
            .finish_non_exhaustive()
    }
}
