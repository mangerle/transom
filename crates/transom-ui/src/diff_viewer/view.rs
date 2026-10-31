//! 对比面板渲染：分栏差异主体、变更块导航与单向合并确认。

use super::{DiffPhase, DiffViewerState};
use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::diff::Diff;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::*;
use transom_core::MergeDirection;

impl AppShell {
    /// 渲染分栏对比对话框；没有面板时返回空元素。
    /// 分栏对比弹窗的弹窗体。
    ///
    /// 外壳交给 Dialog 容器：宽度按视口 94%、高度按 88% 折算成像素传入，
    /// 与迁移前 `relative(0.94)/relative(0.88)` 的观感一致；标题行原先自带
    /// 一个关闭 X，与容器右上角的关闭件重复，删掉统一走 `close_diff_viewer`。
    pub(crate) fn render_diff_viewer_body(
        &self,
        app: &Entity<AppShell>,
        window: &mut Window,
        _cx: &App,
        p: &Palette,
    ) -> AnyElement {
        let Some(viewer) = self.diff_viewer.as_ref() else {
            return div().into_any_element();
        };
        let title = rust_i18n::t!(
            "diff.title",
            local = viewer.local_path.display().to_string(),
            remote = viewer.remote_path.as_str()
        )
        .to_string();
        let body = render_body(viewer, p);
        let toolbar = render_toolbar(viewer, app, p);
        let footer = render_footer(viewer, app, p);
        let dialog = v_flex()
            .id("diff-viewer-dialog")
            .w_full()
            .h(window.viewport_size().height * 0.88)
            .gap_2()
            .overflow_hidden()
            .child(
                h_flex().items_center().gap_2().child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                ),
            )
            .child(toolbar)
            .child(body)
            .child(footer);
        dialog.into_any_element()
    }
}

fn render_body(viewer: &DiffViewerState, p: &Palette) -> AnyElement {
    if viewer.phase == DiffPhase::Loading {
        return v_flex()
            .flex_1()
            .items_center()
            .justify_center()
            .gap_2()
            .text_sm()
            .text_color(p.muted_foreground)
            .child(Spinner::new().small().color(p.primary))
            .child(
                if viewer.local_text.is_some() && viewer.remote_text.is_some() {
                    rust_i18n::t!("diff.computing").to_string()
                } else {
                    rust_i18n::t!("diff.reading").to_string()
                },
            )
            .into_any_element();
    }
    if viewer.diff.as_ref().is_some_and(|diff| diff.is_empty()) {
        return v_flex()
            .flex_1()
            .items_center()
            .justify_center()
            .text_sm()
            .text_color(p.muted_foreground)
            .child(rust_i18n::t!("diff.identical").to_string())
            .into_any_element();
    }
    match viewer.view.as_ref() {
        Some(state) => Diff::new(state)
            .line_number(true)
            .syntax_highlight(true)
            .change_background(true)
            .flex_1()
            .w_full()
            .into_any_element(),
        None => v_flex()
            .flex_1()
            .items_center()
            .justify_center()
            .text_sm()
            .text_color(p.muted_foreground)
            .child(rust_i18n::t!("diff.not_ready").to_string())
            .into_any_element(),
    }
}

fn render_toolbar(viewer: &DiffViewerState, app: &Entity<AppShell>, p: &Palette) -> AnyElement {
    let (additions, deletions, count, index) = match viewer.diff.as_ref() {
        Some(diff) => (
            diff.additions,
            diff.deletions,
            diff.hunks.len(),
            viewer.hunk_index.unwrap_or(0),
        ),
        None => (0, 0, 0, 0),
    };
    let ready = viewer.phase == DiffPhase::Ready;
    let merge_blocked = viewer.remote_meta.is_none_or(|meta| meta.transcoded);
    let mut toolbar = h_flex()
        .items_center()
        .gap_2()
        .text_xs()
        .text_color(p.muted_foreground)
        .child(
            div().flex_1().child(
                rust_i18n::t!(
                    "diff.summary",
                    count = count,
                    additions = additions,
                    deletions = deletions
                )
                .to_string(),
            ),
        );
    if count > 0 {
        toolbar = toolbar
            .child(div().child(
                rust_i18n::t!("diff.hunk_position", index = index + 1, count = count).to_string(),
            ))
            .child(
                Button::new("diff-prev-hunk")
                    .ghost()
                    .compact()
                    .icon(IconName::ArrowUp)
                    .tooltip(rust_i18n::t!("diff.prev_hunk").to_string())
                    .on_click(crate::dialog_host::on_event(
                        app,
                        |this, _: &ClickEvent, _, cx| {
                            this.step_diff_hunk(false, cx);
                        },
                    )),
            )
            .child(
                Button::new("diff-next-hunk")
                    .ghost()
                    .compact()
                    .icon(IconName::ArrowDown)
                    .tooltip(rust_i18n::t!("diff.next_hunk").to_string())
                    .on_click(crate::dialog_host::on_event(
                        app,
                        |this, _: &ClickEvent, _, cx| {
                            this.step_diff_hunk(true, cx);
                        },
                    )),
            );
    }
    toolbar
        .child(
            Button::new("diff-apply-local")
                .ghost()
                .compact()
                .icon(IconName::Download)
                .label(rust_i18n::t!("diff.apply_local").to_string())
                .disabled(!ready || count == 0)
                .tooltip(rust_i18n::t!("diff.apply_local_tooltip").to_string())
                .on_click(crate::dialog_host::on_event(
                    app,
                    |this, _: &ClickEvent, _, cx| {
                        this.request_diff_merge(MergeDirection::RightToLeft, cx);
                    },
                )),
        )
        .child(
            Button::new("diff-apply-remote")
                .ghost()
                .compact()
                .icon(IconName::Upload)
                .label(rust_i18n::t!("diff.apply_remote").to_string())
                .disabled(!ready || count == 0 || merge_blocked)
                .tooltip(if merge_blocked {
                    rust_i18n::t!("diff.apply_remote_blocked").to_string()
                } else {
                    rust_i18n::t!("diff.apply_remote_tooltip").to_string()
                })
                .on_click(crate::dialog_host::on_event(
                    app,
                    |this, _: &ClickEvent, _, cx| {
                        this.request_diff_merge(MergeDirection::LeftToRight, cx);
                    },
                )),
        )
        .into_any_element()
}

