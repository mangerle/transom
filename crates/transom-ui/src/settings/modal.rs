//! 偏好设置中心弹窗的骨架与页签装配。

use super::SettingsView;
use super::search_keywords::GroupSearch;
use crate::app::app_settings;
use crate::divider::{DividerEdge, DividerStyled};
use crate::palette::Palette;
use crate::theme::tokens::{rem_px, size};
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

/// 分组渲染函数：普通签名或需要窗口度量的签名。
type PlainGroup = fn(&mut SettingsView, &Palette, &mut Context<SettingsView>) -> AnyElement;
type WindowGroup =
    fn(&mut SettingsView, &mut Window, &Palette, &mut Context<SettingsView>) -> AnyElement;

enum GroupRender {
    Plain(PlainGroup),
    WithWindow(WindowGroup),
}

struct GroupSpec {
    search: GroupSearch,
    render: GroupRender,
}

struct PageSpec {
    title_key: &'static str,
    icon: IconName,
    groups: &'static [GroupSpec],
}

const fn plain(search: GroupSearch, render: PlainGroup) -> GroupSpec {
    GroupSpec {
        search,
        render: GroupRender::Plain(render),
    }
}

const fn window(search: GroupSearch, render: WindowGroup) -> GroupSpec {
    GroupSpec {
        search,
        render: GroupRender::WithWindow(render),
    }
}

/// 设置页签与分组目录；顺序即界面顺序。
static SETTINGS_PAGES: &[PageSpec] = &[
    PageSpec {
        title_key: "settings.tab_appearance",
        icon: IconName::Palette,
        groups: &[
            plain(GroupSearch::UiTheme, SettingsView::render_settings_ui_theme),
            plain(GroupSearch::UiFont, SettingsView::render_settings_ui_font),
            plain(
                GroupSearch::WindowAppearance,
                SettingsView::render_settings_window_appearance,
            ),
            plain(GroupSearch::Divider, SettingsView::render_settings_divider),
        ],
    },
    PageSpec {
        title_key: "settings.tab_terminal",
        icon: IconName::Terminal,
        groups: &[
            plain(
                GroupSearch::TerminalFont,
                SettingsView::render_settings_terminal_font,
            ),
            plain(
                GroupSearch::LocalShell,
                SettingsView::render_settings_local_shell,
            ),
            plain(GroupSearch::Cursor, SettingsView::render_settings_cursor),
            plain(
                GroupSearch::TerminalTheme,
                SettingsView::render_settings_themes,
            ),
            plain(
                GroupSearch::Highlight,
                SettingsView::render_settings_highlight,
            ),
            plain(GroupSearch::Trigger, SettingsView::render_settings_triggers),
            plain(
                GroupSearch::BlockView,
                SettingsView::render_settings_block_view,
            ),
            plain(
                GroupSearch::KeyBindings,
                SettingsView::render_settings_key_bindings,
            ),
        ],
    },
    PageSpec {
        title_key: "settings.tab_files",
        icon: IconName::Folder,
        groups: &[
            plain(GroupSearch::Editor, SettingsView::render_settings_editor),
            plain(
                GroupSearch::FilePanel,
                SettingsView::render_settings_file_panel,
            ),
            plain(
                GroupSearch::Transfer,
                SettingsView::render_settings_transfer,
            ),
            plain(GroupSearch::Zmodem, SettingsView::render_settings_zmodem),
            plain(GroupSearch::Audit, SettingsView::render_settings_audit),
            plain(
                GroupSearch::Identities,
                SettingsView::render_settings_identities,
            ),
            plain(GroupSearch::History, SettingsView::render_settings_history),
        ],
    },
    PageSpec {
        title_key: "settings.tab_mouse_clipboard",
        icon: IconName::ArrowLeftRight,
        groups: &[
            plain(GroupSearch::Mouse, SettingsView::render_settings_mouse),
            plain(
                GroupSearch::Selection,
                SettingsView::render_settings_selection,
            ),
            plain(
                GroupSearch::Clipboard,
                SettingsView::render_settings_clipboard,
            ),
        ],
    },
    PageSpec {
        title_key: "settings.tab_shortcuts",
        icon: IconName::Search,
        groups: &[plain(
            GroupSearch::Shortcuts,
            SettingsView::render_settings_shortcuts,
        )],
    },
    PageSpec {
        title_key: "settings.tab_window_layout",
        icon: IconName::PanelLeft,
        groups: &[
            plain(GroupSearch::Tabs, SettingsView::render_settings_tabs),
            plain(GroupSearch::Window, SettingsView::render_settings_window),
            plain(
                GroupSearch::ActivityBar,
                SettingsView::render_settings_activity_bar,
            ),
            plain(
                GroupSearch::StatusBar,
                SettingsView::render_settings_status_bar,
            ),
            plain(GroupSearch::Startup, SettingsView::render_settings_startup),
        ],
    },
    PageSpec {
        title_key: "settings.tab_general",
        icon: IconName::Globe,
        groups: GENERAL_PAGE_GROUPS,
    },
    PageSpec {
        title_key: "settings.tab_monitor",
        icon: IconName::Gauge,
        groups: &[
            plain(
                GroupSearch::MonitorSections,
                SettingsView::render_settings_monitor_sections,
            ),
            plain(
                GroupSearch::MonitorBehavior,
                SettingsView::render_settings_monitor_behavior,
            ),
        ],
    },
    PageSpec {
        title_key: "settings.tab_git",
        icon: IconName::GitBranch,
        groups: &[
            plain(
                GroupSearch::GitExecutable,
                SettingsView::render_settings_git_executable,
            ),
            plain(
                GroupSearch::GitCommitPush,
                SettingsView::render_settings_git_commit_push,
            ),
            plain(GroupSearch::GitDiff, SettingsView::render_settings_git_diff),
        ],
    },
    PageSpec {
        title_key: "settings.tab_containers",
        icon: IconName::Container,
        groups: &[plain(
            GroupSearch::ContainerRegistries,
            SettingsView::render_settings_container_registries,
        )],
    },
    PageSpec {
        title_key: "settings.tab_about",
        icon: IconName::Info,
        groups: &[
            window(GroupSearch::About, SettingsView::render_settings_about),
            plain(GroupSearch::Update, SettingsView::render_settings_update),
        ],
    },
];

