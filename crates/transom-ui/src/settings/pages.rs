//! 偏好设置中心之主题配色页、鼠标剪贴板页与标签页窗口页渲染。

use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::button::Button;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::theme::{ThemeConfig, ThemeMode, ThemeRegistry, try_parse_color};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::rc::Rc;
use transom_core::{
    FOLLOW_UI_THEME, PaletteMode, PanelSide, UiDensity, UiThemeMode, preset_themes,
};

impl AppShell {
    pub(crate) fn render_settings_themes(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let themes = preset_themes();
        let current_theme_name = self.settings.terminal_profile.theme_name.clone();

        // "跟随界面主题"卡：终端配色默认随 UI 主题明暗联动（Islands Dark 配
        // Islands 终端色），显式选预设即覆盖。置于列表首位。
        let follow_selected = current_theme_name == FOLLOW_UI_THEME;
        let follow_card = v_flex()
            .id(ElementId::Name("terminal-follow-card".into()))
            .w_full()
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(if follow_selected {
                p.sidebar_accent_foreground
            } else {
                p.border
            })
            .bg(if follow_selected {
                p.sidebar_accent
            } else {
                p.popover
            })
            .gap_2p5()
            .cursor_pointer()
            .hover(|s| {
                if follow_selected {
                    s
                } else {
                    s.border_color(p.muted_foreground)
                }
            })
            .on_click(cx.listener(|this, _, _, cx| {
                this.settings.terminal_profile.theme_name = FOLLOW_UI_THEME.to_string();
                this.sync_terminal_palette(cx);
                cx.notify();
            }))
            .child(
                h_flex().w_full().justify_between().items_center().child(
                    div()
                        .text_sm()
                        .font_weight(if follow_selected {
                            FontWeight::BOLD
                        } else {
                            FontWeight::NORMAL
                        })
                        .child(rust_i18n::t!("settings.terminal_follow_ui").to_string()),
                ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("settings.terminal_follow_ui_note").to_string()),
            )
            .into_any_element();

        let theme_cards: Vec<AnyElement> = themes
            .into_iter()
            .enumerate()
            .map(|(idx, theme)| {
                let is_selected = theme.name == current_theme_name;
                let theme_clone = theme.clone();
                let theme_name = theme.name.clone();

                let dots = h_flex()
                    .gap_1()
                    .children(theme.palette.ansi.iter().take(8).map(|color| {
                        div()
                            .size(px(10.0))
                            .rounded_full()
                            .bg(Hsla::from(rgb(color.to_hex())))
                    }));

                v_flex()
                    .id(ElementId::NamedInteger("theme-card".into(), idx as u64))
                    .w_full()
                    .p_3()
                    .rounded(p.radius.lg)
                    .border_1()
                    .border_color(if is_selected {
                        p.sidebar_accent_foreground
                    } else {
                        p.border
                    })
                    .bg(if is_selected {
                        p.sidebar_accent
                    } else {
                        p.popover
                    })
                    .gap_2p5()
                    .cursor_pointer()
                    .hover(|s| {
                        if is_selected {
                            s
                        } else {
                            s.border_color(p.muted_foreground)
                        }
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings.terminal_profile.theme_name = theme_clone.name.clone();
                        this.settings.terminal_profile.palette = theme_clone.palette.clone();
                        cx.notify();
                    }))
                    .child(
                        h_flex()
                            .w_full()
                            .justify_between()
                            .items_center()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        div()
                                            .size(px(12.0))
                                            .rounded_full()
                                            .border_2()
                                            .border_color(if is_selected {
                                                p.sidebar_accent_foreground
                                            } else {
                                                p.border
                                            })
                                            .bg(if is_selected {
                                                p.sidebar_accent_foreground
                                            } else {
                                                transparent_black()
                                            }),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::BOLD)
                                            .child(theme_name),
                                    ),
                            )
                            .child(dots),
                    )
                    .child(
                        v_flex()
                            .w_full()
                            .p_2p5()
                            .rounded(p.radius.md)
                            .bg(Hsla::from(rgb(theme.palette.background.to_hex())))
                            .text_xs()
                            .gap_1()
                            .child(
                                h_flex()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_color(Hsla::from(rgb(
                                                theme.palette.ansi[2].to_hex()
                                            )))
                                            .child("user@transom"),
                                    )
                                    .child(
                                        div()
                                            .text_color(Hsla::from(rgb(theme
                                                .palette
                                                .foreground
                                                .to_hex())))
                                            .child(":"),
                                    )
                                    .child(
                                        div()
                                            .text_color(Hsla::from(rgb(
                                                theme.palette.ansi[4].to_hex()
                                            )))
                                            .child("~"),
                                    )
                                    .child(
                                        div()
                                            .text_color(Hsla::from(rgb(theme
                                                .palette
                                                .foreground
                                                .to_hex())))
                                            .child("$ ls -la"),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .gap_3()
                                    .child(
                                        h_flex()
                                            .gap_1()
                                            .child(Icon::new(IconName::Folder).size_3().text_color(
                                                Hsla::from(rgb(theme.palette.ansi[4].to_hex())),
                                            ))
                                            .child(
                                                div()
                                                    .font_weight(FontWeight::BOLD)
                                                    .text_color(Hsla::from(rgb(theme.palette.ansi
                                                        [4]
                                                    .to_hex())))
                                                    .child("Documents"),
                                            ),
                                    )
                                    .child(
                                        h_flex()
                                            .gap_1()
                                            .child(Icon::new(IconName::Folder).size_3().text_color(
                                                Hsla::from(rgb(theme.palette.ansi[2].to_hex())),
                                            ))
                                            .child(
                                                div()
                                                    .font_weight(FontWeight::BOLD)
                                                    .text_color(Hsla::from(rgb(theme.palette.ansi
                                                        [2]
                                                    .to_hex())))
                                                    .child("Downloads"),
                                            ),
                                    )
                                    .child(
                                        h_flex()
                                            .gap_1()
                                            .child(Icon::new(IconName::File).size_3().text_color(
                                                Hsla::from(rgb(theme.palette.foreground.to_hex())),
                                            ))
                                            .child(
                                                div()
                                                    .text_color(Hsla::from(rgb(theme
                                                        .palette
                                                        .foreground
                                                        .to_hex())))
                                                    .child("main.rs"),
                                            ),
                                    ),
                            ),
                    )
                    .into_any_element()
            })
            .collect();

        // 组标题由 SettingGroup 提供（"终端配色"），页内不再重复标题行。
        v_flex()
            .gap_3()
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("settings.terminal_theme_note").to_string()),
            )
            .child(
                v_flex()
                    .gap_2p5()
                    .children([follow_card])
                    .children(theme_cards),
            )
            .into_any_element()
    }

    /// 界面主题组：明暗模式、密度档位与全局主题卡片。
    ///
    /// 选择即预览：每次改动经 [`AppShell::update_appearance`] 立即应用到全局
    /// 主题，保存/取消分别负责落盘与回滚。选择主题卡会把明暗模式一并切到该
    /// 主题自带的属性——主题本身携带明暗语义，只记名字不改模式会出现"选了
    /// 深色主题但界面还是浅色"的假死状态。
    pub(crate) fn render_settings_ui_theme(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let themes: Vec<Rc<ThemeConfig>> = ThemeRegistry::global(cx)
            .sorted_themes()
            .into_iter()
            .cloned()
            .collect();
        let active_name = cx.theme().theme_name().clone();

        let mode_row = h_flex()
            .justify_between()
            .items_center()
            .py_1()
            .child(
                div()
                    .text_sm()
                    .child(rust_i18n::t!("settings.ui_mode").to_string()),
            )
            .child(
                h_flex().gap_1().children(
                    [
                        (
                            "ui-mode-system",
                            "settings.ui_mode_system",
                            UiThemeMode::System,
                        ),
                        (
                            "ui-mode-light",
                            "settings.ui_mode_light",
                            UiThemeMode::Light,
                        ),
                        ("ui-mode-dark", "settings.ui_mode_dark", UiThemeMode::Dark),
                    ]
                    .into_iter()
                    .map(|(id, key, mode)| {
                        Button::new(id)
                            .small()
                            .label(rust_i18n::t!(key).to_string())
                            .selected(self.settings.appearance.mode == mode)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.update_appearance(|appearance| appearance.mode = mode, cx);
                            }))
                    }),
                ),
            );

        let density_row = h_flex()
            .justify_between()
            .items_center()
            .py_1()
            .child(
                div()
                    .text_sm()
                    .child(rust_i18n::t!("settings.density").to_string()),
            )
            .child(
                h_flex()
                    .gap_1()
                    .children(UiDensity::ALL.into_iter().map(|density| {
                        Button::new(density.label_key())
                            .small()
                            .label(rust_i18n::t!(density.label_key()).to_string())
                            .selected(self.settings.appearance.density == density)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.update_appearance(
                                    |appearance| appearance.density = density,
                                    cx,
                                );
                            }))
                    })),
            );

        let cards: Vec<AnyElement> = themes
            .iter()
            .enumerate()
            .map(|(idx, config)| {
                let is_active = config.name == active_name;
                let name = config.name.to_string();
                // 点击闭包要拿走一份名称，卡片标题继续用原值，各持一份。
                let click_name = name.clone();
                let mode = config.mode;
                let mode_label = match mode {
                    ThemeMode::Dark => rust_i18n::t!("settings.theme_mode_dark").to_string(),
                    ThemeMode::Light => rust_i18n::t!("settings.theme_mode_light").to_string(),
                };
                h_flex()
                    .id(ElementId::NamedInteger("ui-theme-card".into(), idx as u64))
                    .w_full()
                    .px_3()
                    .py_2()
                    .gap_3()
                    .items_center()
                    .rounded(p.radius.md)
                    .border_1()
                    .border_color(if is_active { p.primary } else { p.border })
                    .bg(if is_active { p.muted } else { p.popover })
                    .cursor_pointer()
                    .hover(|s| {
                        if is_active {
                            s
                        } else {
                            s.border_color(p.muted_foreground)
                        }
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let theme_mode = match mode {
                            ThemeMode::Dark => UiThemeMode::Dark,
                            ThemeMode::Light => UiThemeMode::Light,
                        };
                        this.update_appearance(
                            |appearance| {
                                appearance.ui_theme = click_name.clone();
                                appearance.mode = theme_mode;
                            },
                            cx,
                        );
                    }))
                    .child(theme_color_dots(config))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(if is_active {
                                FontWeight::BOLD
                            } else {
                                FontWeight::NORMAL
                            })
                            .child(name),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(mode_label),
                    )
                    .into_any_element()
            })
            .collect();

        // 明暗反差提示：界面主题与终端配色一个深一个浅时给出文案，不改用户选择。
        let terminal_mode = preset_themes()
            .into_iter()
            .find(|theme| theme.name == self.settings.terminal_profile.theme_name)
            .map(|theme| theme.mode);
        let contrast_hint = matches!(
            (cx.theme().is_dark(), terminal_mode),
            (true, Some(PaletteMode::Light)) | (false, Some(PaletteMode::Dark))
        );

        v_flex()
            .gap_3()
            .child(mode_row)
            .child(density_row)
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("settings.ui_theme_note").to_string()),
            )
            .child(v_flex().gap_2().children(cards))
            .when(contrast_hint, |this| {
                this.child(
                    h_flex()
                        .gap_2()
                        .items_start()
                        .child(
                            Icon::new(IconName::TriangleAlert)
                                .size_3()
                                .text_color(p.warning),
                        )
                        .child(
                            div()
                                .flex_1()
                                .text_xs()
                                .text_color(p.warning)
                                .child(rust_i18n::t!("settings.appearance_contrast").to_string()),
                        ),
                )
            })
            .into_any_element()
    }
}

