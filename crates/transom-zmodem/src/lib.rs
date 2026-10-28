//! ZMODEM 文件传输协议实现（独立 crate）。
//!
//! 只依赖 `transom-core` 的终端 IO 契约与事件类型、`flume` 通道与 `log`
//! 门面；不感知 SSH/SFTP 或界面。协议可单独测试与复用，宿主 crate 只需
//! 把 `TerminalIo` 交给 [`attach_zmodem`]。
//!
//! 模块分层：
//! - [`frames`]：常量、CRC、ZDLE 转义与帧头/子包构造（纯函数）；
//! - [`scanner`]：字节流增量解析（帧头/子包事件化）；
//! - [`receiver`] / [`sender`]：收发双方的状态机（纯逻辑，I/O 由调用方执行）；
//! - [`files`]：文件读写执行层（状态机由此隔离系统调用）；
//! - [`fsm`]：会话状态机与驱动循环；
//! - [`attach`]：读写过滤层，并把上述部件装配到 `TerminalIo` 上。
//!
//! 全部字节级语义以 lrzsz 参考实现为准，关键分歧点（头字节序、ZDLE 转义
//! 范围、CRC 字序、rz 的 ZRINIT 轮询行为）已在各文件文档注释中定案。

mod attach;
mod files;
mod frames;
mod fsm;
mod receiver;
mod scanner;
mod sender;

pub use attach::{ZmodemConfig, ZmodemControl, attach_zmodem};
