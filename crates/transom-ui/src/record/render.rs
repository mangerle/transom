//! 回放选择弹窗渲染：列表、选中、确认与取消都经 `cx.listener` 回到实体。
//!
//! 渲染期实体已被租借，这里只读状态；打开播放器、终端回放与外部文件选择
//! 属于窗口级动作，统一通过 [`super::ReplayEvent`] 交回外壳处理。

use super::ReplayModalState;
use super::ReplayModalView;
use super::types::RecordingEntry;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Disableable;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::*;

impl ReplayModalView {
    /// 渲染回放脚本选择弹窗。
    pub(crate) fn render_body(&self, cx: &mut Context<Self>, p: &Palette) -> AnyElement {
        let Some(state) = self.state() else {
            return div().into_any_element();
        };

        let list = Self::render_list(state, cx, p);
        let body = v_flex()
            .gap_3()
            .h(px(320.0))
            .child(Self::render_toolbar(cx, p))
            .child(
                div()
                    .flex_1()
                    .p_1()
                    .rounded(p.radius.md)
                    .bg(p.muted.opacity(0.2))
                    .border_1()
                    .border_color(p.border)
                    .child(list),
            );

        let footer = Self::render_footer(state, cx);

        crate::widgets::dialog_body(
            "replay-cast-modal",
            rust_i18n::t!("recorder.replay_title").to_string(),
            Some(rust_i18n::t!("recorder.format_notice").to_string()),
            body,
            Some(footer),
        )
    }

    /// 脚本列表；空目录展示录制引导。
    fn render_list(state: &ReplayModalState, cx: &mut Context<Self>, p: &Palette) -> AnyElement {
        let mut list = v_flex()
            .id("replay-script-list")
            .gap_1()
            .overflow_y_scroll();
        if state.entries.is_empty() {
            list = list.child(
                v_flex()
                    .p_4()
                    .gap_2()
                    .items_center()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("recorder.no_recordings").to_string())
                    .child(
                        div()
                            .text_center()
                            .child(rust_i18n::t!("recorder.no_recordings_hint").to_string()),
                    ),
            );
        } else {
            for (idx, entry) in state.entries.iter().enumerate() {
                list = list.child(replay_row(idx, entry, idx == state.selected_index, cx, p));
            }
        }
        list.into_any_element()
    }

    /// 回放工具栏：双击提示、打开外部播放器与选择外部脚本。
    fn render_toolbar(cx: &mut Context<Self>, p: &Palette) -> AnyElement {
        h_flex()
            .justify_between()
            .items_center()
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("recorder.double_click_play_hint").to_string()),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Button::new("pick-external-play-btn")
                            .outline()
                            .small()
                            .icon(IconName::Disc)
                            .label(rust_i18n::t!("recorder.open_player").to_string())
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.pick_external_player(cx);
                            })),
                    )
                    .child(
                        Button::new("pick-external-cast-btn")
                            .ghost()
                            .small()
                            .icon(IconName::FolderOpen)
                            .label(rust_i18n::t!("recorder.pick_external").to_string())
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.pick_external_replay(cx);
                            })),
                    ),
            )
            .into_any_element()
    }

    /// 回放弹窗页脚：取消、按选中脚本回放、或在独立播放器中打开。
    fn render_footer(state: &ReplayModalState, cx: &mut Context<Self>) -> AnyElement {
        let has_selection = state.entries.get(state.selected_index).is_some();
        h_flex()
            .justify_between()
            .items_center()
            .child(crate::widgets::dialog_cancel_button(
                "cancel-replay-modal",
                rust_i18n::t!("common.cancel").to_string(),
                cx.listener(|this, _: &ClickEvent, _, cx| this.close(cx)),
            ))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Button::new("start-replay-selected-btn")
                            .outline()
                            .disabled(!has_selection)
                            .icon(IconName::Play)
                            .label(rust_i18n::t!("recorder.start_replay").to_string())
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.replay_in_terminal(cx);
                            })),
                    )
                    .child(
                        crate::widgets::dialog_confirm_button(
                            "open-in-player-btn",
                            rust_i18n::t!("recorder.open_player").to_string(),
                            Some(IconName::Disc),
                            crate::widgets::DialogConfirmKind::Primary,
                            cx.listener(|this, _: &ClickEvent, _, cx| this.confirm(cx)),
                        )
                        .disabled(!has_selection),
                    ),
            )
            .into_any_element()
    }
}

/// 单条回放脚本行：双击直接打开播放器，单击选中；右侧为独立播放按钮。
fn replay_row(
    idx: usize,
    entry: &RecordingEntry,
    is_selected: bool,
    cx: &mut Context<ReplayModalView>,
    p: &Palette,
) -> AnyElement {
    let bg = if is_selected {
        p.muted
    } else {
        gpui::transparent_black()
    };
    let border = if is_selected {
        p.primary
    } else {
        gpui::transparent_black()
    };
    let title = if entry.title.is_empty() {
        entry.file_name.clone()
    } else {
        format!("{} ({})", entry.title, entry.file_name)
    };
    v_flex()
        .id(ElementId::Name(format!("replay-item-{idx}").into()))
        .p_2()
        .rounded(p.radius.sm)
        .bg(bg)
        .border_1()
        .border_color(border)
        .cursor_pointer()
        .hover(|s| s.bg(p.muted.opacity(0.4)))
        .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
            if event.click_count() >= 2 {
                this.select(idx, cx);
                this.confirm(cx);
            } else {
                this.select(idx, cx);
            }
        }))
        .child(
            h_flex()
                .justify_between()
                .items_center()
                .child(
                    div()
                        .flex_1()
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.foreground)
                        .truncate()
                        .child(title),
                )
                .child(
                    crate::widgets::IconButton::new(
                        ElementId::Name(format!("play-entry-btn-{idx}").into()),
                        IconName::Disc,
                        rust_i18n::t!("recorder.open_player").to_string(),
                    )
                    .size(crate::widgets::IconButtonSize::Compact)
                    .render()
                    .on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            this.select(idx, cx);
                            this.confirm(cx);
                        },
                    )),
                ),
        )
        .child(
            h_flex()
                .justify_between()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(
                    rust_i18n::t!(
                        "recorder.duration_steps",
                        seconds = format!("{:.1}", entry.duration_secs),
                        steps = entry.step_count
                    )
                    .to_string(),
                )
                .child(entry.modified_display.clone()),
        )
        .into_any_element()
}
