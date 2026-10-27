//! 端口转发面板与公网监听确认弹窗模块。

mod actions;
mod view;

use gpui_kit::component::input::InputState;
use gpui_kit::*;
use transom_core::{ConnectionId, ForwardKind, ForwardRule, TabId};

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

/// 转发规则的监听地址；未知类型返回空串（不会命中公网判定）。
pub(crate) fn forward_bind_host(rule: &ForwardRule) -> &str {
    match &rule.kind {
        ForwardKind::Local { bind, .. }
        | ForwardKind::Remote { bind, .. }
        | ForwardKind::Dynamic { bind, .. } => bind.host.as_str(),
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::{forward_bind_host, is_public_bind_address};
    use transom_core::{BindAddress, ForwardKind, ForwardRule, SocksOptions, TargetAddress};

    fn rule(kind: ForwardKind) -> ForwardRule {
        ForwardRule::new("test", kind)
    }

    #[test]
    fn public_bind_detection_covers_all_forward_kinds() {
        let target = TargetAddress {
            host: "db.internal".into(),
            port: 5432,
        };
        let public = BindAddress {
            host: "0.0.0.0".into(),
            port: 8080,
        };
        let loopback = BindAddress::local(8080);
        for rule in [
            rule(ForwardKind::Local {
                bind: public.clone(),
                target: target.clone(),
            }),
            rule(ForwardKind::Remote {
                bind: public.clone(),
                target: target.clone(),
            }),
            rule(ForwardKind::Dynamic {
                bind: public.clone(),
                socks: SocksOptions::default(),
            }),
        ] {
            assert!(is_public_bind_address(forward_bind_host(&rule)));
        }
        for rule in [
            rule(ForwardKind::Local {
                bind: loopback.clone(),
                target: target.clone(),
            }),
            rule(ForwardKind::Remote {
                bind: loopback.clone(),
                target: target.clone(),
            }),
            rule(ForwardKind::Dynamic {
                bind: loopback.clone(),
                socks: SocksOptions::default(),
            }),
        ] {
            assert!(!is_public_bind_address(forward_bind_host(&rule)));
        }
    }
}
