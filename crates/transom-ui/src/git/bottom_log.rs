//! 底部 Git 提交记录抽屉面板与展开切换。
//!
//! 设计原理（Why）：
//! - 对齐 IntelliJ IDEA 底部 Git 工具窗口（Tool Window）风格，点击底栏或快捷键时向上展开；
//! - 顶部提供类似 IDE Tool Window 风格的标题与 Tab 项（含当前分支名称），以及收起/关闭控制；
//! - 主体复用 `render_git_log_view`，完整呈现引用树、提交图列表与提交详情三栏，避免代码重复。

use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use transom_core::{HeadState, TabId};

impl AppShell {
    /// 切换底部 Git 提交记录面板展开/收起。
    pub(crate) fn toggle_git_bottom_log(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(state) = self.git_state_of_tab_mut(tab_id) {
            state.bottom_log_open = !state.bottom_log_open;
            let is_open = state.bottom_log_open;
            let project = state.project.clone();
            if is_open {
                self.git_refresh(&project, cx);
                self.git_load_log(tab_id, 0, cx);
            }
        }
        cx.notify();
    }

    /// 渲染底部 Git 提交记录抽屉面板（向上占用一部分空间，对齐 IDEA 底部 Git 工具窗口风格）。
    pub(crate) fn render_git_bottom_log_panel(
        &self,
        tab_id: TabId,
        app: Entity<Self>,
        p: &Palette,
        cx: &App,
    ) -> AnyElement {
        let Some(index) = self.state.workspace.tab_index(tab_id) else {
            return div().into_any_element();
        };
        let Some(state) = self.state.workspace.tabs[index].git() else {
            return div().into_any_element();
        };
        let branch = match state.status.as_ref().map(|s| &s.head) {
            Some(HeadState::OnBranch(name)) | Some(HeadState::Initial { branch: name }) => {
                Some(name.clone())
            }
            _ => self
                .state
                .git
                .badges
                .get(&state.project.id)
                .and_then(|b| b.branch.clone()),
        };
        let tab_title = match branch {
            Some(name) if !name.is_empty() => {
                rust_i18n::t!("git.view.log_with_branch", branch = name.as_str()).to_string()
            }
            _ => rust_i18n::t!("git.view.log").to_string(),
        };
        let close_app = app.clone();

        let header = h_flex()
            .w_full()
            .h(px(26.0))
            .flex_none()
            .bg(p.background)
            .border_color(p.divider(p.border))
            .border_b_1()
            .items_center()
            .px_2()
            .justify_between()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::BOLD)
                            .text_color(p.muted_foreground)
                            .child("Git"),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .gap_1()
                            .px_2()
                            .py_0p5()
                            .rounded(px(3.0))
                            .bg(p.background)
                            .border_color(p.primary)
                            .border_b_2()
                            .text_xs()
                            .child(
                                Icon::new(IconName::GitCommitHorizontal)
                                    .xsmall()
                                    .text_color(p.primary),
                            )
                            .child(
                                div()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(p.foreground)
                                    .child(tab_title),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(
                        Button::new("git-bottom-log-collapse")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Minus)
                            .tooltip(rust_i18n::t!("git.view.collapse_log").to_string())
                            .on_click({
                                let close_app = close_app.clone();
                                move |_, _, cx| {
                                    close_app.update(cx, |this, cx| {
                                        this.toggle_git_bottom_log(tab_id, cx);
                                    });
                                }
                            }),
                    )
                    .child(
                        Button::new("git-bottom-log-close")
                            .ghost()
                            .xsmall()
                            .icon(IconName::X)
                            .tooltip(rust_i18n::t!("common.close").to_string())
                            .on_click(move |_, _, cx| {
                                close_app.update(cx, |this, cx| {
                                    this.toggle_git_bottom_log(tab_id, cx);
                                });
                            }),
                    ),
            );

        v_flex()
            .w_full()
            .h_full()
            .min_h_0()
            .overflow_hidden()
            .bg(p.background)
            .child(header)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_git_log_view(tab_id, app, p, cx)),
            )
            .into_any_element()
    }
}
