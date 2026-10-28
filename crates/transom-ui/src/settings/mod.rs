//! 偏好设置中心的通用布局行组件与运行时外观下发逻辑。

mod about_page;
mod actions;
mod appearance_page;
mod files_page;
mod general_page;
mod input_page;
mod layout_page;
mod logs_page;
mod modal;
mod modal_state;
mod security_page;
mod shortcuts_page;
mod startup_page;
mod sync_page;
mod terminal_page;
mod window_page;

pub(crate) use modal_state::{ProxyProtocol, SettingsModalState, WindowAppearanceSnapshot};

use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::input::{InputState, NumberInput};
use gpui_kit::component::slider::SliderValue;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::{AppearanceSettings, FOLLOW_UI_THEME};
use transom_terminal::TerminalView;

/// 路径型设置值的显示上限（rem，标准档 16px 基准）：够看清目录层级，又不会把
/// 右侧控件挤出面板；全文由 tooltip 给出。
const PATH_VALUE_MAX_WIDTH: f32 = 12.0;

/// 读取字体族输入框：留空表示"不指定"，交给默认字体或系统回退。
pub(super) fn read_font_family(input: &Entity<InputState>, cx: &App) -> Option<String> {
    let value = input.read(cx).value();
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// 滑块取值折算成百分比整数。
///
/// 区间滑块返回的是范围，这里只用单值档；出现范围时退回 0，避免把区间下界
/// 当成百分比显示。
pub(super) fn slider_percent(value: &SliderValue) -> u8 {
    match value {
        SliderValue::Single(value) => value.round().clamp(0.0, 100.0) as u8,
        SliderValue::Range(_, _) => 0,
    }
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

    /// 数字步进器控件（直接复用 gpui-kit 内置的 NumberInput 组件）：
    /// 原生集成减加微调按钮与居中文本输入框，内置键盘上下键微调与 min/max/step 约束。
    pub(crate) fn setting_stepper(input: &Entity<InputState>, width: Pixels) -> AnyElement {
        div()
            .w(width)
            .child(NumberInput::new(input).small())
            .into_any_element()
    }

    /// 路径型设置值：定宽截断，完整路径挂在悬停 tooltip 上。
    ///
    /// 也用于项目主页这类"长而必须看全"的只读值（URL 同样按内容排版会撑爆整行）。
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

    /// 把外观偏好下发到所有已打开的窗格。
    ///
    /// 不做这件事的话，改完字体要重开标签才见效，等于"设置了但没反应"。
    /// 会话本身也带 profile，但当前没有任何界面在会话级别改字体，因此以全局偏好为准；
    /// 将来开放会话级字体时，这里必须改成只补空白、不覆盖会话选择。
    pub(crate) fn apply_appearance_to_live_panes(&self, cx: &mut Context<Self>) {
        let mut profile = self.settings.terminal_profile.clone();
        // 终端背景随窗口整体透明：窗口半透明时，终端区要透出桌面，
        // 否则只有侧栏与标签栏变透明，终端仍是一块实色。
        profile.background_transparency = self.settings.window.window_transparency;
        // 先收集实体再逐个更新：遍历 self.workspace.tabs 与逐条 update 会同时借用 self。
        let terminals: Vec<Entity<TerminalView>> = self
            .workspace
            .tabs
            .iter()
            .filter_map(|tab| tab.terminal())
            .collect();
        for terminal in terminals {
            terminal.update(cx, |view, cx| view.apply_appearance(&profile, cx));
        }
    }

    /// 把语义块偏好热下发给所有运行中的窗格。
    ///
    /// 每个窗格按自己会话的覆盖档位解析，不能统一吃全局值：生产构建机可能
    /// 强制开启，而同一窗口里的交换机 Console 强制关闭。
    pub(crate) fn apply_block_view_settings(&self, cx: &mut Context<Self>) {
        let global = self.settings.block_view;
        let terminals: Vec<(Entity<TerminalView>, transom_core::BlockViewSettings)> = self
            .workspace
            .tabs
            .iter()
            .filter_map(|tab| {
                let terminal = tab.terminal()?;
                let override_ = self
                    .catalog
                    .sessions
                    .iter()
                    .find(|session| session.id == tab.session_id)
                    .map_or(tab.session.block_view, |session| session.block_view);
                Some((terminal, global.resolved(override_)))
            })
            .collect();
        for (terminal, settings) in terminals {
            terminal.update(cx, |view, cx| view.set_block_view(Some(settings), cx));
        }
    }
}
