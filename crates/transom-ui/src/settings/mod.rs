//! 偏好设置中心弹窗与通用/渲染/快捷键配置页模块。

mod pages;

use crate::app::AppShell;
use crate::keymap;
use crate::palette::{Palette, localized_error};
use gpui_kit::assets::IconName;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::setting::{SettingGroup, SettingItem, SettingPage, Settings};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::{BellStyle, TerminalCursorShape};

impl AppShell {
    pub(crate) fn open_settings_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let font_size = self.settings.terminal_profile.font_size.to_string();
        self.settings_font_size.update(cx, |input, cx| {
            input.set_value(&font_size, window, cx);
        });
        let line_height = self.settings.terminal_profile.line_height.to_string();
        self.settings_line_height.update(cx, |input, cx| {
            input.set_value(&line_height, window, cx);
        });
        let scrollback = self.settings.terminal_profile.scrollback.to_string();
        self.settings_scrollback.update(cx, |input, cx| {
            input.set_value(&scrollback, window, cx);
        });
        self.show_settings_modal = true;
        cx.notify();
    }

    pub(crate) fn save_settings(&mut self, cx: &mut Context<Self>) {
        if let Ok(font_size) = self
            .settings_font_size
            .read(cx)
            .value()
            .trim()
            .parse::<f32>()
            && (8.0..=36.0).contains(&font_size)
        {
            self.settings.terminal_profile.font_size = font_size;
        }
        if let Ok(line_height) = self
            .settings_line_height
            .read(cx)
            .value()
            .trim()
            .parse::<f32>()
            && (1.0..=2.5).contains(&line_height)
        {
            self.settings.terminal_profile.line_height = line_height;
        }
        if let Ok(scrollback) = self
            .settings_scrollback
            .read(cx)
            .value()
            .trim()
            .parse::<usize>()
            && (100..=100_000).contains(&scrollback)
        {
            self.settings.terminal_profile.scrollback = scrollback;
        }
        if let Err(error) = self.services.settings_store.save(&self.settings) {
            self.error = Some(localized_error(&error));
        } else {
            self.apply_shortcut_settings(cx);
            self.status = Some(rust_i18n::t!("status.ready").to_string());
        }
        self.show_settings_modal = false;
        cx.notify();
    }

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
                        let p = *p;
                        move |_options, _window, cx| {
                            app.update(cx, |this, cx| this.render_settings_rendering(&p, cx))
                        }
                    })),
            );

        let page_themes = SettingPage::new(rust_i18n::t!("settings.tab_themes").to_string())
            .icon(IconName::Server)
            .group(SettingGroup::new().item(SettingItem::render({
                let app = app_entity.clone();
                let p = *p;
                move |_options, _window, cx| {
                    app.update(cx, |this, cx| this.render_settings_themes(&p, cx))
                }
            })));

        let page_mouse =
            SettingPage::new(rust_i18n::t!("settings.tab_mouse_clipboard").to_string())
                .icon(IconName::ArrowLeftRight)
                .group(SettingGroup::new().item(SettingItem::render({
                    let app = app_entity.clone();
                    let p = *p;
                    move |_options, _window, cx| {
                        app.update(cx, |this, cx| this.render_settings_mouse_clipboard(&p, cx))
                    }
                })));

        let page_shortcuts = SettingPage::new(rust_i18n::t!("settings.tab_shortcuts").to_string())
            .icon(IconName::Search)
            .group(SettingGroup::new().item(SettingItem::render({
                let app = app_entity.clone();
                let p = *p;
                move |_options, _window, cx| {
                    app.update(cx, |this, cx| this.render_settings_shortcuts(&p, cx))
                }
            })));

        let page_tabs = SettingPage::new(rust_i18n::t!("settings.tab_tabs_window").to_string())
            .icon(IconName::PanelLeft)
            .group(SettingGroup::new().item(SettingItem::render({
                let app = app_entity.clone();
                let p = *p;
                move |_options, _window, cx| {
                    app.update(cx, |this, cx| this.render_settings_tabs_window(&p, cx))
                }
            })));

        let page_general = SettingPage::new(rust_i18n::t!("settings.tab_general").to_string())
            .icon(IconName::Globe)
            .group(SettingGroup::new().item(SettingItem::render({
                let app = app_entity.clone();
                let p = *p;
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
                page_general,
            ]);

        let panel = v_flex()
            .w(px(880.0))
            .h(px(640.0))
            .bg(p.popover)
            .border_1()
            .border_color(p.border)
            .rounded_lg()
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
                                this.show_settings_modal = false;
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
                                    "{}: {}",
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
                                        this.show_settings_modal = false;
                                        cx.notify();
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

    pub(crate) fn render_settings_rendering(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .gap_4()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::BOLD)
                    .child(rust_i18n::t!("settings.appearance").to_string()),
            )
            .child(Self::form_field(
                rust_i18n::t!("settings.font_size").to_string(),
                self.settings_font_size.clone(),
                p,
            ))
            .child(Self::form_field(
                rust_i18n::t!("settings.line_height").to_string(),
                self.settings_line_height.clone(),
                p,
            ))
            .child(Self::form_field(
                rust_i18n::t!("settings.scrollback").to_string(),
                self.settings_scrollback.clone(),
                p,
            ))
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .py_1()
                    .child(
                        div()
                            .text_sm()
                            .child(rust_i18n::t!("settings.cursor_shape").to_string()),
                    )
                    .child(
                        h_flex().gap_1().children(
                            [
                                (
                                    "cursor-block",
                                    "settings.cursor_block",
                                    TerminalCursorShape::Block,
                                ),
                                (
                                    "cursor-bar",
                                    "settings.cursor_bar",
                                    TerminalCursorShape::Bar,
                                ),
                                (
                                    "cursor-underline",
                                    "settings.cursor_underline",
                                    TerminalCursorShape::Underline,
                                ),
                            ]
                            .into_iter()
                            .map(|(id, key, shape)| {
                                Button::new(id)
                                    .small()
                                    .label(rust_i18n::t!(key).to_string())
                                    .selected(self.settings.terminal_profile.cursor_shape == shape)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.settings.terminal_profile.cursor_shape = shape;
                                        cx.notify();
                                    }))
                            }),
                        ),
                    ),
            )
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .py_1()
                    .child(
                        div()
                            .text_sm()
                            .child(rust_i18n::t!("settings.cursor_blink").to_string()),
                    )
                    .child(
                        Switch::new("toggle-cursor-blink")
                            .small()
                            .checked(self.settings.terminal_profile.cursor_blink)
                            .on_click(cx.listener(|this, &checked, _, cx| {
                                this.settings.terminal_profile.cursor_blink = checked;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .py_1()
                    .child(
                        div()
                            .text_sm()
                            .child(rust_i18n::t!("settings.bold_is_bright").to_string()),
                    )
                    .child(
                        Switch::new("toggle-bold-is-bright")
                            .small()
                            .checked(self.settings.terminal_profile.bold_is_bright)
                            .on_click(cx.listener(|this, &checked, _, cx| {
                                this.settings.terminal_profile.bold_is_bright = checked;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .py_1()
                    .child(
                        div()
                            .text_sm()
                            .child(rust_i18n::t!("settings.scroll_on_input").to_string()),
                    )
                    .child(
                        Switch::new("toggle-scroll-on-input")
                            .small()
                            .checked(self.settings.terminal_profile.scroll_on_input)
                            .on_click(cx.listener(|this, &checked, _, cx| {
                                this.settings.terminal_profile.scroll_on_input = checked;
                                cx.notify();
                            })),
                    ),
            )
            .into_any_element()
    }

    pub(crate) fn render_settings_shortcuts(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key_tag = |label: String| {
            div()
                .px_2()
                .py_0p5()
                .rounded_md()
                .border_1()
                .border_color(p.border)
                .bg(p.muted)
                .text_xs()
                .font_weight(FontWeight::BOLD)
                .text_color(p.foreground)
                .child(label)
        };

        let rows: Vec<AnyElement> = keymap::specs()
            .iter()
            .map(|spec| {
                let keys = keymap::pretty_keys(spec, &self.settings.shortcuts);
                let customized = self.settings.shortcuts.bindings.contains_key(spec.id);
                let id = spec.id;
                let key_elements = h_flex()
                    .gap_1()
                    .children(keys.split(" / ").map(|key| key_tag(key.to_string())));
                h_flex()
                    .justify_between()
                    .items_center()
                    .py_1p5()
                    .border_b_1()
                    .border_color(p.border)
                    .child(
                        div()
                            .text_sm()
                            .child(rust_i18n::t!(spec.label_key).to_string()),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(key_elements)
                            .child(
                                Button::new(ElementId::Name(format!("shortcut-edit-{id}").into()))
                                    .ghost()
                                    .xsmall()
                                    .label(rust_i18n::t!("settings.shortcut_edit").to_string())
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.open_shortcut_capture(id, window, cx);
                                    })),
                            )
                            .when(customized, |row| {
                                row.child(
                                    Button::new(ElementId::Name(
                                        format!("shortcut-reset-{id}").into(),
                                    ))
                                    .ghost()
                                    .xsmall()
                                    .label(rust_i18n::t!("settings.shortcut_reset").to_string())
                                    .on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            this.settings.shortcuts.bindings.remove(id);
                                            cx.notify();
                                        },
                                    )),
                                )
                            }),
                    )
                    .into_any_element()
            })
            .collect();

        let conflicts = keymap::conflict_pairs(&self.settings.shortcuts);
        let conflict_warning = (!conflicts.is_empty()).then(|| {
            let detail = conflicts
                .iter()
                .map(|(first, second)| {
                    format!(
                        "{} ↔ {}",
                        rust_i18n::t!(first.label_key),
                        rust_i18n::t!(second.label_key)
                    )
                })
                .collect::<Vec<_>>()
                .join("、");
            div()
                .text_xs()
                .text_color(p.danger)
                .child(rust_i18n::t!("settings.shortcut_conflicts", list = detail).to_string())
        });

        let has_custom = !self.settings.shortcuts.bindings.is_empty();
        v_flex()
            .gap_3()
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::BOLD)
                            .child(rust_i18n::t!("settings.tab_shortcuts").to_string()),
                    )
                    .when(has_custom, |row| {
                        row.child(
                            Button::new("reset-all-shortcuts")
                                .ghost()
                                .xsmall()
                                .label(rust_i18n::t!("settings.shortcut_reset_all").to_string())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.settings.shortcuts.bindings.clear();
                                    cx.notify();
                                })),
                        )
                    }),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("settings.shortcut_hint").to_string()),
            )
            .when_some(conflict_warning, |panel, warning| panel.child(warning))
            .child(v_flex().gap_1().children(rows))
            .into_any_element()
    }

    pub(crate) fn render_settings_general(
        &mut self,
        _window: &mut Window,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let current_bell = self.settings.window.bell_style;

        v_flex()
            .gap_4()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::BOLD)
                    .child(rust_i18n::t!("settings.general").to_string()),
            )
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .py_1()
                    .child(
                        div()
                            .text_sm()
                            .child(rust_i18n::t!("app.language").to_string()),
                    )
                    .child(
                        Button::new("settings-lang")
                            .small()
                            .label(if self.settings.language == "zh-CN" {
                                "中文"
                            } else {
                                "English"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.toggle_language(cx);
                            })),
                    ),
            )
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .py_1()
                    .child(
                        div()
                            .text_sm()
                            .child(rust_i18n::t!("settings.bell_style").to_string()),
                    )
                    .child(
                        h_flex().gap_1().children(
                            [
                                ("bell-none", "settings.bell_none", BellStyle::None),
                                ("bell-visual", "settings.bell_visual", BellStyle::Visual),
                                ("bell-audible", "settings.bell_audible", BellStyle::Audible),
                            ]
                            .into_iter()
                            .map(|(id, key, style)| {
                                Button::new(id)
                                    .small()
                                    .label(rust_i18n::t!(key).to_string())
                                    .selected(current_bell == style)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.settings.window.bell_style = style;
                                        cx.notify();
                                    }))
                            }),
                        ),
                    ),
            )
            .child(
                v_flex()
                    .gap_2()
                    .pt_2()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::BOLD)
                            .child(rust_i18n::t!("backup.title").to_string()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("backup.settings_hint").to_string()),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("backup-export")
                                    .small()
                                    .icon(IconName::Download)
                                    .label(rust_i18n::t!("backup.export_action").to_string())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_backup_export(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("backup-import")
                                    .small()
                                    .icon(IconName::Upload)
                                    .label(rust_i18n::t!("backup.import_action").to_string())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_backup_import(window, cx);
                                    })),
                            ),
                    ),
            )
            .into_any_element()
    }
}
