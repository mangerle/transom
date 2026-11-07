//! 偏好设置中心的页目录、通用控件与运行时外观下发。

mod about_page;
mod actions;
mod actions_collect;
mod actions_sync;
mod entity;
mod event;
mod handler;
mod search_keywords;

pub(crate) use entity::{SettingsServices, SettingsView};
pub(crate) use event::{SettingsEvent, SettingsRequest};
mod appearance_page;
mod containers_page;
mod files_page;
mod font_field;
mod font_select;
mod general_page;
mod git_page;
mod input_page;
mod layout_page;
mod logs_page;
mod modal;
mod modal_state;
mod monitor_page;
mod security_page;
mod shortcuts_page;
mod startup_page;
pub(crate) mod sync_form;
mod sync_page;
mod terminal_page;
mod terminal_themes;
mod window_page;

pub(crate) use font_field::FontFieldSpec;
pub(crate) use modal_state::{ProxyProtocol, SettingsModalState, WindowAppearanceSnapshot};

/// Git 设置页在偏好设置中心的分页序号。
pub(crate) const GIT_PAGE_INDEX: usize = 8;

use crate::app::AppShell;
use crate::app::app_settings;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::input::InputState;
use gpui_kit::component::slider::SliderValue;
use gpui_kit::*;
use transom_terminal::TerminalView;

/// 设置页目录：顺序必须与 [`SettingsView::build_settings_pages`] 的装配顺序一致。
///
/// 命令面板据此列出可跳转的设置页；标题与图标复用页面自身的定义，避免两处
/// 各写一份文案。`build_settings_pages` 里用 debug 断言守住顺序，出现漏改时
/// 在开发期立即暴露，而不是让面板把用户跳到错误的页。
pub(crate) fn page_entries() -> Vec<(&'static str, IconName)> {
    vec![
        ("settings.tab_appearance", IconName::Palette),
        ("settings.tab_terminal", IconName::Terminal),
        ("settings.tab_files", IconName::Folder),
        ("settings.tab_mouse_clipboard", IconName::ArrowLeftRight),
        ("settings.tab_shortcuts", IconName::Search),
        ("settings.tab_window_layout", IconName::PanelLeft),
        ("settings.tab_general", IconName::Globe),
        ("settings.tab_monitor", IconName::Gauge),
        ("settings.tab_git", IconName::GitBranch),
        ("settings.tab_containers", IconName::Container),
        ("settings.tab_about", IconName::Info),
    ]
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

impl SettingsView {
    /// 数字步进器控件（直接复用 gpui-kit 内置的 NumberInput 组件）。
    pub(crate) fn setting_stepper(input: &Entity<InputState>, width: Pixels) -> AnyElement {
        crate::widgets::number_stepper(input, width)
    }

    /// 通用设置项下拉选择器（Zed 风格）。
    pub(crate) fn setting_dropdown<T: Clone + PartialEq + 'static>(
        id: impl Into<ElementId>,
        current_value: T,
        options: Vec<(T, String)>,
        on_change: impl Fn(&mut Self, T, &mut Context<Self>) + 'static + Clone,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        Self::setting_dropdown_with_window(
            id,
            current_value,
            options,
            move |this, value, _window, cx| on_change(this, value, cx),
            cx,
        )
    }

    /// 需要窗口句柄的下拉：切项时要回填其它输入框的调用方用这条。
    pub(crate) fn setting_dropdown_with_window<T: Clone + PartialEq + 'static>(
        id: impl Into<ElementId>,
        current_value: T,
        options: Vec<(T, String)>,
        on_change: impl Fn(&mut Self, T, &mut Window, &mut Context<Self>) + 'static + Clone,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        crate::widgets::dropdown(id, current_value, options, on_change, cx.entity().clone())
    }

    /// 路径型设置值：定宽截断，完整路径挂在悬停 tooltip 上。
    pub(crate) fn setting_path_value(
        element_key: &'static str,
        label: String,
        p: &Palette,
    ) -> AnyElement {
        crate::widgets::path_value(element_key, label, p)
    }
}

