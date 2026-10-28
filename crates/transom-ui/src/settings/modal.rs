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

/// 组装配 helper（带 `window` 版）。
///
/// 只有需要按窗口度量出图的分组走这里——目前就"关于"页一处：品牌标志的 SVG 必须
/// 按 `window.scale_factor()` 光栅化，否则会得到一张 512px 位图再被 GPU 缩小采样。
/// 没有把通用 `setting_group` 一并改掉，是为了不让七个页的签名跟着一起动。
fn setting_group_with_window(
    app: &Entity<AppShell>,
    p: &Palette,
    title: &'static str,
    render: fn(&mut AppShell, &mut Window, &Palette, &mut Context<AppShell>) -> AnyElement,
) -> SettingGroup {
    let app = app.clone();
    let p = p.clone();
    SettingGroup::new()
        .title(rust_i18n::t!(title).to_string())
        .item(SettingItem::render(move |_options, window, cx| {
            app.update(cx, |this, cx| render(this, window, &p, cx))
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
        let settings_component = Settings::new("app-settings")
            .sidebar_width(px(220.0))
            .pages(Self::build_settings_pages(&app_entity, p));

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
                                this.modals.settings.open = false;
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
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.cancel_settings(window, cx);
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

    /// 构建偏好设置所有分类页面。
    fn build_settings_pages(app: &Entity<AppShell>, p: &Palette) -> Vec<SettingPage> {
        let page_appearance =
            SettingPage::new(rust_i18n::t!("settings.tab_appearance").to_string())
                .icon(IconName::Palette)
                .group(setting_group(
                    app,
                    p,
                    "settings.ui_theme_group",
                    AppShell::render_settings_ui_theme,
                ))
                .group(setting_group(
                    app,
                    p,
                    "settings.ui_font_group",
                    AppShell::render_settings_ui_font,
                ))
                .group(setting_group(
                    app,
                    p,
                    "settings.window_appearance_group",
                    AppShell::render_settings_window_appearance,
                ));

        let page_terminal = SettingPage::new(rust_i18n::t!("settings.tab_terminal").to_string())
            .icon(IconName::Terminal)
            .group(setting_group(
                app,
                p,
                "settings.terminal_font_group",
                AppShell::render_settings_terminal_font,
            ))
            .group(setting_group(
                app,
                p,
                "settings.cursor_group",
                AppShell::render_settings_cursor,
            ))
            .group(setting_group(
                app,
                p,
                "settings.terminal_theme_group",
                AppShell::render_settings_themes,
            ))
            .group(setting_group(
                app,
                p,
                "settings.highlight_group",
                AppShell::render_settings_highlight,
            ))
            .group(setting_group(
                app,
                p,
                "settings.trigger_group",
                AppShell::render_settings_triggers,
            ))
            .group(setting_group(
                app,
                p,
                "settings.block_view_group",
                AppShell::render_settings_block_view,
            ))
            .group(setting_group(
                app,
                p,
                "settings.key_bindings_group",
                AppShell::render_settings_key_bindings,
            ));

        let page_files = SettingPage::new(rust_i18n::t!("settings.tab_files").to_string())
            .icon(IconName::Folder)
            .group(setting_group(
                app,
                p,
                "settings.editor_group",
                AppShell::render_settings_editor,
            ))
            .group(setting_group(
                app,
                p,
                "settings.file_panel_group",
                AppShell::render_settings_file_panel,
            ))
            .group(setting_group(
                app,
                p,
                "settings.transfer_group",
                AppShell::render_settings_transfer,
            ))
            .group(setting_group(
                app,
                p,
                "settings.zmodem_group",
                AppShell::render_settings_zmodem,
            ))
            .group(setting_group(
                app,
                p,
                "settings.audit_group",
                AppShell::render_settings_audit,
            ))
            .group(setting_group(
                app,
                p,
                "settings.identity_group",
                AppShell::render_settings_identities,
            ))
            .group(setting_group(
                app,
                p,
                "settings.history_group",
                AppShell::render_settings_history,
            ));

        let page_mouse =
            SettingPage::new(rust_i18n::t!("settings.tab_mouse_clipboard").to_string())
                .icon(IconName::ArrowLeftRight)
                .group(setting_group(
                    app,
                    p,
                    "settings.mouse_group",
                    AppShell::render_settings_mouse,
                ))
                .group(setting_group(
                    app,
                    p,
                    "settings.selection_group",
                    AppShell::render_settings_selection,
                ))
                .group(setting_group(
                    app,
                    p,
                    "settings.clipboard_group",
                    AppShell::render_settings_clipboard,
                ));

        let page_shortcuts = SettingPage::new(rust_i18n::t!("settings.tab_shortcuts").to_string())
            .icon(IconName::Search)
            .group(SettingGroup::new().item(SettingItem::render({
                let app = app.clone();
                let p = p.clone();
                move |_options, _window, cx| {
                    app.update(cx, |this, cx| this.render_settings_shortcuts(&p, cx))
                }
            })));

        let page_window_layout =
            SettingPage::new(rust_i18n::t!("settings.tab_window_layout").to_string())
                .icon(IconName::PanelLeft)
                .group(setting_group(
                    app,
                    p,
                    "settings.tabs_group",
                    AppShell::render_settings_tabs,
                ))
                .group(setting_group(
                    app,
                    p,
                    "settings.window_group",
                    AppShell::render_settings_window,
                ))
                .group(setting_group(
                    app,
                    p,
                    "settings.activity_bar",
                    AppShell::render_settings_activity_bar,
                ))
                .group(setting_group(
                    app,
                    p,
                    "settings.status_bar",
                    AppShell::render_settings_status_bar,
                ))
                .group(setting_group(
                    app,
                    p,
                    "settings.startup_group",
                    AppShell::render_settings_startup,
                ));

        let page_general = SettingPage::new(rust_i18n::t!("settings.tab_general").to_string())
            .icon(IconName::Globe)
            .group(setting_group(
                app,
                p,
                "settings.language_group",
                AppShell::render_settings_language,
            ))
            .group(setting_group(
                app,
                p,
                "settings.notify_group",
                AppShell::render_settings_notify,
            ))
            .group(setting_group(
                app,
                p,
                "settings.proxy_group",
                AppShell::render_settings_proxy,
            ));

        #[cfg(target_os = "linux")]
        let page_general = page_general.group(setting_group(
            app,
            p,
            "settings.desktop_integration_group",
            AppShell::render_settings_desktop_integration,
        ));
        let page_general = page_general
            .group(setting_group(
                app,
                p,
                "settings.backup_group",
                AppShell::render_settings_backup,
            ))
            .group(setting_group(
                app,
                p,
                "settings.sync_group",
                AppShell::render_settings_cloud_sync,
            ))
            .group(setting_group(
                app,
                p,
                "settings.log_group",
                AppShell::render_settings_logs,
            ));

        let page_about = SettingPage::new(rust_i18n::t!("settings.tab_about").to_string())
            .icon(IconName::Info)
            .group(setting_group_with_window(
                app,
                p,
                "settings.about_group",
                AppShell::render_settings_about,
            ));

        vec![
            page_appearance,
            page_terminal,
            page_files,
            page_mouse,
            page_shortcuts,
            page_window_layout,
            page_general,
            page_about,
        ]
    }
}
