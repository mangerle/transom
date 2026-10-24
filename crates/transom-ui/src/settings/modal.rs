//! 偏好设置中心弹窗的骨架与页签装配。

use crate::app::AppShell;
use crate::palette::Palette;
use crate::theme::tokens::size;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::setting::{SettingGroup, SettingItem, SettingPage, Settings};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;

use super::layout_page;

impl AppShell {
    pub(crate) fn render_settings_modal(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let app_entity = cx.entity().clone();

        let page_rendering = SettingPage::new(rust_i18n::t!("settings.tab_rendering").to_string())
            .icon(IconName::Terminal)
            .group(
                SettingGroup::new()
                    .title(rust_i18n::t!("settings.appearance").to_string())
                    .item(SettingItem::render({
                        let app = app_entity.clone();
                        let p = p.clone();
                        move |_options, _window, cx| {
                            app.update(cx, |this, cx| this.render_settings_rendering(&p, cx))
                        }
                    })),
            );

        let page_themes = SettingPage::new(rust_i18n::t!("settings.tab_themes").to_string())
            .icon(IconName::Server)
            .group(
                SettingGroup::new()
                    .title(rust_i18n::t!("settings.ui_theme_group").to_string())
                    .item(SettingItem::render({
                        let app = app_entity.clone();
                        let p = p.clone();
                        move |_options, _window, cx| {
                            app.update(cx, |this, cx| this.render_settings_ui_theme(&p, cx))
                        }
                    })),
            )
            .group(
                SettingGroup::new()
                    .title(rust_i18n::t!("settings.terminal_theme_group").to_string())
                    .item(SettingItem::render({
                        let app = app_entity.clone();
                        let p = p.clone();
                        move |_options, _window, cx| {
                            app.update(cx, |this, cx| this.render_settings_themes(&p, cx))
                        }
                    })),
            );

        let page_mouse =
            SettingPage::new(rust_i18n::t!("settings.tab_mouse_clipboard").to_string())
                .icon(IconName::ArrowLeftRight)
                .group(SettingGroup::new().item(SettingItem::render({
                    let app = app_entity.clone();
                    let p = p.clone();
                    move |_options, _window, cx| {
                        app.update(cx, |this, cx| this.render_settings_mouse_clipboard(&p, cx))
                    }
                })));

        let page_shortcuts = SettingPage::new(rust_i18n::t!("settings.tab_shortcuts").to_string())
            .icon(IconName::Search)
            .group(SettingGroup::new().item(SettingItem::render({
                let app = app_entity.clone();
                let p = p.clone();
                move |_options, _window, cx| {
                    app.update(cx, |this, cx| this.render_settings_shortcuts(&p, cx))
                }
            })));

        let page_tabs = SettingPage::new(rust_i18n::t!("settings.tab_tabs_window").to_string())
            .icon(IconName::PanelLeft)
            .group(SettingGroup::new().item(SettingItem::render({
                let app = app_entity.clone();
                let p = p.clone();
                move |_options, _window, cx| {
                    app.update(cx, |this, cx| this.render_settings_tabs_window(&p, cx))
                }
            })));

        let page_layout = layout_page::layout_settings_page(app_entity.clone(), p.clone());

        let page_general = SettingPage::new(rust_i18n::t!("settings.tab_general").to_string())
            .icon(IconName::Globe)
            .group(SettingGroup::new().item(SettingItem::render({
                let app = app_entity.clone();
                let p = p.clone();
                move |_options, window, cx| {
                    app.update(cx, |this, cx| this.render_settings_general(window, &p, cx))
                }
            })));

        let settings_component = Settings::new("app-settings")
            .sidebar_width(px(220.0))
            .pages(vec![
                page_rendering,
                page_themes,
                page_mouse,
                page_shortcuts,
                page_tabs,
                page_layout,
                page_general,
            ]);

        // 尺寸随密度档位缩放；max_w/max_h 以 overlay 全屏为上限，宽松档下小
        // 窗口弹窗贴边而不被裁切，内容区自身可滚动。
        let panel = v_flex()
            .w(rems(size::SETTINGS_DIALOG_WIDTH))
            .max_w_full()
            .h(rems(size::SETTINGS_DIALOG_HEIGHT))
            .max_h_full()
            .bg(p.popover)
            .border_1()
            .border_color(p.border)
            .rounded(p.radius.lg)
            .shadow_lg()
            .overflow_hidden()
            .on_mouse_down(MouseButton::Left, |_, _, _| {})
            .child(
                h_flex()
                    .w_full()
                    .px_5()
                    .py_3()
                    .justify_between()
                    .items_center()
                    .border_b_1()
                    .border_color(p.border)
                    .bg(p.title_bar)
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                Icon::new(IconName::Settings)
                                    .size_4()
                                    .text_color(p.sidebar_accent_foreground),
                            )
                            .child(
                                div()
                                    .text_base()
                                    .font_weight(FontWeight::BOLD)
                                    .child(rust_i18n::t!("settings.title").to_string()),
                            ),
                    )
                    .child(
                        Button::new("close-settings")
                            .ghost()
                            .xsmall()
                            .icon(IconName::X)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.settings_modal.open = false;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .w_full()
                    .overflow_hidden()
                    .child(settings_component),
            )
            .child(
                h_flex()
                    .w_full()
                    .px_5()
                    .py_3()
                    .justify_between()
                    .items_center()
                    .border_t_1()
                    .border_color(p.border)
                    .bg(p.status_bar)
                    .child(
                        h_flex().gap_2().items_center().child(
                            div()
                                .text_xs()
                                .text_color(p.muted_foreground)
                                .child(format!(
                                    "{}: {}　{}: {}",
                                    rust_i18n::t!("settings.current_ui_theme"),
                                    cx.theme().theme_name(),
                                    rust_i18n::t!("settings.current_theme"),
                                    self.settings.terminal_profile.theme_name
                                )),
                        ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("cancel-settings")
                                    .ghost()
                                    .small()
                                    .label(rust_i18n::t!("common.cancel").to_string())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.cancel_settings(cx);
                                    })),
                            )
                            .child(
                                Button::new("save-settings")
                                    .primary()
                                    .small()
                                    .label(rust_i18n::t!("common.save").to_string())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.save_settings(cx);
                                    })),
                            ),
                    ),
            );

        Self::overlay(panel, p)
    }
}