/// 从主题配置解析三个预览色点（背景、前景、强调色）；解析失败显示透明点。
///
/// 主题文件的颜色值是字符串（hex 或 Tailwind 名），交给 gpui-kit 的解析器
/// 统一处理，解析规则与实际应用主题时一致，避免预览与真实效果色差。
fn theme_color_dots(config: &ThemeConfig) -> AnyElement {
    let entries = [
        config.colors.background.as_deref(),
        config.colors.foreground.as_deref(),
        config.colors.primary.as_deref(),
    ];
    h_flex()
        .gap_1()
        .children(entries.into_iter().map(|value| {
            let color = value.and_then(|value| try_parse_color(value).ok());
            div()
                .size(px(10.0))
                .rounded_full()
                .bg(color.unwrap_or(transparent_black()))
        }))
        .into_any_element()
}

impl AppShell {
    /// 内置编辑器组：软换行与行号显示。
    pub(crate) fn render_settings_editor(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .gap_2()
            .child(Self::setting_row(
                "settings.editor_soft_wrap",
                Some("settings.editor_soft_wrap_desc"),
                Switch::new("toggle-editor-soft-wrap")
                    .small()
                    .checked(self.settings.sftp.editor_soft_wrap)
                    .on_click(cx.listener(|this, &checked, _, cx| {
                        this.settings.sftp.editor_soft_wrap = checked;
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .child(Self::setting_row(
                "settings.editor_line_numbers",
                Some("settings.editor_line_numbers_desc"),
                Switch::new("toggle-editor-line-numbers")
                    .small()
                    .checked(self.settings.sftp.editor_line_numbers)
                    .on_click(cx.listener(|this, &checked, _, cx| {
                        this.settings.sftp.editor_line_numbers = checked;
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .into_any_element()
    }

    /// 文件面板组：显示排除规则（只影响可见性，不拦截传输）。
    pub(crate) fn render_settings_file_panel(
        &mut self,
        p: &Palette,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .gap_2()
            .child(Self::form_field(
                rust_i18n::t!("settings.panel_hidden").to_string(),
                self.settings_modal.panel_hidden_patterns.clone(),
                p,
            ))
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("settings.panel_hidden_note").to_string()),
            )
            .into_any_element()
    }

    /// 传输组：并发上限与完成后校验。
    pub(crate) fn render_settings_transfer(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .gap_2()
            .child(Self::form_field(
                rust_i18n::t!("settings.max_concurrent_transfers").to_string(),
                self.settings_modal.max_concurrent_transfers.clone(),
                p,
            ))
            .child(Self::setting_row(
                "settings.verify_after_transfer",
                Some("settings.verify_after_transfer_desc"),
                Switch::new("toggle-verify-after-transfer")
                    .small()
                    .checked(self.settings.sftp.verify_after_transfer)
                    .on_click(cx.listener(|this, &checked, _, cx| {
                        this.settings.sftp.verify_after_transfer = checked;
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .into_any_element()
    }

    /// ZMODEM 组：下载目录与自动接收开关。
    ///
    /// 与其余设置一致采用"即时改内存、保存时统一落盘"；下载目录为空表示
    /// 跟随系统下载目录，选择后写入显式路径。
    pub(crate) fn render_settings_zmodem(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let resolved = self.settings.zmodem.resolved_download_dir();
        let dir_label = match self.settings.zmodem.download_dir.clone() {
            Some(dir) => dir.display().to_string(),
            None => format!(
                "{}（{}）",
                rust_i18n::t!("settings.zmodem_dir_default"),
                resolved.display()
            ),
        };
        let browse = cx.listener(|this, _, _, _cx| {
            if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                this.settings.zmodem.download_dir = Some(dir);
            }
        });
        v_flex()
            .gap_2()
            .child(Self::setting_row(
                "settings.zmodem_dir",
                Some("settings.zmodem_dir_desc"),
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .max_w(rems(24.0))
                            .truncate()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(dir_label),
                    )
                    .child(
                        Button::new("zmodem-dir-browse")
                            .small()
                            .label(rust_i18n::t!("settings.zmodem_dir_browse").to_string())
                            .on_click(browse),
                    )
                    .into_any_element(),
                p,
            ))
            .child(Self::setting_row(
                "settings.zmodem_auto_receive",
                Some("settings.zmodem_auto_receive_desc"),
                Switch::new("toggle-zmodem-auto-receive")
                    .small()
                    .checked(self.settings.zmodem.auto_receive)
                    .on_click(cx.listener(|this, &checked, _, cx| {
                        this.settings.zmodem.auto_receive = checked;
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .into_any_element()
    }

    /// 历史组：命令采集与建议开关。
    pub(crate) fn render_settings_history(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .gap_2()
            .child(Self::setting_row(
                "settings.history_enabled",
                Some("settings.history_enabled_desc"),
                Switch::new("toggle-history-enabled")
                    .small()
                    .checked(self.settings.history.enabled)
                    .on_click(cx.listener(|this, &checked, _, cx| {
                        this.settings.history.enabled = checked;
                        // 关掉时把内存里的建议一起收起：下次开回来仍从文件恢复，
                        // 不做删除，避免"关一下就把历史清空"的误伤。
                        if !checked {
                            this.history.write().clear();
                        } else if this.history.read().is_empty() {
                            let loaded = this.services.history.load();
                            *this.history.write() = loaded;
                        }
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .into_any_element()
    }

    /// 审计组：会话输出落盘开关与日志目录入口。
    pub(crate) fn render_settings_audit(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let dir = transom_core::paths::audit_dir();
        let reveal_dir = dir.clone();
        let reveal = cx.listener(move |this, _, _, cx| {
            crate::audit::reveal_in_file_manager(reveal_dir.clone());
            this.status = Some(rust_i18n::t!("settings.audit_revealed").to_string());
            cx.notify();
        });
        v_flex()
            .gap_2()
            .child(Self::setting_row(
                "settings.audit_enabled",
                Some("settings.audit_enabled_desc"),
                Switch::new("toggle-audit-enabled")
                    .small()
                    .checked(self.settings.audit.enabled)
                    .on_click(cx.listener(|this, &checked, _, cx| {
                        this.settings.audit.enabled = checked;
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .child(Self::setting_row(
                "settings.audit_dir",
                Some("settings.audit_dir_desc"),
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .max_w(rems(24.0))
                            .truncate()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(dir.display().to_string()),
                    )
                    .child(
                        Button::new("audit-dir-reveal")
                            .small()
                            .label(rust_i18n::t!("settings.audit_dir_open").to_string())
                            .on_click(reveal),
                    )
                    .into_any_element(),
                p,
            ))
            .into_any_element()
    }

    /// 鼠标组：右键动作与中键粘贴。
    pub(crate) fn render_settings_mouse(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let is_paste_mode = self.settings.terminal_interaction.right_click_paste;
        v_flex()
            .gap_2()
            .child(Self::setting_row(
                "settings.right_click_action",
                Some("settings.right_click_action_desc"),
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("rc-menu")
                            .small()
                            .label(rust_i18n::t!("settings.right_click_menu").to_string())
                            .selected(!is_paste_mode)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.settings.terminal_interaction.right_click_paste = false;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("rc-paste")
                            .small()
                            .label(rust_i18n::t!("settings.right_click_paste_action").to_string())
                            .selected(is_paste_mode)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.settings.terminal_interaction.right_click_paste = true;
                                cx.notify();
                            })),
                    )
                    .into_any_element(),
                p,
            ))
            .child(Self::setting_row(
                "settings.middle_click_paste",
                Some("settings.middle_click_paste_desc"),
                Switch::new("toggle-middle-click-paste")
                    .small()
                    .checked(self.settings.terminal_interaction.middle_click_paste)
                    .on_click(cx.listener(|this, &checked, _, cx| {
                        this.settings.terminal_interaction.middle_click_paste = checked;
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .into_any_element()
    }

    /// 选择组：双击分词边界。
    pub(crate) fn render_settings_selection(
        &mut self,
        p: &Palette,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .gap_2()
            .child(Self::form_field(
                rust_i18n::t!("settings.word_separators").to_string(),
                self.settings_modal.word_separators.clone(),
                p,
            ))
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("settings.word_separators_note").to_string()),
            )
            .into_any_element()
    }

    /// 剪贴板组：选中复制、多行粘贴确认与尾随空白清理。
    pub(crate) fn render_settings_clipboard(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .gap_2()
            .child(Self::setting_row(
                "settings.copy_on_select",
                Some("settings.copy_on_select_desc"),
                Switch::new("toggle-copy-on-select")
                    .small()
                    .checked(self.settings.terminal_interaction.copy_on_select)
                    .on_click(cx.listener(|this, &checked, _, cx| {
                        this.settings.terminal_interaction.copy_on_select = checked;
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .child(Self::setting_row(
                "settings.multiline_paste",
                Some("settings.multiline_paste_desc"),
                Switch::new("toggle-multiline-paste")
                    .small()
                    .checked(self.settings.terminal_interaction.multiline_paste_confirm)
                    .on_click(cx.listener(|this, &checked, _, cx| {
                        this.settings.terminal_interaction.multiline_paste_confirm = checked;
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .child(Self::setting_row(
                "settings.trim_trailing_whitespace",
                Some("settings.trim_trailing_whitespace_desc"),
                Switch::new("toggle-trim-whitespace")
                    .small()
                    .checked(self.settings.terminal_interaction.trim_trailing_whitespace)
                    .on_click(cx.listener(|this, &checked, _, cx| {
                        this.settings.terminal_interaction.trim_trailing_whitespace = checked;
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .into_any_element()
    }

    /// 标签页组：关闭按钮与最后标签退出行为。
    pub(crate) fn render_settings_tabs(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .gap_2()
            .child(Self::setting_row(
                "settings.show_tab_close_button",
                Some("settings.show_tab_close_button_desc"),
                Switch::new("toggle-tab-close-btn")
                    .small()
                    .checked(self.settings.window.show_tab_close_button)
                    .on_click(cx.listener(|this, &checked, _, cx| {
                        this.settings.window.show_tab_close_button = checked;
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .child(Self::setting_row(
                "settings.close_window_on_all_tabs_closed",
                Some("settings.close_window_on_all_tabs_closed_desc"),
                Switch::new("toggle-close-on-last-tab")
                    .small()
                    .checked(self.settings.window.close_window_on_all_tabs_closed)
                    .on_click(cx.listener(|this, &checked, _, cx| {
                        this.settings.window.close_window_on_all_tabs_closed = checked;
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .into_any_element()
    }

    /// 窗口组：停靠方向、窗口记忆与会话恢复。
    pub(crate) fn render_settings_window(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .gap_2()
            .child(Self::setting_row(
                "settings.sftp_panel_side",
                Some("settings.sftp_panel_side_desc"),
                h_flex()
                    .gap_1()
                    .children(
                        [
                            (
                                "panel-side-left",
                                "settings.panel_side_left",
                                PanelSide::Left,
                            ),
                            (
                                "panel-side-right",
                                "settings.panel_side_right",
                                PanelSide::Right,
                            ),
                        ]
                        .into_iter()
                        .map(|(id, key, side)| {
                            Button::new(id)
                                .small()
                                .label(rust_i18n::t!(key).to_string())
                                .selected(self.settings.window.sftp_panel_side == side)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.set_sftp_panel_side(side, cx);
                                }))
                        }),
                    )
                    .into_any_element(),
                p,
            ))
            .child(Self::setting_row(
                "settings.remember_window_bounds",
                Some("settings.remember_window_bounds_desc"),
                Switch::new("toggle-remember-bounds")
                    .small()
                    .checked(self.settings.window.remember_window_bounds)
                    .on_click(cx.listener(|this, &checked, _, cx| {
                        this.settings.window.remember_window_bounds = checked;
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .child(Self::setting_row(
                "settings.restore_previous_tabs",
                Some("settings.restore_previous_tabs_desc"),
                Switch::new("toggle-restore-tabs")
                    .small()
                    .checked(self.settings.window.restore_previous_tabs)
                    .on_click(cx.listener(|this, &checked, _, cx| {
                        this.settings.window.restore_previous_tabs = checked;
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .into_any_element()
    }
}