fn render_footer(viewer: &DiffViewerState, app: &Entity<AppShell>, p: &Palette) -> AnyElement {
    let mut footer = h_flex().items_center().gap_2().min_h(px(30.0));
    if let Some(confirm) = viewer.confirm {
        let (source, target) = match confirm.direction {
            MergeDirection::LeftToRight => (
                rust_i18n::t!("diff.local").to_string(),
                rust_i18n::t!("diff.remote").to_string(),
            ),
            MergeDirection::RightToLeft => (
                rust_i18n::t!("diff.remote").to_string(),
                rust_i18n::t!("diff.local").to_string(),
            ),
        };
        let line = viewer
            .diff
            .as_ref()
            .and_then(|diff| diff.hunks.get(confirm.hunk))
            .map(|hunk| match confirm.direction {
                MergeDirection::LeftToRight => hunk.new_start + 1,
                MergeDirection::RightToLeft => hunk.old_start + 1,
            })
            .unwrap_or(0);
        footer = footer
            .child(
                div().flex_1().text_xs().text_color(p.warning).child(
                    rust_i18n::t!(
                        "diff.confirm_overwrite",
                        source = source,
                        target = target,
                        line = line
                    )
                    .to_string(),
                ),
            )
            .child(
                Button::new("diff-merge-cancel")
                    .ghost()
                    .compact()
                    .label(rust_i18n::t!("common.cancel").to_string())
                    .on_click(crate::dialog_host::on_event(
                        app,
                        |this, _: &ClickEvent, _, cx| {
                            this.cancel_diff_merge(cx);
                        },
                    )),
            )
            .child(
                Button::new("diff-merge-confirm")
                    .danger()
                    .compact()
                    .icon(IconName::Check)
                    .label(rust_i18n::t!("diff.confirm_action").to_string())
                    .on_click(crate::dialog_host::on_event(
                        app,
                        |this, _: &ClickEvent, _, cx| {
                            this.confirm_diff_merge(cx);
                        },
                    )),
            );
    } else if let Some(error) = viewer.error.as_deref() {
        footer = footer
            .child(
                div()
                    .flex_1()
                    .text_xs()
                    .text_color(p.danger)
                    .child(error.to_string()),
            )
            .child(
                Button::new("diff-error-close")
                    .ghost()
                    .compact()
                    .label(rust_i18n::t!("diff.close").to_string())
                    .on_click(crate::dialog_host::on_event(
                        app,
                        |this, _: &ClickEvent, _, cx| {
                            this.close_diff_viewer(cx);
                        },
                    )),
            );
    } else if viewer.phase == DiffPhase::Writing {
        footer = footer
            .child(
                h_flex()
                    .flex_1()
                    .items_center()
                    .gap_2()
                    .child(Spinner::new().xsmall().color(p.primary))
                    .child(
                        div()
                            .text_xs()
                            .child(rust_i18n::t!("diff.writing").to_string()),
                    ),
            )
            .child(
                Button::new("diff-writing-close")
                    .ghost()
                    .compact()
                    .label(rust_i18n::t!("diff.close").to_string())
                    .disabled(true),
            );
    } else {
        footer = footer
            .child(
                div()
                    .flex_1()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("diff.merge_hint").to_string()),
            )
            .child(
                Button::new("diff-footer-close")
                    .ghost()
                    .compact()
                    .label(rust_i18n::t!("diff.close").to_string())
                    .on_click(crate::dialog_host::on_event(
                        app,
                        |this, _: &ClickEvent, _, cx| {
                            this.close_diff_viewer(cx);
                        },
                    )),
            );
    }
    footer.into_any_element()
}
