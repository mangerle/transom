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

/// 组装配 helper：把 AppShell 的组渲染函数包装成带标题的 `SettingGroup`。
fn setting_group(
    app: &Entity<AppShell>,
    p: &Palette,
    title: &'static str,
    render: fn(&mut AppShell, &Palette, &mut Context<AppShell>) -> AnyElement,
) -> SettingGroup {
    let app = app.clone();
    let p = p.clone();
    SettingGroup::new()
        .title(rust_i18n::t!(title).to_string())
        .item(SettingItem::render(move |_options, _window, cx| {
            app.update(cx, |this, cx| render(this, &p, cx))
        }))
}

impl AppShell {
    pub(crate) fn render_settings_modal(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let app_entity = cx.entity().clone();
        // 页面结构对齐 Zed：外观（界面皮肤）与终端（终端一切）分立，
        // 窗口与布局合并为一页。
        let page_appearance =
            SettingPage::new(rust_i18n::t!("settings.tab_appearance").to_string())
                .icon(IconName::Palette)
                .group(setting_group(
                    &app_entity,
                    p,
                    "settings.ui_theme_group",
                    AppShell::render_settings_ui_theme,
                ))
                .group(setting_group(
                    &app_entity,
                    p,
                    "settings.ui_font_group",
                    AppShell::render_settings_ui_font,
                ));

        let page_terminal = SettingPage::new(rust_i18n::t!("settings.tab_terminal").to_string())
            .icon(IconName::Terminal)
            .group(setting_group(
                &app_entity,
                p,
                "settings.terminal_font_group",
                AppShell::render_settings_terminal_font,
            ))
            .group(setting_group(
                &app_entity,
                p,
                "settings.cursor_group",
                AppShell::render_settings_cursor,
            ))
            .group(setting_group(
                &app_entity,
                p,
                "settings.terminal_theme_group",
                AppShell::render_settings_themes,
            ))
            .group(setting_group(
                &app_entity,
                p,
                "settings.highlight_group",
                AppShell::render_settings_highlight,
            ));

        let page_files = SettingPage::new(rust_i18n::t!("settings.tab_files").to_string())
            .icon(IconName::Folder)
            .group(setting_group(
                &app_entity,
                p,
                "settings.editor_group",
                AppShell::render_settings_editor,
            ))
            .group(setting_group(
                &app_entity,
                p,
                "settings.file_panel_group",
                AppShell::render_settings_file_panel,
            ))
            .group(setting_group(
                &app_entity,
                p,
                "settings.transfer_group",
                AppShell::render_settings_transfer,
            ))
            .group(setting_group(
                &app_entity,
                p,
                "settings.zmodem_group",
                AppShell::render_settings_zmodem,
            ))
            .group(setting_group(
                &app_entity,
                p,
                "settings.audit_group",
                AppShell::render_settings_audit,
            ))
            .group(setting_group(
                &app_entity,
                p,
                "settings.identity_group",
                AppShell::render_settings_identities,
            ))
            .group(setting_group(
                &app_entity,
                p,
                "settings.history_group",
                AppShell::render_settings_history,
            ));

        let page_mouse =
            SettingPage::new(rust_i18n::t!("settings.tab_mouse_clipboard").to_string())
                .icon(IconName::ArrowLeftRight)
                .group(setting_group(
                    &app_entity,
                    p,
                    "settings.mouse_group",
                    AppShell::render_settings_mouse,
                ))
                .group(setting_group(
                    &app_entity,
                    p,
                    "settings.selection_group",
                    AppShell::render_settings_selection,
                ))
                .group(setting_group(
                    &app_entity,
                    p,
                    "settings.clipboard_group",
                    AppShell::render_settings_clipboard,
                ));

        let page_shortcuts = SettingPage::new(rust_i18n::t!("settings.tab_shortcuts").to_string())
            .icon(IconName::Search)
            .group(SettingGroup::new().item(SettingItem::render({
                let app = app_entity.clone();
                let p = p.clone();
                move |_options, _window, cx| {
                    app.update(cx, |this, cx| this.render_settings_shortcuts(&p, cx))
                }
            })));

        // 窗口与布局合并：标签页、窗口与会话、活动栏、状态栏四组。
        let page_window_layout =
            SettingPage::new(rust_i18n::t!("settings.tab_window_layout").to_string())
                .icon(IconName::PanelLeft)
                .group(setting_group(
                    &app_entity,
                    p,
                    "settings.tabs_group",
                    AppShell::render_settings_tabs,
                ))
                .group(setting_group(
                    &app_entity,
                    p,
                    "settings.window_group",
                    AppShell::render_settings_window,
                ))
                .group(setting_group(
                    &app_entity,
                    p,
                    "settings.activity_bar",
                    AppShell::render_settings_activity_bar,
                ))
                .group(setting_group(
                    &app_entity,
                    p,
                    "settings.status_bar",
                    AppShell::render_settings_status_bar,
                ));

        let page_general = SettingPage::new(rust_i18n::t!("settings.tab_general").to_string())
            .icon(IconName::Globe)
            .group(setting_group(
                &app_entity,
                p,
                "settings.language_group",
                AppShell::render_settings_language,
            ))
            .group(setting_group(
                &app_entity,
                p,
                "settings.notify_group",
                AppShell::render_settings_notify,
            ))
            .group(setting_group(
                &app_entity,
                p,
                "settings.backup_group",
                AppShell::render_settings_backup,
            ))
            .group(setting_group(
                &app_entity,
                p,
                "settings.sync_group",
                AppShell::render_settings_cloud_sync,
            ));

        let settings_component = Settings::new("app-settings")
            .sidebar_width(px(220.0))
            .pages(vec![
                page_appearance,
                page_terminal,
                page_files,
                page_mouse,
                page_shortcuts,
                page_window_layout,
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
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.save_settings(window, cx);
                                    })),
                            ),
                    ),
            );

        Self::overlay(panel, p)
    }
}
