//! 端口转发面板实体：持有面板状态，负责自身渲染与规则表单校验。
//!
//! # 设计原理（Why）
//! - 转发面板随窗格（连接）生死，与 SFTP 面板同形：每个持有转发的窗格一份
//!   `Entity<ForwardView>`，由窗格强持有；
//! - 面板**不持有外壳句柄**：会话规则与转发状态由外壳在变更时注入，面板渲染只读
//!   自己的状态；需要外壳做事（启停、删除、提交、关闭）时发事件，外壳订阅后执行。
//!   这样跨实体调用只有"数据向下、意图向上"两个方向，不会形成租借环；
//! - 规则表单的读取与校验属于面板内部：输入框是面板自己的子实体，校验失败的错误
//!   提示写在面板上。
//!
//! # 代价与局限
//! 外壳需要在规则或状态变化后主动推送一次；遗漏会表现为面板显示旧数据。

use super::{ForwardPanelEvent, ForwardPanelState};
use gpui_kit::*;
use std::collections::HashMap;
use transom_core::{ForwardId, ForwardRule, ForwardStatus, TabId};

/// 转发面板实体。
pub(crate) struct ForwardView {
    /// 面板状态：五个输入框、转发类型与错误提示。
    pub(crate) state: ForwardPanelState,
    /// 所属标签；事件按它寻址。
    tab_id: TabId,
    /// 会话规则快照：外壳在规则变化时注入。
    rules: Vec<ForwardRule>,
    /// 转发状态快照：外壳在状态变化时注入。
    statuses: HashMap<ForwardId, ForwardStatus>,
}

impl EventEmitter<ForwardPanelEvent> for ForwardView {}

impl ForwardView {
    /// 用已建好的状态创建面板实体。
    pub(crate) fn new(state: ForwardPanelState, tab_id: TabId) -> Self {
        Self {
            state,
            tab_id,
            rules: Vec::new(),
            statuses: HashMap::new(),
        }
    }

    /// 接收外壳推送的会话规则与转发状态。
    pub(crate) fn set_session_data(
        &mut self,
        rules: Vec<ForwardRule>,
        statuses: HashMap<ForwardId, ForwardStatus>,
    ) {
        self.rules = rules;
        self.statuses = statuses;
    }

    /// 面板所属标签。
    pub(crate) fn tab_id(&self) -> TabId {
        self.tab_id
    }

    /// 当前规则快照。
    pub(crate) fn rules(&self) -> &[ForwardRule] {
        &self.rules
    }

    /// 当前转发状态快照。
    pub(crate) fn statuses(&self) -> &HashMap<ForwardId, ForwardStatus> {
        &self.statuses
    }

    /// 读取表单并校验，返回可提交的转发规则。
    ///
    /// 校验失败时把错误写回面板并返回 `None`：错误归属是面板，外壳只关心能否拿到规则。
    pub(crate) fn build_rule(&mut self, cx: &mut Context<Self>) -> Option<ForwardRule> {
        use super::ForwardKindChoice;
        use transom_core::{BindAddress, ForwardKind, SocksOptions, TargetAddress};

        let name = self.state.name_input.read(cx).value().trim().to_string();
        let bind_host = self
            .state
            .bind_host_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let target_host = self
            .state
            .target_host_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let bind_port: u16 = match self.state.bind_port_input.read(cx).value().trim().parse() {
            Ok(port) if port > 0 => port,
            _ => {
                self.state.error = Some(rust_i18n::t!("forward.error_port").to_string());
                cx.notify();
                return None;
            }
        };
        let target_port: u16 = match self.state.target_port_input.read(cx).value().trim().parse() {
            Ok(port) if port > 0 => port,
            _ => {
                self.state.error = Some(rust_i18n::t!("forward.error_port").to_string());
                cx.notify();
                return None;
            }
        };
        if bind_host.is_empty() {
            self.state.error = Some(rust_i18n::t!("forward.bind_host").to_string());
            cx.notify();
            return None;
        }
        if self.state.kind != ForwardKindChoice::Dynamic && target_host.is_empty() {
            self.state.error = Some(rust_i18n::t!("forward.error_target").to_string());
            cx.notify();
            return None;
        }
        let kind = match self.state.kind {
            ForwardKindChoice::Local => ForwardKind::Local {
                bind: BindAddress {
                    host: bind_host.clone(),
                    port: bind_port,
                },
                target: TargetAddress {
                    host: target_host,
                    port: target_port,
                },
            },
            ForwardKindChoice::Remote => ForwardKind::Remote {
                bind: BindAddress {
                    host: bind_host.clone(),
                    port: bind_port,
                },
                target: TargetAddress {
                    host: target_host,
                    port: target_port,
                },
            },
            ForwardKindChoice::Dynamic => ForwardKind::Dynamic {
                bind: BindAddress {
                    host: bind_host.clone(),
                    port: bind_port,
                },
                socks: SocksOptions::default(),
            },
        };
        let name = if name.is_empty() {
            format!("{bind_host}:{bind_port}")
        } else {
            name
        };
        self.state.error = None;
        Some(ForwardRule::new(name, kind))
    }
}

impl Render for ForwardView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = crate::palette::Palette::from(cx);
        self.render_forward_panel(window, cx, &p)
    }
}
