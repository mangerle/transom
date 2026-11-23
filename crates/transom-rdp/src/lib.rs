//! Transom RDP（远程桌面协议）原生客户端后端。
//!
//! 设计原理（Why）：基于 IronRDP 细粒度协议状态机、256x256 瓦片增量渲染引擎、
//! 本地音频直通与受控剪贴板文件流。
//!
//! 核心架构组成：
//! - [`session`]：独立操作系统线程 + current-thread 运行时的会话驱动器；
//! - [`tiles`]：256x256 瓦片网格与增量损伤 GPU 纹理常驻缓存；
//! - [`pointer`]：远端光标位图解码与 GPUI 原生系统光标适配；
//! - [`keyboard`]：GPUI 逻辑按键到 PC XT 扫描码的按键映射；
//! - [`audio`]：RDPSND / DVC 远端音频流与本地 cpal 播放后端；
//! - [`clipboard`]：双向 UTF-16LE 文本剪贴板与沙箱文件流（严格路径防穿越）；
//! - [`error`]：强类型且语义详尽的领域错误定义。

pub mod audio;
pub mod clipboard;
pub mod error;
pub mod graphics;
pub mod keyboard;
pub mod pointer;
pub mod session;
pub mod settings;
pub mod tiles;
pub mod transport;

pub use audio::AudioPlayer;
pub use clipboard::{ClipboardEvent, FileTransferEvent};
pub use error::RdpError;
pub use pointer::CursorUpdate;
pub use session::{MouseBtn, RdpInput, RdpOutput, RdpSession};
pub use settings::SessionConfig;
pub use tiles::TiledDesktopFrame;

#[cfg(test)]
mod tests;