/// 把一次外观变更写到全局设置并立即应用到主题。
///
/// 设置实体与命令面板共用这一条预览路径：写入、主题换装与终端配色计算都在
/// 这里完成，调用方只决定要不要把结果下发到活窗格。
pub(crate) fn preview_appearance(
    change: impl FnOnce(&mut transom_core::AppearanceSettings),
    cx: &mut gpui_kit::App,
) {
    let mut appearance = app_settings(cx).appearance.clone();
    change(&mut appearance);
    crate::app::update_app_settings(cx, |s| s.appearance = appearance.clone());
    crate::theme::apply_appearance(&appearance, cx);
}

/// 终端配色跟随界面主题：写回当前 UI 主题的伴生调色板，返回是否发生变化。
///
/// 只改全局设置，不碰任何窗格；调用方拿到 `true` 后自行决定下发范围。
pub(crate) fn sync_terminal_palette(cx: &mut gpui_kit::App) -> bool {
    if app_settings(cx).terminal_profile.theme_name != transom_core::FOLLOW_UI_THEME {
        return false;
    }
    let active = cx.theme().theme_name().to_string();
    let Some(palette) =
        crate::theme::resolve_terminal_palette(transom_core::FOLLOW_UI_THEME, &active, cx)
    else {
        return false;
    };
    crate::app::update_app_settings(cx, |s| s.terminal_profile.palette = palette);
    true
}

impl AppShell {
    /// 弹窗内的下拉选择：供会话表单等非设置实体复用。
    ///
    /// 与 [`SettingsView::setting_dropdown_with_window`] 共用同一份通用构建，
    /// 只把实体类型换成本类型。
    pub(crate) fn setting_dropdown_in_modal<T: Clone + PartialEq + 'static>(
        id: impl Into<ElementId>,
        current_value: T,
        options: Vec<(T, String)>,
        on_change: impl Fn(&mut Self, T, &mut Window, &mut Context<Self>) + 'static + Clone,
        app: &Entity<AppShell>,
    ) -> AnyElement {
        crate::widgets::dropdown(id, current_value, options, on_change, app.clone())
    }

    /// 把外观偏好下发到所有已打开的窗格。
    ///
    /// 不做这件事的话，改完字体要重开标签才见效，等于"设置了但没反应"。
    /// 会话本身也带 profile，但当前没有任何界面在会话级别改字体，因此以全局偏好为准；
    /// 将来开放会话级字体时，这里必须改成只补空白、不覆盖会话选择。
    pub(crate) fn apply_appearance_to_live_panes(&self, cx: &mut Context<Self>) {
        let mut profile = app_settings(cx).terminal_profile.clone();
        // 终端背景随窗口整体透明：窗口半透明时，终端区要透出桌面，
        // 否则只有侧栏与标签栏变透明，终端仍是一块实色。
        profile.background_transparency = app_settings(cx).window.window_transparency;
        // 先收集实体再逐个更新：遍历 self.state.workspace.tabs 与逐条 update 会同时借用 self。
        let terminals: Vec<Entity<TerminalView>> = self
            .state
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
        let global = app_settings(cx).block_view;
        let terminals: Vec<(Entity<TerminalView>, transom_core::BlockViewSettings)> = self
            .state
            .workspace
            .tabs
            .iter()
            .filter_map(|tab| {
                let terminal = tab.terminal()?;
                let override_ = self
                    .state
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

impl AppShell {
    /// 命令面板等非设置入口的外观切换：预览与终端配色共用的写路径不变，
    /// 变化后把终端外观下发到活窗格。
    pub(crate) fn update_appearance(
        &mut self,
        change: impl FnOnce(&mut transom_core::AppearanceSettings),
        cx: &mut Context<Self>,
    ) {
        preview_appearance(change, cx);
        if sync_terminal_palette(cx) {
            self.apply_appearance_to_live_panes(cx);
        }
        cx.notify();
    }
}
