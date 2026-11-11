//! 命令面板的条目构建：把五类既有来源转成可搜索的分组条目。
//!
//! 设计原理（Why）：
//! - 数据全部来自既有可枚举来源（`catalog.sessions`、`keymap::specs()`、设置页
//!   目录、主题注册表、快捷命令库），本模块不维护第二份动作清单，各入口新增
//!   能力时面板自动覆盖，杜绝两套目录漂移。
//! - 代价与局限：每个新能力需要一个"能被枚举"的入口；面板不做模糊评分，只做
//!   大小写无关的子串匹配，排序沿用来源顺序。

use crate::app::app_settings;
use crate::app::{AppShell, AppState};
use crate::keymap;
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::command::{CommandGroup, CommandItem};
use gpui_kit::component::theme::ThemeRegistry;
use gpui_kit::*;
use transom_core::AppSettings;
use transom_core::{SessionId, SnippetId, TabId, preset_themes};
use transom_ui_icons::AppIcon;
use transom_ui_icons::DomainIcon;
use transom_ui_icons::UiIcon;

/// 面板条目的落点；Action 条目的动作已挂在 `CommandItem` 上，由组件派发。
#[derive(Clone)]
pub(super) enum PaletteTarget {
    Local,
    Session(SessionId),
    QuickConnect,
    /// 条目自带 Action，确认时由组件直接派发，这里只负责关闭面板。
    Action,
    SettingsPage(usize),
    UiTheme(String),
    TerminalTheme(String),
    Snippet(SnippetId),
    /// 容器跳转目标：指定标签与容器 ID。
    Container {
        tab_id: TabId,
        container_id: String,
    },
    /// 容器直接启停动作：指定标签、容器名与生命周期动作。
    ContainerLifecycle {
        tab_id: TabId,
        container_name: String,
        action: transom_core::ContainerLifecycleAction,
    },
}

/// 一次渲染的完整模型：分组条目与逐条对应的落点。
///
/// 两者必须同序同步追加，确认回调才能用 `(section, row)` 找回正确的目标；
/// 因此只通过 [`PaletteModel::push`] 写入，不允许单独改任一侧。
pub(super) struct PaletteModel {
    pub(super) groups: Vec<CommandGroup>,
    pub(super) targets: Vec<Vec<PaletteTarget>>,
}

impl PaletteModel {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            groups: Vec::with_capacity(capacity),
            targets: Vec::with_capacity(capacity),
        }
    }

    fn push(&mut self, heading: String, items: Vec<(CommandItem, PaletteTarget)>) {
        if items.is_empty() {
            return;
        }
        let mut group = CommandGroup::new().label(heading);
        let mut targets = Vec::with_capacity(items.len());
        for (item, target) in items {
            group = group.item(item);
            targets.push(target);
        }
        self.groups.push(group);
        self.targets.push(targets);
    }
}

/// 大小写无关的子串匹配：空查询命中一切。
fn hit(needle: &str, label: &str, keywords: &[SharedString]) -> bool {
    needle.is_empty()
        || label.to_lowercase().contains(needle)
        || keywords
            .iter()
            .any(|keyword| keyword.to_lowercase().contains(needle))
}

impl AppState {
    fn session_items(&self, term: &str, needle: &str) -> Vec<(CommandItem, PaletteTarget)> {
        let mut items = Vec::with_capacity(self.catalog.sessions.len() + 2);
        let local_label = rust_i18n::t!("app.new_local").to_string();
        let local_keywords = vec![
            SharedString::from("local"),
            SharedString::from("terminal"),
            SharedString::from("bash"),
            SharedString::from("zsh"),
            SharedString::from("sh"),
        ];
        if hit(needle, &local_label, &local_keywords) {
            items.push((
                CommandItem::new()
                    .icon(AppIcon::Domain(DomainIcon::TERMINAL))
                    .label(local_label)
                    .keywords(local_keywords),
                PaletteTarget::Local,
            ));
        }

        for session in &self.catalog.sessions {
            let detail = crate::connection::display_with_port(&session.connection);
            let icon_name = crate::connection::icon(&session.connection);

            let mut keywords: Vec<SharedString> = vec![session.name.clone().into(), detail.into()];
            if let Some(folder_id) = session.folder_id
                && let Some(path) = self.catalog.folder_path(folder_id)
            {
                keywords.push(path.into());
            }
            if !hit(needle, &session.name, &keywords) {
                continue;
            }
            items.push((
                CommandItem::new()
                    .icon(icon_name)
                    .label(session.name.clone())
                    .keywords(keywords),
                PaletteTarget::Session(session.id),
            ));
        }

        // 免保存直连虚拟项：查询词本身形似主机地址时追加。动作模式不提供，
        // 那里的列表只表达"能执行什么"。
        if transom_core::looks_like_host(term) {
            items.push((
                CommandItem::new()
                    .icon(AppIcon::Ui(UiIcon::LANGUAGE))
                    .label(rust_i18n::t!("app.quick_connect", host = term).to_string())
                    .keywords(vec![SharedString::from(term)]),
                PaletteTarget::QuickConnect,
            ));
        }
        items
    }

