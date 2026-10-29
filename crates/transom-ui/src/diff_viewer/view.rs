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
    pub(crate) fn render_diff_viewer(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(viewer) = self.diff_viewer.as_ref() else {
            return div().into_any_element();
        };
        let title = format!(
            "本地：{}　远端：{}",
            viewer.local_path.display(),
            viewer.remote_path
        );
        let body = render_body(viewer, p);
        let toolbar = render_toolbar(viewer, p, cx);
        let footer = render_footer(viewer, p, cx);
        let dialog = v_flex()
            .id("diff-viewer-dialog")
            .w(relative(0.94))
            .h(relative(0.88))
            .gap_2()
            .p_4()
            .rounded(p.radius.xl)
            .bg(p.popover)
            .text_color(p.popover_foreground)
            .border_1()
            .border_color(p.border)
            .shadow_2xl()
            .overflow_hidden()
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .child(
                        Button::new("diff-viewer-close")
                            .ghost()
                            .compact()
                            .icon(IconName::Close)
                            .tooltip("关闭对比")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.close_diff_viewer(cx);
                            })),
                    ),
            )
            .child(toolbar)
            .child(body)
            .child(footer);
        Self::overlay(dialog, p)
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
                    "正在计算差异…"
                } else {
                    "正在读取两侧内容…"
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
            .child("两侧内容完全一致，没有可合并的差异")
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
            .child("差异视图尚未生成")
            .into_any_element(),
    }
}

fn render_toolbar(viewer: &DiffViewerState, p: &Palette, cx: &mut Context<AppShell>) -> AnyElement {
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
            div()
                .flex_1()
                .child(format!("差异 {count} 处：+{additions} 行，−{deletions} 行")),
        );
    if count > 0 {
        toolbar = toolbar
            .child(div().child(format!("第 {}/{} 处", index + 1, count)))
            .child(
                Button::new("diff-prev-hunk")
                    .ghost()
                    .compact()
                    .icon(IconName::ArrowUp)
                    .tooltip("上一处差异")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.step_diff_hunk(false, cx);
                    })),
            )
            .child(
                Button::new("diff-next-hunk")
                    .ghost()
                    .compact()
                    .icon(IconName::ArrowDown)
                    .tooltip("下一处差异")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.step_diff_hunk(true, cx);
                    })),
            );
    }
    toolbar
        .child(
            Button::new("diff-apply-local")
                .ghost()
                .compact()
                .icon(IconName::Download)
                .label("应用到本地")
                .disabled(!ready || count == 0)
                .tooltip("用远端当前变更块覆盖本地对应行")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.request_diff_merge(MergeDirection::RightToLeft, cx);
                })),
        )
        .child(
            Button::new("diff-apply-remote")
                .ghost()
                .compact()
                .icon(IconName::Upload)
                .label("应用到远端")
                .disabled(!ready || count == 0 || merge_blocked)
                .tooltip(if merge_blocked {
                    "远端文件不可写回（非 UTF-8 文本或版本指纹缺失）"
                } else {
                    "用本地当前变更块覆盖远端对应行"
                })
                .on_click(cx.listener(|this, _, _, cx| {
                    this.request_diff_merge(MergeDirection::LeftToRight, cx);
                })),
        )
        .into_any_element()
}

fn render_footer(viewer: &DiffViewerState, p: &Palette, cx: &mut Context<AppShell>) -> AnyElement {
    let mut footer = h_flex().items_center().gap_2().min_h(px(30.0));
    if let Some(confirm) = viewer.confirm {
        let (source, target) = match confirm.direction {
            MergeDirection::LeftToRight => ("本地", "远端"),
            MergeDirection::RightToLeft => ("远端", "本地"),
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
                div()
                    .flex_1()
                    .text_xs()
                    .text_color(p.warning)
                    .child(format!(
                        "确认用{source}内容覆盖{target}第 {line} 行附近的这一处变更？覆盖后无法从本应用撤销。"
                    )),
            )
            .child(
                Button::new("diff-merge-cancel")
                    .ghost()
                    .compact()
                    .label("取消")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.cancel_diff_merge(cx);
                    })),
            )
            .child(
                Button::new("diff-merge-confirm")
                    .danger()
                    .compact()
                    .icon(IconName::Check)
                    .label("确认覆盖")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.confirm_diff_merge(cx);
                    })),
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
                    .label("关闭")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.close_diff_viewer(cx);
                    })),
            );
    } else if viewer.phase == DiffPhase::Writing {
        footer = footer
            .child(
                h_flex()
                    .flex_1()
                    .items_center()
                    .gap_2()
                    .child(Spinner::new().xsmall().color(p.primary))
                    .child(div().text_xs().child("正在写回…")),
            )
            .child(
                Button::new("diff-writing-close")
                    .ghost()
                    .compact()
                    .label("关闭")
                    .disabled(true),
            );
    } else {
        footer = footer
            .child(
                div()
                    .flex_1()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child("合并只影响当前选中的这一处变更；写入后会自动重新对比。"),
            )
            .child(
                Button::new("diff-footer-close")
                    .ghost()
                    .compact()
                    .label("关闭")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.close_diff_viewer(cx);
                    })),
            );
    }
    footer.into_any_element()
}
