//! Transom RDP（远程桌面协议）原生客户端后端。
//!
//! 设计原理（Why）：基于 IronRDP 细粒度协议状态机、256x256 纯像素增量瓦片提取引擎、
//! 本地音频直通与双向剪贴板。协议层与 GUI 框架彻底解耦，仅向上层输出纯像素补丁与强类型事件。
//!
//! 核心架构组成：
//! - [`session`]：独立操作系统线程 + current-thread 运行时的会话驱动器与取消停机机制；
//! - [`tiles`]：128x128 纯 RGBA8 增量瓦片补丁提取器；
//! - [`keyboard`]：逻辑按键到 PC XT 扫描码的无堆分配按键映射；
//! - [`audio`]：RDPSND 远端音频流与本地 cpal 静音直通后端；
//! - [`clipboard`]：双向 UTF-16LE 文本剪贴板与沙箱路径防穿越校验；
//! - [`error`]：强类型且语义详尽的领域错误定义。

pub mod audio;
pub mod clipboard;
pub mod error;
pub mod keyboard;
pub mod session;
pub mod settings;
pub mod tiles;
pub mod transport;

pub use audio::TransomAudioBackend;
pub use clipboard::ClipboardEvent;
pub use error::RdpError;
pub use keyboard::{KeyChord, ModifierState};
pub use session::resize::{MIN_DESKTOP_HEIGHT, MIN_DESKTOP_WIDTH};
pub use session::{MouseBtn, RdpInput, RdpOutput, RdpReceivers, RdpSession};
pub use settings::{SessionConfig, TlsCertVerifier, TlsCertificateInfo, TlsVerifyOutcome};
pub use tiles::{FrameUpdate, TILE_SIZE, TileBufferPool, TilePatch};

#[cfg(test)]
mod tests;
