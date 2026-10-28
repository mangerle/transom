//! 协议与同步：SSH 配置/URL 解析、代理、SFTP 契约、同步合并与采样解析。
//!
//! 设计原理（Why）：这些模块描述"与外界的线格式与对齐规则"（配置文本、
//! URL、传输元数据、同步文档、采样输出），与 UI 无关、可独立测试；与领域
//! 模型的"状态形状"分开放置后，两边的边界一眼可见。

pub mod proxy;
pub mod sftp;
pub mod sftp_filter;
pub mod sftp_sync;
pub mod ssh_config;
pub mod ssh_url;
pub mod sync;
pub mod system_stats;
pub mod traffic;
pub mod zmodem;
