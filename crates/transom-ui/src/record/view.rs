//! 终端录制底栏指示器与回放选择模态弹窗渲染。

use crate::app::AppShell;
use crate::palette::Palette;
use crate::status_bar::BADGE_HEIGHT;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Disableable;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::*;

impl AppShell {
    /// 渲染底栏录制指示器。
    pub(crate) fn render_recording_badge(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let active_tab_id = self.workspace.active_tab;
        let active_is_recording = active_tab_id
            .and_then(|id| self.workspace.tabs.iter().find(|t| t.id == id))
            .and_then(|t| t.terminal.as_ref())
            .is_some_and(|term| term.read(cx).is_recording());

        let app_entity = cx.entity().clone();

        let badge = if !active_is_recording {
            Button::new("start-recording-btn")
                .ghost()
                .xsmall()
                .icon(IconName::Disc)
                .tooltip(rust_i18n::t!("recorder.start_btn").to_string())
                .on_click(cx.listener(|this, _, window, cx| {
                    this.toggle_recording_for_active_tab(window, cx);
                }))
                .into_any_element()
        } else {
            let stop_tip = rust_i18n::t!("recorder.stop_btn").to_string();
            // 正在录制：红色圆点 + 闪烁/高亮提示
            h_flex()
                .id("recording-indicator")
                .gap_1p5()
                .items_center()
                .px_2()
                .h(rems(BADGE_HEIGHT))
                .rounded(p.radius.md)
                .bg(p.danger.opacity(0.15))
                .border_1()
                .border_color(p.danger)
                .cursor_pointer()
                .tooltip(move |_window, cx| Tooltip::new(stop_tip.clone()).build(_window, cx))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.toggle_recording_for_active_tab(window, cx);
                }))
                .child(div().size(px(7.0)).rounded_full().bg(p.danger))
                .child(
                    div()
                        .text_xs()
                        .text_color(p.danger)
                        .font_weight(FontWeight::MEDIUM)
                        .child(rust_i18n::t!("recorder.recording").to_string()),
                )
                .into_any_element()
        };

        h_flex()
            .id("recording-badge-wrapper")
            .items_center()
            .child(badge)
            .context_menu(move |menu, _window, _cx| {
                let app = app_entity.clone();
                let toggle_label = if active_is_recording {
                    rust_i18n::t!("recorder.stop_btn").to_string()
                } else {
                    rust_i18n::t!("recorder.start_btn").to_string()
                };

                let mut menu = menu.item(
                    PopupMenuItem::new(toggle_label)
                        .icon(if active_is_recording {
                            IconName::Square
                        } else {
                            IconName::Disc
                        })
                        .on_click({
                            let app = app.clone();
                            move |_, window, cx| {
                                app.update(cx, |this, cx| {
                                    this.toggle_recording_for_active_tab(window, cx);
                                });
                            }
                        }),
                );

                if let Some(tab_id) = active_tab_id {
                    menu = menu.item(
                        PopupMenuItem::new(rust_i18n::t!("recorder.replay_menu").to_string())
                            .icon(IconName::Play)
                            .on_click({
                                let app = app.clone();
                                move |_, window, cx| {
                                    app.update(cx, |this, cx| {
                                        this.open_replay_modal(tab_id, window, cx);
                                    });
                                }
                            }),
                    );
                }

                menu
            })
            .into_any_element()
    }

    /// 渲染回放脚本选择弹窗。
    pub(crate) fn render_replay_modal(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(modal) = self.modals.replay.as_ref() else {
            return div().into_any_element();
        };

        let target_tab_id = modal.target_tab_id;
        let selected_idx = modal.selected_index;
        let selected_entry = modal.entries.get(selected_idx).cloned();

        let mut list = v_flex()
            .id("replay-script-list")
            .gap_1()
            .overflow_y_scroll();
        if modal.entries.is_empty() {
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
            for (idx, entry) in modal.entries.iter().enumerate() {
                let is_sel = idx == selected_idx;
                let bg = if is_sel {
                    p.muted
                } else {
                    gpui::transparent_black()
                };
                let border = if is_sel {
                    p.primary
                } else {
                    gpui::transparent_black()
                };
                let title = if entry.title.is_empty() {
                    entry.file_name.clone()
                } else {
                    format!("{} ({})", entry.title, entry.file_name)
                };

                let entry_path = entry.path.clone();
                let entry_title = entry.title.clone();
                let item_play_path = entry.path.clone();
                let item_play_title = entry.title.clone();

                list = list.child(
                    v_flex()
                        .id(ElementId::Name(format!("replay-item-{idx}").into()))
                        .p_2()
                        .rounded(p.radius.sm)
                        .bg(bg)
                        .border_1()
                        .border_color(border)
                        .cursor_pointer()
                        .hover(|s| s.bg(p.muted.opacity(0.4)))
                        .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                            if event.click_count() >= 2 {
                                this.modals.replay = None;
                                this.open_cast_player(
                                    entry_path.clone(),
                                    Some(entry_title.clone()),
                                    window,
                                    cx,
                                );
                            } else if let Some(m) = this.modals.replay.as_mut() {
                                m.selected_index = idx;
                                cx.notify();
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
                                    Button::new(ElementId::Name(
                                        format!("play-entry-btn-{idx}").into(),
                                    ))
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Disc)
                                    .tooltip(rust_i18n::t!("recorder.open_player").to_string())
                                    .on_click(cx.listener(
                                        move |this, _, window, cx| {
                                            this.modals.replay = None;
                                            this.open_cast_player(
                                                item_play_path.clone(),
                                                Some(item_play_title.clone()),
                                                window,
                                                cx,
                                            );
                                        },
                                    )),
                                ),
                        )
                        .child(
                            h_flex()
                                .justify_between()
                                .text_xs()
                                .text_color(p.muted_foreground)
                                .child(format!(
                                    "{:.1}s · {} 步",
                                    entry.duration_secs, entry.step_count
                                ))
                                .child(entry.modified_display.clone()),
                        ),
                );
            }
        }

        let body = v_flex()
            .gap_3()
            .h(px(320.0))
            .child(
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
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.modals.replay = None;
                                        this.pick_and_play_external_cast(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("pick-external-cast-btn")
                                    .ghost()
                                    .small()
                                    .icon(IconName::FolderOpen)
                                    .label(rust_i18n::t!("recorder.pick_external").to_string())
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.modals.replay = None;
                                        this.pick_and_replay_external_cast(
                                            target_tab_id,
                                            window,
                                            cx,
                                        );
                                    })),
                            ),
                    ),
            )
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

        let play_entry = selected_entry.clone();
        let footer = h_flex()
            .justify_between()
            .items_center()
            .child(
                Button::new("cancel-replay-modal")
                    .ghost()
                    .label(rust_i18n::t!("common.cancel").to_string())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.modals.replay = None;
                        cx.notify();
                    })),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Button::new("start-replay-selected-btn")
                            .outline()
                            .disabled(selected_entry.is_none())
                            .icon(IconName::Play)
                            .label(rust_i18n::t!("recorder.start_replay").to_string())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(entry) = selected_entry.as_ref() {
                                    let path = entry.path.clone();
                                    this.modals.replay = None;
                                    this.execute_replay_file(target_tab_id, &path, cx);
                                }
                            })),
                    )
                    .child(
                        Button::new("open-in-player-btn")
                            .primary()
                            .disabled(play_entry.is_none())
                            .icon(IconName::Disc)
                            .label(rust_i18n::t!("recorder.open_player").to_string())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if let Some(entry) = play_entry.as_ref() {
                                    let path = entry.path.clone();
                                    let title = entry.title.clone();
                                    this.modals.replay = None;
                                    this.open_cast_player(path, Some(title), window, cx);
                                }
                            })),
                    ),
            );

        let dialog = Self::dialog_frame(
            "replay-cast-modal",
            rust_i18n::t!("recorder.replay_title").to_string(),
            Some(rust_i18n::t!("recorder.format_notice").to_string()),
            body,
            Some(footer),
            px(560.0),
            p,
        );
        Self::overlay(dialog, p)
    }
}
