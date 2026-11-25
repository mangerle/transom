//! 工作区标签的公共元数据与类型化内容。
//!
//! 设计原理（Why）：终端会话、录像播放器、主机监控、容器工作区和 Git 工作区
//! 共享标签栏、分组、拖拽与并排能力，但只有终端会话拥有连接、工作目录和终端
//! 窗格。把这些会话字段留在公共标签上会迫使非会话标签伪造 `Session`，并在
//! 连接、SFTP、转发和监控路径中不断增加排除条件。按内容类型拆分后，会话能力
//! 由类型系统直接限定在 `SessionTab` 中。
//!
//! 代价与局限：调用方需要先按标签类型取得专用状态，不能再直接访问统一的
//! `tab.session` 字段；标签组也需要同时支持会话归属与单工作区归属。

mod accessors;
mod api_client;
mod container;
mod git;
mod group;
mod model;
mod monitor;
mod player;
pub(crate) mod presentation;
mod rdp;
mod session;
#[cfg(test)]
mod tests;

pub(crate) use api_client::{ApiClientTab, ApiHomeTab};
pub(crate) use container::ContainerTab;
pub(crate) use git::GitTab;
pub(crate) use group::{TabGroup, TabGroupOwner};
pub(crate) use model::{SessionMenuData, TabCloseAction, TabContent, TabStatus, WorkspaceTab};
pub(crate) use monitor::MonitorTab;
pub(crate) use player::PlayerTab;
pub(crate) use rdp::RdpTab;
pub(crate) use session::{SessionPaneState, SessionTab, TabProgress};
