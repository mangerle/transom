//! 主工作区标签栏、分屏窗格、断线重连提示与底部硬件状态监控栏渲染模块。

mod empty_state;
mod pane_header;
mod panes;
pub(crate) mod render;
pub(crate) mod state;
mod tab_drag;
pub(crate) mod tab_selection;
mod tab_strip;
pub(crate) mod tabs;
mod tiled;
mod tiling_menu;
mod vertical_tabs;

pub use self::state::WorkspaceState;

use crate::app::app_settings;
use crate::app::tab::TabProgress;
use crate::app::{AppShell, TabStatus};
use crate::palette::{Palette, localized_error};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::*;
use transom_core::{TabId, TabPosition};
use transom_terminal::ProgressState;
use transom_ui_icons::AppIcon;
use transom_ui_icons::DomainIcon;
use transom_ui_icons::UiIcon;

impl AppShell {
    pub(crate) fn render_main(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let disconnected_tab = self.state.workspace.active_tab.and_then(|tab_id| {
            self.state
                .workspace
                .tabs
                .iter()
                .find(|tab| tab.id == tab_id && matches!(&tab.status, TabStatus::Disconnected))
                .map(|tab| tab.id)
        });
        let content = self.render_active_content(window, cx, p);

        // 标签位置为左侧时，主区始终不绘制顶部横向条：切换标签只走活动栏
        // 的"标签"条目与快捷键，避免打开会话树时又冒出一条横向标签栏。
        let mut main = v_flex()
            .flex_1()
            .h_full()
            .overflow_hidden()
            .bg(p.background);
        if !self.state.ui.immersive && app_settings(cx).window.tab_position != TabPosition::Left {
            main = main.child(self.render_tab_strip(window, p, cx));
        }
        if let Some(tab_id) = disconnected_tab {
            main = main.child(self.render_reconnect_banner(tab_id, p, cx));
        }
        main = main.child(content);
        // 广播条贴在终端区底部、状态栏之上；只对当前具备可输入终端的标签显示。
        if self.state.features.broadcast.open && self.state.active_tab_has_terminal() {
            main =
                main.child(crate::broadcast::BroadcastOps::new(&mut self.state).render_bar(cx, p));
        }
        main.into_any_element()
    }

    /// 按标签组渲染标签栏：一个标签组只显示一个条目，标题取活动窗格。
    pub(crate) fn render_reconnect_banner(
        &mut self,
        tab_id: TabId,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        h_flex()
            .px_3()
            .py_1()
            .gap_3()
            .items_center()
            .justify_between()
            .bg(p.muted)
            .text_color(p.muted_foreground)
            .child(
                div()
                    .text_xs()
                    .child(rust_i18n::t!("tab.disconnected").to_string()),
            )
            .child(
                Button::new(ElementId::Name(format!("banner-reconnect-{tab_id}").into()))
                    .primary()
                    .xsmall()
                    .label(rust_i18n::t!("common.reconnect").to_string())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.reconnect_tab(tab_id, window, cx)
                    })),
            )
            .into_any_element()
    }

    pub(crate) fn render_tab_status(
        &mut self,
        index: usize,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let tab_id = self.state.workspace.tabs[index].id;
        let status = &self.state.workspace.tabs[index].status;
        let (label, detail) = match status {
            TabStatus::Connecting => (
                rust_i18n::t!("tab.connecting").to_string(),
                rust_i18n::t!("status.connecting").to_string(),
            ),
            TabStatus::Connected => (rust_i18n::t!("tab.connected").to_string(), String::new()),
            TabStatus::Disconnected => (
                rust_i18n::t!("tab.disconnected").to_string(),
                rust_i18n::t!("status.disconnected").to_string(),
            ),
            TabStatus::Failed(error) => (
                rust_i18n::t!("tab.failed").to_string(),
                localized_error(error),
            ),
        };
        v_flex()
            .flex_1()
            .items_center()
            .justify_center()
            .gap_3()
            .p_6()
            .child(div().text_lg().child(label))
            .child(div().text_sm().text_color(p.muted_foreground).child(detail))
            .child(
                Button::new(ElementId::Name(format!("reconnect-{tab_id}").into()))
                    .primary()
                    .label(rust_i18n::t!("common.reconnect").to_string())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.reconnect_tab(tab_id, window, cx)
                    })),
            )
            .into_any_element()
    }
}

/// 窗格状态的可视标识：状态灯颜色与图标。
///
/// 窗格头与标签栏的窗格列表共用同一套映射，避免两处对"什么颜色代表连接失败"
/// 的理解分叉。
pub(crate) fn pane_status_indicator(status: &TabStatus, p: &Palette) -> (Hsla, AppIcon) {
    match status {
        TabStatus::Connecting => (p.muted_foreground, AppIcon::Ui(UiIcon::REFRESH)),
        TabStatus::Connected => (p.success, AppIcon::Domain(DomainIcon::TERMINAL)),
        TabStatus::Failed(_) | TabStatus::Disconnected => (p.danger, AppIcon::Ui(UiIcon::ERROR)),
    }
}

/// OSC 9;4 任务进度的迷你展示：固定宽度进度条加百分比。
///
/// 横向标签栏与纵向标签面板共用同一渲染，保证两条栏的进度语义不漂移。
/// 设计原理（Why）：任务进度是即时状态，快照来自 [`TabProgress`]（进度条
/// 随标签快照每帧重建），不引入逐帧动画——不定态用静态占位宽度表达，
/// 避免为低频事件维护常驻动画帧回调。
///
/// `Remove` 状态不会到达渲染层（事件处理已清空快照），到达时按无进度返回。
pub(crate) fn render_tab_progress_indicator(progress: TabProgress, p: &Palette) -> AnyElement {
    let (ratio, color, label) = match progress.state {
        ProgressState::Remove => return div().into_any_element(),
        ProgressState::Set => (
            progress.progress as f32 / 100.0,
            p.primary,
            format!("{}%", progress.progress),
        ),
        ProgressState::Error => (
            progress.progress as f32 / 100.0,
            p.danger,
            format!("{}%", progress.progress),
        ),
        // 不定态：进度未知，画约五分之二宽的占位条并用省略号示意。
        ProgressState::Indeterminate => (0.4, p.muted_foreground, "…".to_string()),
    };
    h_flex()
        .flex_none()
        .items_center()
        .gap_1()
        .child(
            div()
                .w(px(24.0))
                .h(px(3.0))
                .rounded_full()
                .bg(p.muted_foreground.opacity(0.25))
                .overflow_hidden()
                .child(div().w(relative(ratio)).h_full().rounded_full().bg(color)),
        )
        .child(
            div()
                .text_xs()
                .flex_none()
                .text_color(if progress.state == ProgressState::Error {
                    p.danger
                } else {
                    p.muted_foreground
                })
                .child(label),
        )
        .into_any_element()
}
