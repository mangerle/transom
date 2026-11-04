//! 容器引擎接入层：Docker/Podman 的发现、REST 客户端、exec 桥与归档文件。
//!
//! 设计原理（Why）：协议、平台通道与业务层解耦——HTTP 客户端跑在任意
//! 双向流上，平台差异（Unix socket/命名管道/stdio/CLI）收敛在 transport 与
//! discovery；控制器只面对 [`ContainerService`] 与 [`Engine`]。

mod api;
pub mod auth;
pub mod build;
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
mod tls;
mod transport;
pub mod volume_files;

pub use discovery::{find_in_path, parse_endpoint, validate_endpoint};
pub use engine::{Engine, discover_all, probe_explicit};
pub use error::ContainerError;
pub use exec::open_exec;
pub use files::{
    create_dir, download, list_dir, read_file, remove as remove_path, rename as rename_path,
    upload, validate_container_path, write_file_bytes,
};
pub use logs::{LogSink, stream_logs, stream_project_logs};
pub use service::ContainerService;
pub use volume_files::{
    download_volume_file, list_volume_dir, read_volume_file, write_volume_file_bytes,
};
