//! 容器引擎接入层：Docker/Podman 的发现、REST 客户端、exec 桥与归档文件。
//!
//! 设计原理（Why）：协议、平台通道与业务层解耦——HTTP 客户端跑在任意
//! 双向流上，平台差异（Unix socket/命名管道/stdio/CLI）收敛在 transport 与
//! discovery；控制器只面对 [`ContainerService`] 与 [`Engine`]。

mod api;
mod bridge;
mod cli;
pub mod compose;
mod discovery;
mod engine;
mod error;
mod exec;
mod files;
mod http;
mod logs;
mod models;
mod service;
mod stats;
mod transport;

pub use discovery::{find_in_path, parse_endpoint, validate_endpoint};
pub use engine::{Engine, discover_all, probe_explicit};
pub use error::ContainerError;
pub use exec::open_exec;
pub use files::{
    create_dir, download, list_dir, read_file, remove as remove_path, rename as rename_path,
    upload, validate_container_path,
};
pub use logs::{LogSink, stream_logs};
pub use service::ContainerService;
