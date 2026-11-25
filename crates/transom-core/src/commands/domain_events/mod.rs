//! 应用分领域事件枚举定义。
//!
//! 事件按业务子系统拆分到独立模块，父模块只负责聚合重导出。这样单个事件
//! 契约的增删都落在对应领域文件内，避免所有协议都堆进一个超过维护阈值的
//! 巨型枚举文件，同时保持 `transom_core::commands` 的公开路径稳定。
//!
//! 当前分支：
//! - [`terminal`]：终端连接、认证提示与未知主机确认
//! - [`sftp`]：文件列表、传输、预览、文本编辑与目录比较
//! - [`forward`]：端口转发状态与探测结果
//! - [`host`]：主机采样、拨测、公钥部署与进程动作
//! - [`sync`]：云端备份与 Git 同步
//! - [`zmodem`]：ZMODEM 传输协议事件
//! - [`container`]：容器引擎、资源管理、日志、统计与 exec 事件
//! - [`deep_link`]：外部唤起请求

mod container;
mod deep_link;
mod forward;
mod host;
mod sftp;
mod sync;
mod terminal;
mod zmodem;

pub use container::ContainerAppEvent;
pub use deep_link::DeepLinkRequest;
pub use forward::ForwardAppEvent;
pub use host::HostAppEvent;
pub use sftp::SftpAppEvent;
pub use sync::SyncAppEvent;
pub use terminal::TerminalAppEvent;
pub use zmodem::ZmodemAppEvent;
