//! 偏好设置中心的动作入口与通用/渲染/快捷键配置页模块。

mod about_page;
mod general_page;
mod layout_page;
mod logs_page;
mod modal;
mod pages;
mod shortcuts_page;
mod startup_page;
mod sync_page;

use crate::app::AppShell;
use crate::palette::{Palette, localized_error};
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::InputState;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::{AppearanceSettings, FOLLOW_UI_THEME, TerminalCursorShape, TerminalHighlight};
use transom_terminal::TerminalView;

/// 路径型设置值的显示上限（rem，标准档 16px 基准）：够看清目录层级，又不会把
/// 右侧控件挤出面板；全文由 tooltip 给出。
const PATH_VALUE_MAX_WIDTH: f32 = 12.0;

/// 读取字体族输入框：留空表示"不指定"，交给默认字体或系统回退。
fn read_font_family(input: &Entity<InputState>, cx: &App) -> Option<String> {
    let value = input.read(cx).value();
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

impl AppShell {
    /// Zed 风格设置行：左侧标题与说明、右侧控件。
    ///
    /// 全设置页的开关/按钮行共用这一构造，行高与对齐才一致；说明文字用弱色
    /// 小字，缺省时标题与控件垂直居中。输入类行仍用 `form_field` 纵排（输入框
    /// 需要完整宽度，且多带独立说明）。
    ///
    /// 宽度约束是这里的重点：左列必须 `flex_1 + min_w_0`，控件必须 `flex_none`。
    /// 少任一条，弹性布局就会按内容固有宽度排版——一句长中文说明能把整行撑到比
    /// 面板内容区还宽，右侧开关/按钮被推到面板外截断（审计目录、命令历史、日志
    /// 目录都踩过这个坑）。左列拿到有界宽度后说明文字自然换行，而不是溢出裁切。
    pub(crate) fn setting_row(
        title_key: &'static str,
        description_key: Option<&'static str>,
        control: AnyElement,
        p: &Palette,
    ) -> AnyElement {
        h_flex()
            .w_full()
            .py_1()
            .justify_between()
            .items_center()
            .gap_4()
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_0p5()
                    .child(div().text_sm().child(rust_i18n::t!(title_key).to_string()))
                    .when_some(description_key, |this, key| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(p.muted_foreground)
                                .child(rust_i18n::t!(key).to_string()),
                        )
                    }),
            )
            .child(div().flex_none().child(control))
            .into_any_element()
    }

    /// 路径型设置值：定宽截断，完整路径挂在悬停 tooltip 上。
    ///
    /// 也用于项目主页这类"长而必须看全"的只读值（URL 同样按内容排版会撑爆整行）。
    ///
    /// 为什么定宽：配置目录、下载目录、审计目录都可能是任意长路径，按内容排版
    /// 会把整行撑爆（与上面 `setting_row` 的约束配合，这里再给一个明确上限）。
    /// 为什么要 tooltip：截断后用户看不出"到底写到哪个目录"，而这条信息恰恰是
    /// 排查落盘问题时最需要的，所以把全文挂在悬停上；悬浮状态依赖带 `id` 的元素。
    pub(crate) fn setting_path_value(
        element_key: &'static str,
        label: String,
        p: &Palette,
    ) -> AnyElement {
        let tip = label.clone();
        div()
            .id(ElementId::Name(
                format!("setting-path-{element_key}").into(),
            ))
            .max_w(rems(PATH_VALUE_MAX_WIDTH))
            .truncate()
            .text_xs()
            .text_color(p.muted_foreground)
            .tooltip(move |_window, cx| Tooltip::new(tip.clone()).build(_window, cx))
            .child(label)
            .into_any_element()
    }

    /// 直接操作型设置（活动栏、状态栏、快捷命令执行模式）的即时落盘入口。
    ///
    /// 拖拽或显隐发生在设置弹窗之外，不能等待弹窗的保存按钮；这里只做保存与
    /// 错误上报，不重复解析字体等输入框状态，也不改变弹窗的取消语义。
    pub(crate) fn persist_layout_settings(&mut self, cx: &mut Context<Self>) {
        if let Err(error) = self.services.settings_store.save(&self.settings) {
            log::error!("保存布局设置失败，界面仍是新值但下次启动会回退，原因: {error}");
            self.error = Some(localized_error(&error));
        }
        cx.notify();
    }

    pub(crate) fn open_settings_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let profile = self.settings.terminal_profile.clone();
        // 界面主题与密度在弹窗内即时预览，取消时凭这份快照恢复。
        self.settings_modal.appearance_snapshot = Some(self.settings.appearance.clone());
        self.settings_modal.font_family.update(cx, |input, cx| {
            input.set_value(profile.font_family.unwrap_or_default(), window, cx);
        });
        self.settings_modal.ui_font_family.update(cx, |input, cx| {
            input.set_value(
                self.settings
                    .appearance
                    .ui_font_family
                    .clone()
                    .unwrap_or_default(),
                window,
                cx,
            );
        });
        self.settings_modal.cjk_font.update(cx, |input, cx| {
            input.set_value(profile.cjk_font_family.unwrap_or_default(), window, cx);
        });
        let font_size = self.settings.terminal_profile.font_size.to_string();
        self.settings_modal.font_size.update(cx, |input, cx| {
            input.set_value(&font_size, window, cx);
        });
        let line_height = self.settings.terminal_profile.line_height.to_string();
        self.settings_modal.line_height.update(cx, |input, cx| {
            input.set_value(&line_height, window, cx);
        });
        self.settings_modal.word_separators.update(cx, |input, cx| {
            input.set_value(
                self.settings.terminal_profile.word_separators.clone(),
                window,
                cx,
            );
        });
        self.settings_modal
            .highlight_keywords
            .update(cx, |input, cx| {
                input.set_value(profile.highlight.keywords_joined(), window, cx);
            });
        let notify_threshold = self.settings.window.notify_threshold_secs.to_string();
        self.settings_modal
            .notify_threshold
            .update(cx, |input, cx| {
                input.set_value(notify_threshold, window, cx);
            });
        let scrollback = self.settings.terminal_profile.scrollback.to_string();
        self.settings_modal.scrollback.update(cx, |input, cx| {
            input.set_value(&scrollback, window, cx);
        });
        self.fill_sync_form(window, cx);
        self.settings_modal.open = true;
        cx.notify();
    }

    /// 更新界面外观偏好并即时应用：弹窗内改一档立刻全局预览。
    ///
    /// 落盘仍由 `save_settings` 单点负责；取消走 `cancel_settings` 凭快照回滚。
    /// 预览与回滚共用 [`crate::theme::apply_appearance`] 这一条写路径，保证
    /// "看到的效果"与"保存后的效果"出自同一逻辑。
    pub(crate) fn update_appearance(
        &mut self,
        change: impl FnOnce(&mut AppearanceSettings),
        cx: &mut Context<Self>,
    ) {
        change(&mut self.settings.appearance);
        crate::theme::apply_appearance(&self.settings.appearance, cx);
        self.sync_terminal_palette(cx);
        cx.notify();
    }

    /// 终端配色跟随界面主题：`theme_name` 为哨兵值时，把当前生效 UI 主题的
    /// 伴生终端调色板写入终端 profile 并热应用到活窗格。显式选了预设的用户
    /// 不受影响。设置页外观变化、保存落盘、系统明暗切换三个触发点共用。
    pub(crate) fn sync_terminal_palette(&mut self, cx: &mut Context<Self>) {
        if self.settings.terminal_profile.theme_name != FOLLOW_UI_THEME {
            return;
        }
        let active = cx.theme().theme_name().to_string();
        if let Some(palette) = crate::theme::resolve_terminal_palette(FOLLOW_UI_THEME, &active, cx)
        {
            self.settings.terminal_profile.palette = palette;
            self.apply_appearance_to_live_panes(cx);
        }
    }

    /// 取消设置：外观部分恢复到打开弹窗前的偏好并回滚预览效果。
    ///
    /// 为什么只有外观回滚：它引入了"未保存也生效"的即时预览，取消语义必须
    /// 成对闭环，否则点取消会留下改了一半的界面；其余设置项维持既有的内存态
    /// 修改保留、等待下次保存覆盖的行为。
    pub(crate) fn cancel_settings(&mut self, cx: &mut Context<Self>) {
        if let Some(snapshot) = self.settings_modal.appearance_snapshot.take() {
            self.settings.appearance = snapshot;
            crate::theme::apply_appearance(&self.settings.appearance, cx);
        }
        self.settings_modal.open = false;
        cx.notify();
    }

    pub(crate) fn save_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // 界面字体族：留空表示跟随主题。
        self.settings.appearance.ui_font_family =
            read_font_family(&self.settings_modal.ui_font_family, cx);
        // 文件面板显示排除：原样保存（glob 语法与传输过滤一致，非法规则在
        // 匹配时被静默跳过，不阻断保存）。
        self.settings.sftp.panel_hidden_patterns = self
            .settings_modal
            .panel_hidden_patterns
            .read(cx)
            .value()
            .to_string();
        if let Ok(max) = self
            .settings_modal
            .max_concurrent_transfers
            .read(cx)
            .value()
            .trim()
            .parse::<usize>()
            && (1..=16).contains(&max)
        {
            self.settings.sftp.max_concurrent_transfers = max;
        }
        // 云同步：凭据先落钥匙串（非空才写），端点拓扑清洗后进设置文件。
        if self.persist_sync_credentials(window, cx) {
            self.settings.sync = self.collect_sync_settings(cx);
            self.settings.sync.normalize();
        }
        // 外观偏好在弹窗内已即时写入，落盘前归一化兜底值，保证文件可回读。
        self.settings.appearance.normalize();
        self.settings.terminal_profile.font_family =
            read_font_family(&self.settings_modal.font_family, cx);
        self.settings.terminal_profile.cjk_font_family =
            read_font_family(&self.settings_modal.cjk_font, cx);
        // 关键词列表先解析再清洗（去空、不分大小写去重、限长限条数），
        // 让"界面上看到的"与"存进文件的"是同一份。
        self.settings.terminal_profile.highlight.keywords = TerminalHighlight {
            enabled: self.settings.terminal_profile.highlight.enabled,
            keywords: TerminalHighlight::parse_keywords(
                self.settings_modal
                    .highlight_keywords
                    .read(cx)
                    .value()
                    .as_ref(),
            ),
        }
        .sanitized()
        .keywords;
        if let Ok(font_size) = self
            .settings_modal
            .font_size
            .read(cx)
            .value()
            .trim()
            .parse::<f32>()
            && (8.0..=36.0).contains(&font_size)
        {
            self.settings.terminal_profile.font_size = font_size;
        }
        if let Ok(line_height) = self
            .settings_modal
            .line_height
            .read(cx)
            .value()
            .trim()
            .parse::<f32>()
            && (1.0..=2.5).contains(&line_height)
        {
            self.settings.terminal_profile.line_height = line_height;
        }
        // 空串是有值意义的（不按任何字符断词），因此这里不 trim、也不拒绝空值。
        self.settings.terminal_profile.word_separators = self
            .settings_modal
            .word_separators
            .read(cx)
            .value()
            .to_string();
        if let Some(threshold) = crate::notify::parse_notify_threshold(
            self.settings_modal
                .notify_threshold
                .read(cx)
                .value()
                .as_ref(),
        ) {
            self.settings.window.notify_threshold_secs = threshold;
        }
        if let Ok(scrollback) = self
            .settings_modal
            .scrollback
            .read(cx)
            .value()
            .trim()
            .parse::<usize>()
            && (100..=100_000).contains(&scrollback)
        {
            self.settings.terminal_profile.scrollback = scrollback;
        }
        self.sync_terminal_palette(cx);
        if let Err(error) = self.services.settings_store.save(&self.settings) {
            log::error!("保存偏好设置失败，界面仍是新值但下次启动会回退，原因: {error}");
            self.error = Some(localized_error(&error));
        } else {
            log::info!(
                "偏好设置已保存，语言: {}，日志级别: {}",
                self.settings.language,
                self.settings.logs.level.as_str()
            );
            self.apply_appearance_to_live_panes(cx);
            self.apply_shortcut_settings(cx);
            self.status = Some(rust_i18n::t!("status.ready").to_string());
        }
        self.settings_modal.open = false;
        cx.notify();
    }

    /// 把外观偏好下发到所有已打开的窗格。
    ///
    /// 不做这件事的话，改完字体要重开标签才见效，等于"设置了但没反应"。
    /// 会话本身也带 profile，但当前没有任何界面在会话级别改字体，因此以全局偏好为准；
    /// 将来开放会话级字体时，这里必须改成只补空白、不覆盖会话选择。
    fn apply_appearance_to_live_panes(&self, cx: &mut Context<Self>) {
        let profile = self.settings.terminal_profile.clone();
        // 先收集实体再逐个更新：遍历 self.tabs 与逐条 update 会同时借用 self。
        let terminals: Vec<Entity<TerminalView>> = self
            .tabs
            .iter()
            .filter_map(|tab| tab.terminal.clone())
            .collect();
        for terminal in terminals {
            terminal.update(cx, |view, cx| view.apply_appearance(&profile, cx));
        }
    }

    /// 界面字体组：界面字体族覆盖；字号由配色方案页的密度档驱动。
    pub(crate) fn render_settings_ui_font(
        &mut self,
        p: &Palette,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .gap_2()
            .child(Self::form_field(
                rust_i18n::t!("settings.ui_font_family").to_string(),
                self.settings_modal.ui_font_family.clone(),
                p,
            ))
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("settings.ui_font_note").to_string()),
            )
            .into_any_element()
    }

    /// 终端字体组：终端字体族、CJK 字体、字号与行高。
    pub(crate) fn render_settings_terminal_font(
        &mut self,
        p: &Palette,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .gap_2()
            .child(Self::form_field(
                rust_i18n::t!("settings.font_family").to_string(),
                self.settings_modal.font_family.clone(),
                p,
            ))
            .child(Self::form_field(
                rust_i18n::t!("settings.cjk_font").to_string(),
                self.settings_modal.cjk_font.clone(),
                p,
            ))
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("settings.cjk_font_note").to_string()),
            )
            .child(Self::form_field(
                rust_i18n::t!("settings.font_size").to_string(),
                self.settings_modal.font_size.clone(),
                p,
            ))
            .child(Self::form_field(
                rust_i18n::t!("settings.line_height").to_string(),
                self.settings_modal.line_height.clone(),
                p,
            ))
            .into_any_element()
    }

    /// 光标与滚动组：光标形态、闪烁、粗体亮色、输入滚动与回滚行数。
    pub(crate) fn render_settings_cursor(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .gap_2()
            .child(Self::setting_row(
                "settings.cursor_shape",
                Some("settings.cursor_shape_desc"),
                h_flex()
                    .gap_1()
                    .children(
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
                    )
                    .into_any_element(),
                p,
            ))
            .child(Self::setting_row(
                "settings.cursor_blink",
                Some("settings.cursor_blink_desc"),
                Switch::new("toggle-cursor-blink")
                    .small()
                    .checked(self.settings.terminal_profile.cursor_blink)
                    .on_click(cx.listener(|this, &checked, _, cx| {
                        this.settings.terminal_profile.cursor_blink = checked;
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .child(Self::setting_row(
                "settings.bold_is_bright",
                Some("settings.bold_is_bright_desc"),
                Switch::new("toggle-bold-is-bright")
                    .small()
                    .checked(self.settings.terminal_profile.bold_is_bright)
                    .on_click(cx.listener(|this, &checked, _, cx| {
                        this.settings.terminal_profile.bold_is_bright = checked;
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .child(Self::setting_row(
                "settings.alternate_scroll",
                Some("settings.alternate_scroll_desc"),
                Switch::new("toggle-alternate-scroll")
                    .small()
                    .checked(self.settings.terminal_profile.alternate_scroll)
                    .on_click(cx.listener(|this, &checked, _, cx| {
                        this.settings.terminal_profile.alternate_scroll = checked;
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .child(Self::setting_row(
                "settings.scroll_on_input",
                Some("settings.scroll_on_input_desc"),
                Switch::new("toggle-scroll-on-input")
                    .small()
                    .checked(self.settings.terminal_profile.scroll_on_input)
                    .on_click(cx.listener(|this, &checked, _, cx| {
                        this.settings.terminal_profile.scroll_on_input = checked;
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .child(Self::form_field(
                rust_i18n::t!("settings.scrollback").to_string(),
                self.settings_modal.scrollback.clone(),
                p,
            ))
            .into_any_element()
    }

    /// 关键词高亮组：命中高亮开关与关键词列表。
    pub(crate) fn render_settings_highlight(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .gap_3()
            .child(Self::setting_row(
                "settings.keyword_highlight",
                Some("settings.keyword_highlight_desc"),
                Switch::new("toggle-keyword-highlight")
                    .checked(self.settings.terminal_profile.highlight.enabled)
                    .on_click(cx.listener(|this, &checked, _, cx| {
                        this.settings.terminal_profile.highlight.enabled = checked;
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .child(Self::form_field(
                rust_i18n::t!("settings.highlight_keywords").to_string(),
                self.settings_modal.highlight_keywords.clone(),
                p,
            ))
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("settings.highlight_keywords_note").to_string()),
            )
            .into_any_element()
    }
}
