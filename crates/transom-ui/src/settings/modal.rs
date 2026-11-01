//! 偏好设置中心弹窗的骨架与页签装配。

use super::SettingsView;
use super::search_keywords::GroupSearch;
use crate::app::app_settings;
use crate::divider::{DividerEdge, DividerStyled};
use crate::palette::Palette;
use crate::theme::tokens::size;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::setting::{SelectIndex, SettingGroup, SettingItem, SettingPage, Settings};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;

/// 组装配 helper：把设置实体的组渲染函数包装成带标题的 `SettingGroup`。
///
/// 分组内容整体是一个自绘元素，上游搜索只按元素项声明的关键词过滤，因此这里
/// 必须同时挂上 [`GroupSearch::keywords`]；漏挂就会表现为"搜索什么都搜不到"。
fn setting_group(
    app: &Entity<SettingsView>,
    p: &Palette,
    group: GroupSearch,
    render: fn(&mut SettingsView, &Palette, &mut Context<SettingsView>) -> AnyElement,
) -> SettingGroup {
    let app = app.clone();
    let p = p.clone();
    SettingGroup::new()
        .title(rust_i18n::t!(group.title_key()).to_string())
        .item(
            SettingItem::render(move |_options, _window, cx| {
                app.update(cx, |this, cx| render(this, &p, cx))
            })
            .keywords(group.keywords()),
        )
}

/// 组装配 helper（带 `window` 版）。
///
/// 只有需要按窗口度量出图的分组走这里——目前就"关于"页一处：品牌标志的 SVG 必须
/// 按 `window.scale_factor()` 光栅化，否则会得到一张 512px 位图再被 GPU 缩小采样。
/// 没有把通用 `setting_group` 一并改掉，是为了不让七个页的签名跟着一起动。
fn setting_group_with_window(
    app: &Entity<SettingsView>,
    p: &Palette,
    group: GroupSearch,
    render: fn(&mut SettingsView, &mut Window, &Palette, &mut Context<SettingsView>) -> AnyElement,
) -> SettingGroup {
    let app = app.clone();
    let p = p.clone();
    SettingGroup::new()
        .title(rust_i18n::t!(group.title_key()).to_string())
        .item(
            SettingItem::render(move |_options, window, cx| {
                app.update(cx, |this, cx| render(this, window, &p, cx))
            })
            .keywords(group.keywords()),
        )
}

