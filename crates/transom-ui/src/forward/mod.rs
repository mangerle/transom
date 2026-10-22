//! 端口转发面板与公网监听确认弹窗模块。

mod actions;
mod view;

use gpui_kit::component::input::InputState;
use gpui_kit::*;
use transom_core::{ConnectionId, ForwardRule, TabId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ForwardKindChoice {
    Local,
    Remote,
    Dynamic,
}

pub(crate) struct ForwardPanelState {
    pub(crate) name_input: Entity<InputState>,
    pub(crate) bind_host_input: Entity<InputState>,
    pub(crate) bind_port_input: Entity<InputState>,
    pub(crate) target_host_input: Entity<InputState>,
    pub(crate) target_port_input: Entity<InputState>,
    pub(crate) kind: ForwardKindChoice,
    pub(crate) error: Option<String>,
}

pub(crate) struct PendingForwardConfirm {
    pub(crate) tab_id: TabId,
    pub(crate) connection_id: ConnectionId,
    pub(crate) rule: ForwardRule,
}

pub(crate) fn is_public_bind_address(host: &str) -> bool {
    let host = host.trim();
    host == "0.0.0.0" || host == "::" || host == "0:0:0:0:0:0:0:0" || host == "*"
}
