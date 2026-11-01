//! 端口转发面板实体：持有面板状态，负责自身渲染与规则表单校验。
//!
//! # 设计原理（Why）
//! - 转发面板随窗格（连接）生死，与 SFTP 面板同形，因此每个持有转发的窗格一份
//!   `Entity<ForwardView>`，由窗格强持有、实体弱引用回指外壳；
//! - 规则表单的读取与校验属于面板内部：输入框是面板自己的子实体，校验失败的错误
//!   提示写在面板上。外壳只接收「校验通过后的 `ForwardRule`」，再去做发命令、
//!   写会话目录这些跨域动作；
//! - 这样面板的点击回调不会在外壳与实体之间来回重入，规避 GPUI 的双重租借。
//!
//! # 代价与局限
//! 渲染时需要会话规则与转发状态，这两份数据在外壳里，实体通过弱引用只读获取；
//! 弱引用失效（窗口关闭中）时渲染退化为空面板。

use super::ForwardPanelState;
use crate::app::AppShell;
use gpui_kit::*;

/// 转发面板实体。
pub(crate) struct ForwardView {
    /// 面板状态：五个输入框、转发类型与错误提示。
    pub(crate) state: ForwardPanelState,
    /// 回到外壳的弱引用；构造完成后由 `attach` 绑定。
    app: Option<WeakEntity<AppShell>>,
}

impl ForwardView {
    /// 用已建好的状态创建面板实体。
    pub(crate) fn new(state: ForwardPanelState) -> Self {
        Self { state, app: None }
    }

    /// 绑定外壳弱引用；创建后立即调用一次。
    pub(crate) fn attach(&mut self, app: WeakEntity<AppShell>) {
        self.app = Some(app);
    }

    /// 读取外壳（会话规则、转发状态、活动标签）。
    pub(super) fn with_shell<R>(
        &self,
        cx: &mut Context<Self>,
        action: impl FnOnce(&mut AppShell, &mut Context<AppShell>) -> R,
    ) -> Option<R> {
        let app = self.app.as_ref()?.upgrade()?;
        Some(app.update(cx, action))
    }
}

impl ForwardView {
    /// 读取表单并校验，返回可提交的转发规则。
    ///
    /// 校验失败时把错误写回面板并返回 `None`：错误的归属是面板，外壳只关心
    /// "能不能拿到一条规则"。
    pub(crate) fn build_rule(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<transom_core::ForwardRule> {
        use super::ForwardKindChoice;
        use transom_core::{BindAddress, ForwardKind, ForwardRule, SocksOptions, TargetAddress};

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
