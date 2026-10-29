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

/// 会话级清理动作：随会话资源释放时执行一次。
///
/// 设计原理（Why）：会话建立过程中产生的临时资源（本地 Shell 预装用的临时 rcfile
/// 目录等）必须活到 Shell 读完启动文件为止，又不能留在磁盘上等系统清理；
/// 把清理动作挂在会话句柄上，由 RAII 保证"会话结束即回收"。
pub struct SessionCleanup {
    action: Option<Box<dyn FnOnce() + Send>>,
}

impl SessionCleanup {
    /// 登记一个清理动作；`None` 表示本次会话不需要清理。
    pub fn new(action: impl FnOnce() + Send + 'static) -> Self {
        Self {
            action: Some(Box::new(action)),
        }
    }

    /// 不做任何清理的空守卫。
    pub fn none() -> Self {
        Self { action: None }
    }
}

impl Drop for SessionCleanup {
    fn drop(&mut self) {
        if let Some(action) = self.action.take() {
            action();
        }
    }
}

/// 与具体传输方式无关的终端字节流。
///
/// 终端视图独占 `reader`/`writer` 的消费与生产；传输端持有 `control` 的
/// 接收半端，响应尺寸调整与关闭请求。
pub struct TerminalIo {
    pub reader: Box<dyn Read + Send>,
    pub writer: Box<dyn Write + Send>,
    pub control: TerminalControlHandle,
    /// 集成脚本是否已在会话启动阶段预装（如本地 Shell 经 rcfile 注入），以及是哪个方言。
    ///
    /// 为 `Some` 时终端侧不再自行探测与注入：探针命令本身也会被终端回显，
    /// 本地会话要做到零可见，就不能对终端有任何写入动作。
    pub integration_preinstalled: Option<crate::protocol::shell_integration::ShellKind>,
    /// 会话级清理动作：随本会话资源释放时执行（如本地 Shell 预装用的临时目录）。
    pub cleanup: SessionCleanup,
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
            integration_preinstalled: None,
            cleanup: SessionCleanup::none(),
        }
    }

    /// 标记集成脚本已在会话启动阶段预装，终端侧不再自行探测与注入。
    pub fn with_preinstalled_integration(
        mut self,
        kind: crate::protocol::shell_integration::ShellKind,
    ) -> Self {
        self.integration_preinstalled = Some(kind);
        self
    }
}

impl fmt::Debug for TerminalIo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TerminalIo")
            .field("control", &self.control)
            .finish_non_exhaustive()
    }
}
