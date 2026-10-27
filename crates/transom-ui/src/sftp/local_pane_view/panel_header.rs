//! SFTP 面板顶部通用标题栏与关闭入口。
//!
//! 设计原理（Why）：
//! - 实现初衷：为 SFTP 双栏文件浏览器主面板及全屏/单栏模式提供统一的标题条渲染；
//! - 核心优势：集中封装面板标题、关闭事件及额外的右侧按钮扩展点。

use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::h_flex;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::*;

impl AppShell {
    /// 渲染 SFTP 面板顶部通用标题栏，包含标题、扩展按钮插槽与关闭按钮。
    pub(crate) fn render_sftp_panel_header(
        &self,
        title: String,
        extra: Option<AnyElement>,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let close_btn = Button::new("close-sftp")
            .ghost()
            .xsmall()
            .icon(IconName::X)
            .tooltip("关闭面板")
            .on_click(cx.listener(|this, _, _, cx| {
                if let Some(tab_id) = this.workspace.active_tab
                    && let Some(tab) = this.workspace.tabs.iter_mut().find(|tab| tab.id == tab_id)
                    && let Some(pane) = tab.session_pane_mut()
                {
                    pane.sftp = None;
                }
                cx.notify();
            }));

        let mut right_actions = h_flex().gap_1().items_center();
        if let Some(extra_element) = extra {
            right_actions = right_actions.child(extra_element);
        }
        right_actions = right_actions.child(close_btn);

        h_flex()
            .h(px(32.0))
            .px_3()
            .items_center()
            .justify_between()
            .border_b_1()
            .border_color(p.sidebar_border)
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .truncate()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(right_actions)
            .into_any_element()
    }
}
