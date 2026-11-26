//! 端口转发面板与公网监听确认弹窗模块。

/// 转发面板向外壳发出的意图。
#[derive(Clone)]
pub(crate) enum ForwardPanelEvent {
    /// 提交一条已通过校验的新转发规则。
    SubmitRule {
        tab_id: transom_core::TabId,
        rule: transom_core::ForwardRule,
    },
    /// 保存对既有规则的修改：保序替换，不改变规则 id。
    UpdateRule {
        tab_id: transom_core::TabId,
        rule: transom_core::ForwardRule,
    },
    /// 启动一条规则。
    StartRule {
        tab_id: transom_core::TabId,
        rule_id: transom_core::ForwardId,
    },
    /// 停止一条规则。
    StopRule {
        tab_id: transom_core::TabId,
        rule_id: transom_core::ForwardId,
    },
    /// 删除一条规则。
    DeleteRule {
        tab_id: transom_core::TabId,
        rule_id: transom_core::ForwardId,
    },
    /// 探测规则目标端口的连通性；结果一次性回流到规则卡片。
    ProbeRule {
        tab_id: transom_core::TabId,
        rule_id: transom_core::ForwardId,
    },
    /// 复制与本规则等价的 `ssh -N` 命令行。
    CopyCli {
        tab_id: transom_core::TabId,
        rule_id: transom_core::ForwardId,
    },
    /// 在系统浏览器打开 Local 转发的本机入口。
    OpenBrowser {
        tab_id: transom_core::TabId,
        rule_id: transom_core::ForwardId,
    },
    /// 关闭面板；句柄由外壳回收。
    ClosePanel,
}

mod actions;
mod confirm;
mod entity;
mod form;
mod inspect;
mod lifecycle;
mod row;
mod view;

pub(crate) use actions::{auto_start_forwards, on_toggle_forward, toggle_forward_panel};
pub(crate) use confirm::render_forward_confirm_body;
pub(crate) use entity::ForwardView;
pub(crate) use inspect::{copy_forward_cli, open_forward_browser, probe_forward_rule};
pub(crate) use lifecycle::{
    cancel_forward_confirm, confirm_forward_start, delete_forward_rule, start_forward_rule,
    stop_forward_rule, submit_forward_rule, update_forward_rule,
};

use gpui_kit::component::input::InputState;
use gpui_kit::*;
use transom_core::{ConnectionId, ForwardId, ForwardKind, ForwardRule, TabId};

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
    /// 正在编辑的规则 id；`None` 表示表单处于新增模式。
    pub(crate) editing_id: Option<ForwardId>,
    /// 表单输入框的回车提交订阅；随面板状态一同销毁。
    pub(crate) _subscriptions: Vec<Subscription>,
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
