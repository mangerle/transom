//! 统一命令面板：会话、动作、设置页、主题与快捷命令的一处搜索。
//!
//! 设计原理（Why）：
//! - 实现初衷：`$app-k` 原先只能找本地终端、会话与直连地址，而录制、审计、
//!   触发器、键位表、主题分散在活动栏、菜单与设置里，新用户找不到入口。
//! - 核心优势：数据全部来自既有可枚举来源（见 [`items`]），面板本身不维护
//!   第二份动作清单，各入口新增能力时面板自动覆盖，杜绝两套目录漂移。
//! - 交互：输入 `>` 切到动作模式（只保留可执行条目），与 VS Code 的口径一致；
//!   Esc 先清查询再关闭面板，是组件自身的取消语义。

mod items;

use crate::app::AppShell;
use crate::palette::{Palette, localized_error};
use gpui_kit::base::v_flex;
use gpui_kit::component::command::Command;
use gpui_kit::component::theme::{ThemeMode, ThemeRegistry};
use gpui_kit::*;
use items::PaletteTarget;
use std::rc::Rc;
use transom_core::{UiThemeMode, preset_themes};

/// 动作模式前缀：输入 `>` 后只保留可执行条目，避免主机名混进动作列表。
const ACTION_MODE_PREFIX: char = '>';

impl AppShell {
    pub(crate) fn render_command_palette_modal(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let app_entity = cx.entity().clone();
        // 渲染期读取查询词：面板的条目集合由本函数按当前查询重建，因此
        // 只要 on_query 触发一次重绘，列表就与输入框内容保持同步。
        let query = self
            .modals
            .quick_open_state
            .read(cx)
            .query(cx)
            .trim()
            .to_string();
        let action_mode = query.starts_with(ACTION_MODE_PREFIX);
        // `>` 是单字节字符，按下标切片安全；后续内容按动作模式过滤。
        let term = if action_mode {
            query[ACTION_MODE_PREFIX.len_utf8()..].trim_start()
        } else {
            query.as_str()
        };

        let model = self.build_palette_model(action_mode, term, cx);
        let targets = Rc::new(model.targets);

        let placeholder = if action_mode {
            rust_i18n::t!("app.palette_action_placeholder").to_string()
        } else {
            rust_i18n::t!("app.quick_open_placeholder").to_string()
        };

        let muted = p.muted_foreground;
        let mut command = Command::new(&self.modals.quick_open_state)
            .placeholder(placeholder)
            // 外框由宿主面板提供：组件自带的圆角边框会在直角浮层里套出一层内圆角。
            .bordered(false)
            // 过滤在构建期完成：动作模式要剥离 `>` 前缀，本地过滤无法表达。
            .filterable(false)
            .empty(move |_, _, _| {
                div()
                    .p_4()
                    .text_xs()
                    .text_color(muted)
                    .child(rust_i18n::t!("app.palette_empty").to_string())
            })
            .on_query({
                let app = app_entity.clone();
                move |_query, _window, cx| {
                    // 查询变化只影响条目集合，重建由宿主重绘完成。
                    app.update(cx, |_, cx| cx.notify());
                }
            })
            .on_confirm({
                let app = app_entity.clone();
                let targets = targets.clone();
                move |index_path, window, cx| {
                    let target = targets
                        .get(index_path.section)
                        .and_then(|group| group.get(index_path.row))
                        .cloned();
                    app.update(cx, |this, cx| {
                        this.modals.show_quick_open = false;
                        match target {
                            Some(PaletteTarget::Local) => {
                                this.open_local_terminal(window, cx);
                            }
                            Some(PaletteTarget::Session(session_id)) => {
                                this.connect_session(session_id, window, cx);
                            }
                            Some(PaletteTarget::QuickConnect) => {
                                // 主机词以确认时刻的输入框为准，避免渲染期快照过期。
                                let host = this
                                    .modals
                                    .quick_open_state
                                    .read(cx)
                                    .query(cx)
                                    .trim()
                                    .to_string();
                                this.connect_quick_host(&host, cx);
                            }
                            Some(PaletteTarget::SettingsPage(page)) => {
                                this.open_settings_page(page, window, cx);
                            }
                            Some(PaletteTarget::UiTheme(name)) => {
                                this.apply_ui_theme_choice(&name, cx);
                            }
                            Some(PaletteTarget::TerminalTheme(name)) => {
                                this.apply_terminal_theme_choice(&name, cx);
                            }
                            Some(PaletteTarget::Snippet(id)) => {
                                let snippet = this
                                    .settings
                                    .snippets
                                    .items
                                    .iter()
                                    .find(|snippet| snippet.id == id)
                                    .cloned();
                                if let Some(snippet) = snippet {
                                    let execute = this.settings.snippets.execute_directly;
                                    this.apply_snippet(&snippet, execute, cx);
                                }
                            }
                            // 动作已由组件派发；None 表示索引失效，静默忽略。
                            Some(PaletteTarget::Action) | None => {}
                        }
                        cx.notify();
                    });
                }
            })
            .on_cancel({
                let app = app_entity.clone();
                move |_window, cx| {
                    app.update(cx, |this, cx| {
                        this.modals.show_quick_open = false;
                        cx.notify();
                    });
                }
            });

        for group in model.groups {
            command = command.group(group);
        }

        let panel = v_flex()
            .id(ElementId::Name("command-palette-modal".into()))
            .w(px(580.0))
            .rounded(p.radius.none)
            .bg(p.popover)
            .text_color(p.popover_foreground)
            .border_1()
            .border_color(p.border)
            .shadow_lg()
            .overflow_hidden()
            .child(command);

        Self::overlay(panel, p)
    }

    /// 应用界面主题并立即落盘。
    ///
    /// 与设置页的"选择即预览"不同：命令面板没有保存/取消的闭环，因此必须
    /// 当场持久化，否则用户以为已切换、重启后却回退。明暗模式跟随主题自带
    /// 属性，避免"选了深色主题但界面仍是浅色"的假死状态。
    fn apply_ui_theme_choice(&mut self, name: &str, cx: &mut Context<Self>) {
        let Some(config) = ThemeRegistry::global(cx)
            .sorted_themes()
            .into_iter()
            .find(|config| config.name.as_ref() == name)
            .cloned()
        else {
            log::warn!("命令面板选择界面主题失败，主题不在注册表中: {name}");
            return;
        };
        let mode = match config.mode {
            ThemeMode::Dark => UiThemeMode::Dark,
            ThemeMode::Light => UiThemeMode::Light,
        };
        self.update_appearance(
            |appearance| {
                appearance.ui_theme = name.to_string();
                appearance.mode = mode;
            },
            cx,
        );
        self.persist_theme_choice(cx);
    }

    /// 应用终端配色并热下发到全部窗格，随后落盘。
    fn apply_terminal_theme_choice(&mut self, name: &str, cx: &mut Context<Self>) {
        let Some(theme) = preset_themes().into_iter().find(|theme| theme.name == name) else {
            log::warn!("命令面板选择终端配色失败，配色不在预设中: {name}");
            return;
        };
        self.settings.terminal_profile.theme_name = theme.name;
        self.settings.terminal_profile.palette = theme.palette;
        self.apply_appearance_to_live_panes(cx);
        self.persist_theme_choice(cx);
    }

    fn persist_theme_choice(&mut self, cx: &mut Context<Self>) {
        if let Err(error) = self.services.settings_store.save(&self.settings) {
            log::error!("保存主题选择失败，界面仍是新主题但下次启动会回退，原因: {error}");
            self.set_error(localized_error(&error));
        }
        cx.notify();
    }
}