    /// 遍历各容器工作区已拉取到的容器，转为可搜索与可直接操作的条目。
    pub(super) fn container_items(
        &self,
        needle: &str,
        cx: &App,
    ) -> Vec<(CommandItem, PaletteTarget)> {
        let mut items = Vec::new();
        for tab in self.workspace.tabs.iter() {
            let Some(workspace) = tab.containers() else {
                continue;
            };
            for container in &workspace.read(cx).state.containers {
                let name = &container.name;
                let image = &container.image;
                let state_str = container.state.key();
                let is_running = container.state == transom_core::ContainerState::Running;
                let keywords: Vec<SharedString> = vec![
                    name.clone().into(),
                    image.clone().into(),
                    state_str.into(),
                    SharedString::from("container"),
                    SharedString::from("docker"),
                ];

                // 1. 跳转选中该容器项
                if hit(needle, name, &keywords) {
                    let icon = if is_running {
                        AppIcon::Ui(UiIcon::RUN)
                    } else {
                        AppIcon::Ui(UiIcon::STOP)
                    };
                    items.push((
                        CommandItem::new()
                            .icon(icon)
                            .label(name.clone())
                            .keywords(keywords),
                        PaletteTarget::Container {
                            tab_id: tab.id,
                            container_id: container.id.clone(),
                        },
                    ));
                }

                // 2. 容器生命周期直接下发指令（启/停/重启）
                if is_running {
                    let stop_label =
                        rust_i18n::t!("app.palette_container_stop", name = name).to_string();
                    let stop_keywords = vec![
                        name.clone().into(),
                        SharedString::from("stop"),
                        SharedString::from(rust_i18n::t!("container.action.stop").to_string()),
                        SharedString::from("container"),
                    ];
                    if hit(needle, &stop_label, &stop_keywords) {
                        items.push((
                            CommandItem::new()
                                .icon(AppIcon::Ui(UiIcon::STOP))
                                .label(stop_label)
                                .keywords(stop_keywords),
                            PaletteTarget::ContainerLifecycle {
                                tab_id: tab.id,
                                container_name: name.clone(),
                                action: transom_core::ContainerLifecycleAction::Stop,
                            },
                        ));
                    }

                    let restart_label =
                        rust_i18n::t!("app.palette_container_restart", name = name).to_string();
                    let restart_keywords = vec![
                        name.clone().into(),
                        SharedString::from("restart"),
                        SharedString::from(rust_i18n::t!("container.action.restart").to_string()),
                        SharedString::from("container"),
                    ];
                    if hit(needle, &restart_label, &restart_keywords) {
                        items.push((
                            CommandItem::new()
                                .icon(AppIcon::Ui(UiIcon::REFRESH))
                                .label(restart_label)
                                .keywords(restart_keywords),
                            PaletteTarget::ContainerLifecycle {
                                tab_id: tab.id,
                                container_name: name.clone(),
                                action: transom_core::ContainerLifecycleAction::Restart,
                            },
                        ));
                    }
                } else {
                    let start_label =
                        rust_i18n::t!("app.palette_container_start", name = name).to_string();
                    let start_keywords = vec![
                        name.clone().into(),
                        SharedString::from("start"),
                        SharedString::from(rust_i18n::t!("container.action.start").to_string()),
                        SharedString::from("container"),
                    ];
                    if hit(needle, &start_label, &start_keywords) {
                        items.push((
                            CommandItem::new()
                                .icon(AppIcon::Ui(UiIcon::RUN))
                                .label(start_label)
                                .keywords(start_keywords),
                            PaletteTarget::ContainerLifecycle {
                                tab_id: tab.id,
                                container_name: name.clone(),
                                action: transom_core::ContainerLifecycleAction::Start,
                            },
                        ));
                    }
                }
            }
        }
        items
    }
}

impl AppShell {
    /// 按当前模式与查询词构建条目模型。
    ///
    /// 普通模式先给会话（含本地终端与直连），再给命令类条目；动作模式只保留
    /// 可执行条目——会话在动作模式里既不是动作也没有可派发的行为。
    pub(super) fn build_palette_model(
        &self,
        action_mode: bool,
        term: &str,
        cx: &App,
    ) -> PaletteModel {
        let needle = term.to_lowercase();
        let mut model = PaletteModel::with_capacity(7);

        if action_mode {
            // 动作模式把可执行项排在最前：用户敲 `>` 的目的就是执行。
            model.push(
                rust_i18n::t!("app.palette_actions").to_string(),
                action_items(&needle),
            );
            model.push(
                rust_i18n::t!("app.palette_snippets").to_string(),
                self.snippet_items(&needle, cx),
            );
        } else {
            model.push(
                rust_i18n::t!("app.sessions").to_string(),
                self.state.session_items(term, &needle),
            );
            model.push(
                rust_i18n::t!("app.palette_containers").to_string(),
                self.state.container_items(&needle, cx),
            );
            model.push(
                rust_i18n::t!("app.palette_snippets").to_string(),
                self.snippet_items(&needle, cx),
            );
            model.push(
                rust_i18n::t!("app.palette_actions").to_string(),
                action_items(&needle),
            );
        }
        model.push(
            rust_i18n::t!("app.palette_settings_pages").to_string(),
            settings_page_items(&needle),
        );
        model.push(
            rust_i18n::t!("app.palette_ui_themes").to_string(),
            ui_theme_items(&needle, cx),
        );
        model.push(
            rust_i18n::t!("app.palette_terminal_themes").to_string(),
            self.terminal_theme_items(&needle, app_settings(cx)),
        );
        model
    }

