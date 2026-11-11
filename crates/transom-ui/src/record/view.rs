//! 终端录制底栏指示器渲染；回放选择弹窗已迁至 `render.rs`。

use crate::app::AppShell;
use crate::palette::Palette;
use crate::status_bar::BADGE_HEIGHT;
use gpui_kit::base::h_flex;
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::*;
use transom_ui_icons::AppIcon;
use transom_ui_icons::CustomIcon;
use transom_ui_icons::UiIcon;

impl AppShell {
    /// 渲染底栏录制指示器。
    /// 录制中的红色指示徽标：点击停止。
    fn render_recording_indicator(&mut self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let stop_tip = rust_i18n::t!("recorder.stop_btn").to_string();
        // 正在录制：使用 Tag::danger() 规范化徽标
        let tag = Tag::danger()
            .h(rems(BADGE_HEIGHT))
            .rounded(p.radius.md)
            .gap_1p5()
            .child(crate::widgets::status_dot_bare(p.danger))
            .child(rust_i18n::t!("recorder.recording").to_string());
        div()
            .id("recording-indicator")
            .cursor_pointer()
            .tooltip(move |_window, cx| Tooltip::new(stop_tip.clone()).build(_window, cx))
            .on_click(cx.listener(|this, _, window, cx| {
                this.toggle_recording_for_active_tab(window, cx);
            }))
            .child(tag)
            .into_any_element()
    }

    pub(crate) fn render_recording_badge(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let active_tab_id = self.state.workspace.active_tab;
        let active_is_recording = active_tab_id
            .and_then(|id| self.state.workspace.tab(id))
            .and_then(|t| t.terminal())
            .is_some_and(|term| term.read(cx).is_recording());

        let app_entity = cx.entity().clone();

        let badge = if !active_is_recording {
            crate::widgets::IconButton::new(
                "start-recording-btn",
                AppIcon::Custom(CustomIcon::RECORDING),
                rust_i18n::t!("recorder.start_btn").to_string(),
            )
            .size(crate::widgets::IconButtonSize::Compact)
            .render()
            .on_click(cx.listener(|this, _, window, cx| {
                this.toggle_recording_for_active_tab(window, cx);
            }))
            .into_any_element()
        } else {
            self.render_recording_indicator(p, cx)
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

                let mut menu = menu.item(crate::widgets::menu_item(
                    toggle_label,
                    if active_is_recording {
                        AppIcon::Ui(UiIcon::STOP)
                    } else {
                        AppIcon::Custom(CustomIcon::RECORDING)
                    },
                    {
                        let app = app.clone();
                        move |_, window, cx| {
                            app.update(cx, |this, cx| {
                                this.toggle_recording_for_active_tab(window, cx);
                            });
                        }
                    },
                ));

                if let Some(tab_id) = active_tab_id {
                    menu = menu.item(crate::widgets::menu_item(
                        rust_i18n::t!("recorder.replay_menu").to_string(),
                        AppIcon::Ui(UiIcon::RUN),
                        {
                            let app = app.clone();
                            move |_, window, cx| {
                                app.update(cx, |this, cx| {
                                    this.open_replay_modal(tab_id, window, cx);
                                });
                            }
                        },
                    ));
                }

                menu
            })
            .into_any_element()
    }
}
