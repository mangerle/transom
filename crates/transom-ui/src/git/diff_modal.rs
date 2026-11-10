//! 独立 Git 文件差异比较弹窗（对齐 IDEA 双击文件呼出 Diff 独立窗口）。
//!
//! 设计原理（Why）：
//! - 当在 Git 提交详情列表中双击文件或点击比较按钮时，全屏/大窗口呈现双列差异对比；
//! - 避免在有限宽度的抽屉右栏中强塞双列 Diff，释放右栏展示完整的变更文件树与提交说明；
//! - 顶栏提供文件名提示与关闭按钮，主体直接复用 diff_view 的成熟分栏对比能力。

use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use transom_core::TabId;

impl AppShell {
    /// 打开独立全功能 Diff 对比弹窗（对应双击文件或点击比较差异）。
    pub(crate) fn git_open_diff_modal(
        &mut self,
        tab_id: TabId,
        path: &str,
        cx: &mut Context<Self>,
    ) {
        self.git_diff_commit_file(tab_id, path, cx);
        if let Some(workspace) = self.state.git_workspace(tab_id) {
            workspace.update(cx, |workspace, _| {
                workspace.state.diff_modal_open = true;
            });
        }
        cx.notify();
    }

    /// 关闭独立 Diff 对比弹窗。
    pub(crate) fn git_close_diff_modal(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(workspace) = self.state.git_workspace(tab_id) {
            workspace.update(cx, |workspace, _| {
                workspace.state.diff_modal_open = false;
            });
        }
        cx.notify();
    }

    /// 渲染独立 Diff 模态浮层。
    pub(crate) fn render_git_diff_modal(
        &self,
        tab_id: TabId,
        app: Entity<Self>,
        p: &Palette,
        cx: &App,
    ) -> AnyElement {
        let Some(workspace) = self.state.git_workspace(tab_id) else {
            return div().into_any_element();
        };
        let state = &workspace.read(cx).state;
        if !state.diff_modal_open {
            return div().into_any_element();
        }

        let close_app = app.clone();
        let file_path = state
            .active_diff()
            .map(|d| d.path.clone())
            .unwrap_or_default();

        let header = h_flex()
            .w_full()
            .h(px(30.0))
            .flex_none()
            .px_3()
            .items_center()
            .justify_between()
            .bg(p.background)
            .border_color(p.divider(p.border))
            .border_b_1()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Icon::new(IconName::GitCompare)
                            .xsmall()
                            .text_color(p.primary),
                    )
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(p.foreground)
                            .child(file_path),
                    ),
            )
            .child(
                Button::new("git-diff-modal-close")
                    .ghost()
                    .xsmall()
                    .icon(IconName::X)
                    .accessibility_label(rust_i18n::t!("common.close").to_string())
                    .tooltip(rust_i18n::t!("common.close").to_string())
                    .on_click(move |_, _, cx| {
                        close_app.update(cx, |this, cx| {
                            this.git_close_diff_modal(tab_id, cx);
                        });
                    }),
            );

        let body = div()
            .flex_1()
            .min_h_0()
            .child(crate::git::diff_view::render(state, app, tab_id, p));

        // 居中大弹窗容器（宽 92%，高 88%）
        v_flex()
            .absolute()
            .inset_0()
            .bg(p.overlay)
            .items_center()
            .justify_center()
            .child(
                v_flex()
                    .w(relative(0.92))
                    .h(relative(0.88))
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(p.divider(p.border))
                    .bg(p.background)
                    .shadow_lg()
                    .overflow_hidden()
                    .child(header)
                    .child(body),
            )
            .into_any_element()
    }
}
