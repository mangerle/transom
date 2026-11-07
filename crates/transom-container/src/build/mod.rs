//! 镜像构建：流式上下文打包、.dockerignore 过滤与 HTTP/CLI 两条执行路径。
//!
//! 设计原理（Why）：上下文边遍历边打包并通过 chunked 请求上传，避免整项目
//! 进内存；忽略规则在打包前编译，遍历时直接跳过；取消标志贯穿打包与传输。

mod cli;
mod http;
mod ignore;
mod stream;

pub use cli::build_image_cli;
pub use http::build_image_http;