    fn snippet_items(&self, needle: &str, cx: &App) -> Vec<(CommandItem, PaletteTarget)> {
        app_settings(cx)
            .snippets
            .items
            .iter()
            .filter(|snippet| {
                hit(
                    needle,
                    &snippet.name,
                    &[SharedString::from(snippet.command.clone())],
                )
            })
            .map(|snippet| {
                (
                    CommandItem::new()
                        .icon(AppIcon::Ui(UiIcon::LIGHTNING))
                        .label(snippet.name.clone())
                        .keywords(vec![SharedString::from(snippet.command.clone())]),
                    PaletteTarget::Snippet(snippet.id),
                )
            })
            .collect()
    }
    fn terminal_theme_items(
        &self,
        needle: &str,
        settings: &AppSettings,
    ) -> Vec<(CommandItem, PaletteTarget)> {
        let active = settings.terminal_profile.theme_name.as_str();
        // 中文关键词取本地化标题，避免把界面文案硬编码进源码。
        let group_keyword = rust_i18n::t!("app.palette_terminal_themes").to_string();
        preset_themes()
            .into_iter()
            .filter(|theme| {
                hit(
                    needle,
                    &theme.name,
                    &[
                        SharedString::from("terminal"),
                        SharedString::from(group_keyword.clone()),
                    ],
                )
            })
            .map(|theme| {
                let name = theme.name;
                (
                    CommandItem::new()
                        .icon(AppIcon::Ui(UiIcon::LAYOUT))
                        .label(name.clone())
                        .keywords(vec![
                            SharedString::from(name.clone()),
                            SharedString::from("theme"),
                        ])
                        .checked(name.as_str() == active),
                    PaletteTarget::TerminalTheme(name),
                )
            })
            .collect()
    }
}

/// 动作条目：直接携带真实 Action，组件会自动关联并展示当前键位。
fn action_items(needle: &str) -> Vec<(CommandItem, PaletteTarget)> {
    keymap::specs()
        .iter()
        // 面板自身不出现在面板里：选中它只会把当前浮层再关掉。
        .filter(|spec| spec.id != "quick_open")
        .filter_map(|spec| {
            let label = rust_i18n::t!(spec.label_key).to_string();
            let keywords = vec![
                SharedString::from(spec.id),
                SharedString::from(label.clone()),
            ];
            hit(needle, &label, &keywords).then(|| {
                (
                    CommandItem::new()
                        .icon(AppIcon::Ui(UiIcon::RUN_ANYTHING))
                        .label(label)
                        .keywords(keywords)
                        .action((spec.action)()),
                    PaletteTarget::Action,
                )
            })
        })
        .collect()
}

/// 设置页条目：顺序与设置弹窗的装配顺序一致，确认后直接跳到该页。
fn settings_page_items(needle: &str) -> Vec<(CommandItem, PaletteTarget)> {
    // 中文关键词取分组标题的本地化值，避免源码里出现硬编码文案。
    let group_keyword = rust_i18n::t!("app.palette_settings_pages").to_string();
    crate::settings::page_entries()
        .into_iter()
        .enumerate()
        .filter_map(|(page, (label_key, icon))| {
            let label = rust_i18n::t!(label_key).to_string();
            let keywords = vec![
                SharedString::from(label.clone()),
                SharedString::from("settings"),
                SharedString::from(group_keyword.clone()),
            ];
            hit(needle, &label, &keywords).then(|| {
                (
                    CommandItem::new()
                        .icon(icon)
                        .label(label)
                        .keywords(keywords),
                    PaletteTarget::SettingsPage(page),
                )
            })
        })
        .collect()
}

/// 界面主题条目：当前生效主题打勾，确认后立即应用。
fn ui_theme_items(needle: &str, cx: &App) -> Vec<(CommandItem, PaletteTarget)> {
    let active = cx.theme().theme_name().to_string();
    let group_keyword = rust_i18n::t!("app.palette_ui_themes").to_string();
    ThemeRegistry::global(cx)
        .sorted_themes()
        .into_iter()
        .filter(|config| {
            hit(
                needle,
                &config.name,
                &[
                    SharedString::from("ui"),
                    SharedString::from(group_keyword.clone()),
                ],
            )
        })
        .map(|config| {
            let name = config.name.to_string();
            (
                CommandItem::new()
                    .icon(AppIcon::Ui(UiIcon::LAYOUT))
                    .label(name.clone())
                    .keywords(vec![
                        SharedString::from(name.clone()),
                        SharedString::from("theme"),
                    ])
                    .checked(name == active),
                PaletteTarget::UiTheme(name),
            )
        })
        .collect()
}