impl SettingsView {
    /// 偏好设置弹窗的弹窗体。
    ///
    /// 外壳（表面、边框、阴影、宽度、超高滚动与右上角关闭件）交给 Dialog 容器，
    /// 这里只保留"标题行 + 页面 + 页脚"三段结构；标题行原先自带一个 X，
    /// 它只置 `open = false`、不还原外观预览，与"取消"行为不一致，
    /// 因此删掉，统一走容器的关闭 → `cancel_settings`。
    pub(crate) fn render_body(
        &self,
        app: &Entity<SettingsView>,
        _window: &mut Window,
        cx: &App,
        p: &Palette,
    ) -> AnyElement {
        let app_entity = app.clone();
        // 跳页时换组件 id：`Settings` 的选中页存在 keyed state 里，只有首次创建
        // 才读取默认页；用带页号的 id 让"从命令面板指定页"与"从菜单回到上次
        // 位置"两条路径各自持有独立的组件状态。
        let (settings_id, jump_index) = match self.state.jump_page {
            Some(page) => (
                format!("app-settings-page-{page}"),
                Some(SelectIndex {
                    page_ix: page,
                    group_ix: None,
                }),
            ),
            None => ("app-settings".to_string(), None),
        };
        let mut settings_component = Settings::new(settings_id)
            .sidebar_width(px(220.0))
            .pages(Self::build_settings_pages(&app_entity, p));
        if let Some(index) = jump_index {
            settings_component = settings_component.default_selected_index(index);
        }

        // 尺寸随密度档位缩放；max_w/max_h 以 overlay 全屏为上限，宽松档下小
        // 窗口弹窗贴边而不被裁切，内容区自身可滚动。
        // 高度仍按档位固定：设置页内容很高，交给容器自适应会把页面压成滚动条。
        let panel = v_flex()
            .w_full()
            .h(rems(size::SETTINGS_DIALOG_HEIGHT))
            .overflow_hidden()
            .child(
                h_flex()
                    .w_full()
                    .px_5()
                    .py_3()
                    .justify_between()
                    .items_center()
                    .border_color(p.divider(p.border))
                    .divider_edge(DividerEdge::Bottom, p.divider_width)
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
                    .border_color(p.divider(p.border))
                    .divider_edge(DividerEdge::Top, p.divider_width)
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
                                    app_settings(cx).terminal_profile.theme_name
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
                                    .on_click(crate::dialog_host::on_event(
                                        app,
                                        |this, _: &ClickEvent, window, cx| {
                                            this.cancel_settings(window, cx);
                                        },
                                    )),
                            )
                            .child(
                                Button::new("save-settings")
                                    .primary()
                                    .small()
                                    .label(rust_i18n::t!("common.save").to_string())
                                    .on_click(crate::dialog_host::on_event(
                                        app,
                                        |this, _: &ClickEvent, window, cx| {
                                            this.save_settings(window, cx);
                                        },
                                    )),
                            ),
                    ),
            );

        panel.into_any_element()
    }

    /// 构建偏好设置所有分类页面。
    fn build_settings_pages(app: &Entity<SettingsView>, p: &Palette) -> Vec<SettingPage> {
        // 命令面板的设置页目录与本函数的装配顺序必须一致（共 9 页）：
        // 顺序不一致会让跳转落到错误页面，因此开发期直接断言。
        debug_assert_eq!(super::page_entries().len(), 9);
        let page_appearance =
            SettingPage::new(rust_i18n::t!("settings.tab_appearance").to_string())
                .icon(IconName::Palette)
                .group(setting_group(
                    app,
                    p,
                    GroupSearch::UiTheme,
                    SettingsView::render_settings_ui_theme,
                ))
                .group(setting_group(
                    app,
                    p,
                    GroupSearch::UiFont,
                    SettingsView::render_settings_ui_font,
                ))
                .group(setting_group(
                    app,
                    p,
                    GroupSearch::WindowAppearance,
                    SettingsView::render_settings_window_appearance,
                ))
                .group(setting_group(
                    app,
                    p,
                    GroupSearch::Divider,
                    SettingsView::render_settings_divider,
                ));

        let page_terminal = SettingPage::new(rust_i18n::t!("settings.tab_terminal").to_string())
            .icon(IconName::Terminal)
            .group(setting_group(
                app,
                p,
                GroupSearch::TerminalFont,
                SettingsView::render_settings_terminal_font,
            ))
            .group(setting_group(
                app,
                p,
                GroupSearch::Cursor,
                SettingsView::render_settings_cursor,
            ))
            .group(setting_group(
                app,
                p,
                GroupSearch::TerminalTheme,
                SettingsView::render_settings_themes,
            ))
            .group(setting_group(
                app,
                p,
                GroupSearch::Highlight,
                SettingsView::render_settings_highlight,
            ))
            .group(setting_group(
                app,
                p,
                GroupSearch::Trigger,
                SettingsView::render_settings_triggers,
            ))
            .group(setting_group(
                app,
                p,
                GroupSearch::BlockView,
                SettingsView::render_settings_block_view,
            ))
            .group(setting_group(
                app,
                p,
                GroupSearch::KeyBindings,
                SettingsView::render_settings_key_bindings,
            ));

        let page_files = SettingPage::new(rust_i18n::t!("settings.tab_files").to_string())
            .icon(IconName::Folder)
            .group(setting_group(
                app,
                p,
                GroupSearch::Editor,
                SettingsView::render_settings_editor,
            ))
            .group(setting_group(
                app,
                p,
                GroupSearch::FilePanel,
                SettingsView::render_settings_file_panel,
            ))
            .group(setting_group(
                app,
                p,
                GroupSearch::Transfer,
                SettingsView::render_settings_transfer,
            ))
            .group(setting_group(
                app,
                p,
                GroupSearch::Zmodem,
                SettingsView::render_settings_zmodem,
            ))
            .group(setting_group(
                app,
                p,
                GroupSearch::Audit,
                SettingsView::render_settings_audit,
            ))
            .group(setting_group(
                app,
                p,
                GroupSearch::Identities,
                SettingsView::render_settings_identities,
            ))
            .group(setting_group(
                app,
                p,
                GroupSearch::History,
                SettingsView::render_settings_history,
            ));

        let page_mouse =
            SettingPage::new(rust_i18n::t!("settings.tab_mouse_clipboard").to_string())
                .icon(IconName::ArrowLeftRight)
                .group(setting_group(
                    app,
                    p,
                    GroupSearch::Mouse,
                    SettingsView::render_settings_mouse,
                ))
                .group(setting_group(
                    app,
                    p,
                    GroupSearch::Selection,
                    SettingsView::render_settings_selection,
                ))
                .group(setting_group(
                    app,
                    p,
                    GroupSearch::Clipboard,
                    SettingsView::render_settings_clipboard,
                ));

        let page_shortcuts = SettingPage::new(rust_i18n::t!("settings.tab_shortcuts").to_string())
            .icon(IconName::Search)
            .group(
                SettingGroup::new().item(
                    SettingItem::render({
                        let app = app.clone();
                        let p = p.clone();
                        move |_options, _window, cx| {
                            app.update(cx, |this, cx| this.render_settings_shortcuts(&p, cx))
                        }
                    })
                    .keywords(GroupSearch::Shortcuts.keywords()),
                ),
            );

        let page_window_layout =
            SettingPage::new(rust_i18n::t!("settings.tab_window_layout").to_string())
                .icon(IconName::PanelLeft)
                .group(setting_group(
                    app,
                    p,
                    GroupSearch::Tabs,
                    SettingsView::render_settings_tabs,
                ))
                .group(setting_group(
                    app,
                    p,
                    GroupSearch::Window,
                    SettingsView::render_settings_window,
                ))
                .group(setting_group(
                    app,
                    p,
                    GroupSearch::ActivityBar,
                    SettingsView::render_settings_activity_bar,
                ))
                .group(setting_group(
                    app,
                    p,
                    GroupSearch::StatusBar,
                    SettingsView::render_settings_status_bar,
                ))
                .group(setting_group(
                    app,
                    p,
                    GroupSearch::Startup,
                    SettingsView::render_settings_startup,
                ));

        let page_general = SettingPage::new(rust_i18n::t!("settings.tab_general").to_string())
            .icon(IconName::Globe)
            .group(setting_group(
                app,
                p,
                GroupSearch::Language,
                SettingsView::render_settings_language,
            ))
            .group(setting_group(
                app,
                p,
                GroupSearch::Notify,
                SettingsView::render_settings_notify,
            ))
            .group(setting_group(
                app,
                p,
                GroupSearch::Proxy,
                SettingsView::render_settings_proxy,
            ));

        #[cfg(target_os = "linux")]
        let page_general = page_general.group(setting_group(
            app,
            p,
            GroupSearch::DesktopIntegration,
            SettingsView::render_settings_desktop_integration,
        ));
        let page_general = page_general
            .group(setting_group(
                app,
                p,
                GroupSearch::Backup,
                SettingsView::render_settings_backup,
            ))
            .group(setting_group(
                app,
                p,
                GroupSearch::Sync,
                SettingsView::render_settings_cloud_sync,
            ))
            .group(setting_group(
                app,
                p,
                GroupSearch::Logs,
                SettingsView::render_settings_logs,
            ));

        let page_monitor = SettingPage::new(rust_i18n::t!("settings.tab_monitor").to_string())
            .icon(IconName::Gauge)
            .group(setting_group(
                app,
                p,
                GroupSearch::MonitorSections,
                SettingsView::render_settings_monitor_sections,
            ))
            .group(setting_group(
                app,
                p,
                GroupSearch::MonitorBehavior,
                SettingsView::render_settings_monitor_behavior,
            ));

        let page_about = SettingPage::new(rust_i18n::t!("settings.tab_about").to_string())
            .icon(IconName::Info)
            .group(setting_group_with_window(
                app,
                p,
                GroupSearch::About,
                SettingsView::render_settings_about,
            ));

        // 页头横线由组件内部写死 1px，这里用页面样式覆盖成当前档位。
        let header_style = StyleRefinement::default()
            .border_color(p.divider(p.border))
            .divider_edge(DividerEdge::Bottom, p.divider_width);
        [
            page_appearance,
            page_terminal,
            page_files,
            page_mouse,
            page_shortcuts,
            page_window_layout,
            page_general,
            page_monitor,
            page_about,
        ]
        .into_iter()
        .map(|page| page.header_style(&header_style))
        .collect()
    }
}
