//! 三方合并冲突解决器的文件清单与三方文本对比视图渲染。

use super::GitConflictModal;
use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::*;
use transom_ui_icons::{AppIcon, AppIconElement};

impl AppShell {
    pub(super) fn render_conflict_body(
        &self,
        modal: &GitConflictModal,
        app: &Entity<Self>,
        p: &Palette,
    ) -> Div {
        let file_list = self.render_conflict_file_list(modal, app, p);
        let diff_panels = self.render_conflict_3way_panels(modal, p);

        h_flex()
            .flex_1()
            .w_full()
            .min_h_0()
            .child(file_list)
            .child(diff_panels)
    }

    fn render_conflict_file_list(
        &self,
        modal: &GitConflictModal,
        app: &Entity<Self>,
        p: &Palette,
    ) -> Div {
        let app_select = app.clone();
        let selected_idx = modal.selected_index;

        let items: Vec<AnyElement> = modal
            .files
            .iter()
            .enumerate()
            .map(|(idx, path)| {
                let app_item = app_select.clone();
                let is_selected = idx == selected_idx;
                let bg_color = if is_selected {
                    p.list_active
                } else {
                    gpui::transparent_black()
                };
                let text_color = if is_selected { p.primary } else { p.foreground };

                h_flex()
                    .id(ElementId::NamedInteger("conflict-item".into(), idx as u64))
                    .w_full()
                    .h(px(24.0))
                    .px_2()
                    .gap_1p5()
                    .items_center()
                    .bg(bg_color)
                    .rounded(px(4.0))
                    .cursor_pointer()
                    .on_click(move |_, _, cx| {
                        app_item.update(cx, |this, cx| {
                            this.git_conflict_select_file(idx, cx);
                        });
                    })
                    .child(
                        AppIconElement::new(AppIcon::for_file(path))
                            .with_rems(rems(crate::theme::tokens::control::ICON)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_xs()
                            .text_color(text_color)
                            .child(path.clone()),
                    )
                    .into_any_element()
            })
            .collect();

        v_flex()
            .w(px(200.0))
            .h_full()
            .flex_none()
            .bg(p.list_head)
            .border_r_1()
            .border_color(p.divider(p.border))
            .p_1()
            .child(
                div()
                    .px_2()
                    .py_1()
                    .text_xs()
                    .text_color(p.muted)
                    .font_weight(FontWeight::MEDIUM)
                    .child(rust_i18n::t!("git.conflict.files_list").to_string()),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .children(items),
            )
    }

    fn render_conflict_3way_panels(&self, modal: &GitConflictModal, p: &Palette) -> Div {
        if modal.loading {
            return v_flex()
                .flex_1()
                .h_full()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted)
                        .child(rust_i18n::t!("git.conflict.loading_content").to_string()),
                );
        }

        let sides = modal.sides.as_ref();
        let ours_text = sides.and_then(|s| s.ours.as_deref()).unwrap_or("");
        let working_text = sides.map(|s| s.working.as_str()).unwrap_or("");
        let theirs_text = sides.and_then(|s| s.theirs.as_deref()).unwrap_or("");

        let panel_ours = self.render_content_column(
            rust_i18n::t!("git.conflict.col_ours").to_string(),
            ours_text,
            p.primary,
            p,
        );
        let panel_working = self.render_content_column(
            rust_i18n::t!("git.conflict.col_result").to_string(),
            working_text,
            p.warning,
            p,
        );
        let panel_theirs = self.render_content_column(
            rust_i18n::t!("git.conflict.col_theirs").to_string(),
            theirs_text,
            p.success,
            p,
        );

        h_flex()
            .flex_1()
            .h_full()
            .min_w_0()
            .child(panel_ours)
            .child(panel_working)
            .child(panel_theirs)
    }

    fn render_content_column(
        &self,
        title: String,
        content: &str,
        badge_color: gpui::Hsla,
        p: &Palette,
    ) -> Div {
        let lines: Vec<AnyElement> = content
            .lines()
            .enumerate()
            .take(500)
            .map(|(idx, line)| {
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_start()
                    .child(
                        div()
                            .w(px(28.0))
                            .flex_none()
                            .text_right()
                            .text_xs()
                            .text_color(p.muted)
                            .child(format!("{}", idx + 1)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .text_color(p.foreground)
                            .child(line.to_string()),
                    )
                    .into_any_element()
            })
            .collect();

        v_flex()
            .flex_1()
            .h_full()
            .min_w_0()
            .border_r_1()
            .border_color(p.divider(p.border))
            .child(
                h_flex()
                    .w_full()
                    .h(px(26.0))
                    .flex_none()
                    .px_2()
                    .items_center()
                    .bg(p.list_head)
                    .border_b_1()
                    .border_color(p.divider(p.border))
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(badge_color)
                            .child(title),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .p_2()
                    .children(lines),
            )
    }
}