/// 通用页分组：桌面集成只在 Linux 下出现，其余分组固定。
static GENERAL_PAGE_GROUPS: &[GroupSpec] = &[
    plain(
        GroupSearch::Language,
        SettingsView::render_settings_language,
    ),
    plain(GroupSearch::Notify, SettingsView::render_settings_notify),
    plain(GroupSearch::Proxy, SettingsView::render_settings_proxy),
    #[cfg(target_os = "linux")]
    plain(
        GroupSearch::DesktopIntegration,
        SettingsView::render_settings_desktop_integration,
    ),
    plain(GroupSearch::Backup, SettingsView::render_settings_backup),
    plain(GroupSearch::Sync, SettingsView::render_settings_cloud_sync),
    plain(GroupSearch::Logs, SettingsView::render_settings_logs),
];

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
        window: &mut Window,
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
            .sidebar_width(rem_px(13.75, window.rem_size()))
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
            .child(self.render_settings_footer(app, cx, p));

        panel.into_any_element()
    }

    /// 构建偏好设置所有分类页面。
    ///
    /// 页面与分组全部来自声明式表 [`SETTINGS_PAGES`]：顺序即页签顺序，
    /// 新增分组只需在表里补一行，不再在九个 `SettingPage` 链式调用中找位置。
    fn build_settings_pages(app: &Entity<SettingsView>, p: &Palette) -> Vec<SettingPage> {
        // 命令面板的设置页目录与这张表必须一致；顺序不一致会把跳转落到错页。
        debug_assert_eq!(super::page_entries().len(), SETTINGS_PAGES.len());
        let header_style = StyleRefinement::default()
            .border_color(p.divider(p.border))
            .divider_edge(DividerEdge::Bottom, p.divider_width);
        SETTINGS_PAGES
            .iter()
            .map(|page| {
                let mut built =
                    SettingPage::new(rust_i18n::t!(page.title_key).to_string()).icon(page.icon);
                for group in page.groups {
                    built = built.group(match group.render {
                        GroupRender::Plain(render) => setting_group(app, p, group.search, render),
                        GroupRender::WithWindow(render) => {
                            setting_group_with_window(app, p, group.search, render)
                        }
                    });
                }
                built.header_style(&header_style)
            })
            .collect()
    }

    /// 设置弹窗页脚：当前主题提示、取消与保存。
    fn render_settings_footer(
        &self,
        app: &Entity<SettingsView>,
        cx: &App,
        p: &Palette,
    ) -> AnyElement {
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
            )
            .into_any_element()
    }
}
