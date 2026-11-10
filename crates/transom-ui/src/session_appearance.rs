//! 会话图标与环境色在界面层的映射。
//!
//! 核心层只定义"选哪一个图标、哪一种颜色"（枚举），这里负责把它翻译成界面能
//! 画的东西：图标资产名与色值。映射集中在一处，标签栏、侧栏与新建表单读的是
//! 同一份定义，不会出现"表单里选了数据库、标签栏画成服务器"的分裂。

use crate::app::{AppShell, TerminalTab};
use crate::app_assets::AppIcon;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{Selectable, h_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::Button;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::*;
use transom_core::{SessionColor, SessionIcon};

/// 一个会话在界面上的外观：图标与环境色。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct SessionAppearance {
    /// 专属图标；`None` 表示按连接类型取默认图标。
    pub(crate) icon: Option<AppIcon>,
    /// 环境色；`None` 表示不染色。
    pub(crate) color: Option<Hsla>,
}

/// 表单里可选的会话图标（不含"跟随连接类型"）。
pub(crate) const SESSION_ICON_CHOICES: [SessionIcon; 10] = [
    SessionIcon::Server,
    SessionIcon::Database,
    SessionIcon::Container,
    SessionIcon::Cloud,
    SessionIcon::Terminal,
    SessionIcon::Globe,
    SessionIcon::Shield,
    SessionIcon::Storage,
    SessionIcon::Docker,
    SessionIcon::Podman,
];

/// 表单里可选的环境色（不含"不染色"）。
pub(crate) const SESSION_COLOR_CHOICES: [SessionColor; 7] = [
    SessionColor::Red,
    SessionColor::Orange,
    SessionColor::Yellow,
    SessionColor::Green,
    SessionColor::Blue,
    SessionColor::Purple,
    SessionColor::Gray,
];

/// 会话图标对应的界面图标；`Auto` 返回 `None`，由调用方按连接类型取默认图标。
pub(crate) fn session_icon(icon: SessionIcon) -> Option<AppIcon> {
    match icon {
        SessionIcon::Auto => None,
        SessionIcon::Server => Some(IconName::Server.into()),
        SessionIcon::Database => Some(IconName::Database.into()),
        SessionIcon::Container => Some(IconName::Container.into()),
        SessionIcon::Cloud => Some(IconName::Cloud.into()),
        SessionIcon::Terminal => Some(IconName::Terminal.into()),
        SessionIcon::Globe => Some(IconName::Globe.into()),
        SessionIcon::Shield => Some(IconName::Shield.into()),
        SessionIcon::Storage => Some(IconName::HardDrive.into()),
        SessionIcon::Docker => Some(AppIcon::Docker),
        SessionIcon::Podman => Some(AppIcon::Podman),
    }
}

/// 环境色对应的色值；`None` 表示不染色。
///
/// 色值是固定的环境语义色，不随界面主题变化：环境色要能在浅色与深色主题下
/// 都读得出"这是生产机"，跟主题走反而会在换肤后失去含义。
pub(crate) fn session_color(color: SessionColor) -> Option<Hsla> {
    match color {
        SessionColor::None => None,
        SessionColor::Red => Some(hsla(0.0, 0.72, 0.55, 1.0)),
        SessionColor::Orange => Some(hsla(0.07, 0.85, 0.55, 1.0)),
        SessionColor::Yellow => Some(hsla(0.13, 0.85, 0.55, 1.0)),
        SessionColor::Green => Some(hsla(0.33, 0.62, 0.45, 1.0)),
        SessionColor::Blue => Some(hsla(0.58, 0.70, 0.58, 1.0)),
        SessionColor::Purple => Some(hsla(0.75, 0.60, 0.60, 1.0)),
        SessionColor::Gray => Some(hsla(0.0, 0.0, 0.55, 1.0)),
    }
}

impl crate::app::AppState {
    /// 标签对应的会话外观。
    ///
    /// 优先读目录里的会话定义：改了图标与颜色，已打开的标签立刻跟着变，不必
    /// 重连；本地终端没有目录条目，用标签自带的会话快照。
    pub(crate) fn session_appearance(&self, tab: &TerminalTab) -> SessionAppearance {
        let session = self
            .catalog
            .sessions
            .iter()
            .find(|session| session.id == tab.session_id)
            .unwrap_or(&tab.session);
        SessionAppearance {
            icon: session_icon(session.icon),
            color: session_color(session.color),
        }
    }
}

/// 色块直径。
const SWATCH_SIZE: f32 = 18.0;

/// 图标选择行（通用）。
pub(crate) fn render_appearance_icon_row<F>(
    id_prefix: &'static str,
    default_icon: IconName,
    icon: SessionIcon,
    on_icon: F,
    app: &Entity<AppShell>,
    _cx: &App,
) -> AnyElement
where
    F: Fn(&mut AppShell, SessionIcon, &mut Context<AppShell>) + Copy + 'static,
{
    let auto_app = app.clone();
    h_flex()
        .gap_1()
        .child(
            Button::new(ElementId::Name(format!("{id_prefix}-icon-auto").into()))
                .small()
                .icon(default_icon)
                .selected(icon == SessionIcon::Auto)
                .accessibility_label(rust_i18n::t!("form.icon_auto").to_string())
                .tooltip(rust_i18n::t!("form.icon_auto").to_string())
                .on_click(move |_, _window, cx| {
                    auto_app.update(cx, |this, cx| {
                        on_icon(this, SessionIcon::Auto, cx);
                    });
                }),
        )
        .children(SESSION_ICON_CHOICES.into_iter().map(|choice| {
            let Some(name) = session_icon(choice) else {
                return div().into_any_element();
            };
            let choice_app = app.clone();
            Button::new(ElementId::Name(
                format!("{id_prefix}-icon-{choice:?}").into(),
            ))
            .small()
            .icon(name)
            .accessibility_label(format!("{choice:?}"))
            .selected(icon == choice)
            .on_click(move |_, _window, cx| {
                choice_app.update(cx, |this, cx| {
                    on_icon(this, choice, cx);
                });
            })
            .into_any_element()
        }))
        .into_any_element()
}

/// 环境色选择行（通用）。
pub(crate) fn render_appearance_color_row<G>(
    id_prefix: &'static str,
    color: SessionColor,
    on_color: G,
    p: &Palette,
    app: &Entity<AppShell>,
    _cx: &App,
) -> AnyElement
where
    G: Fn(&mut AppShell, SessionColor, &mut Context<AppShell>) + Copy + 'static,
{
    let none_app = app.clone();
    h_flex()
        .gap_2()
        .child(
            div()
                .id(ElementId::Name(format!("{id_prefix}-color-none").into()))
                .size(px(SWATCH_SIZE))
                .rounded_full()
                .border_1()
                .border_color(if color == SessionColor::None {
                    p.primary
                } else {
                    p.border
                })
                .bg(p.background)
                .cursor_pointer()
                .tooltip(move |_window, cx| {
                    Tooltip::new(rust_i18n::t!("form.color_none").to_string()).build(_window, cx)
                })
                .on_click(move |_, _window, cx| {
                    none_app.update(cx, |this, cx| {
                        on_color(this, SessionColor::None, cx);
                    });
                })
                .into_any_element(),
        )
        .children(SESSION_COLOR_CHOICES.into_iter().map(|choice| {
            let Some(value) = session_color(choice) else {
                return div().into_any_element();
            };
            let choice_app = app.clone();
            div()
                .id(ElementId::Name(
                    format!("{id_prefix}-color-{choice:?}").into(),
                ))
                .size(px(SWATCH_SIZE))
                .rounded_full()
                .bg(value)
                .border_1()
                .border_color(if color == choice {
                    p.foreground
                } else {
                    p.border
                })
                .cursor_pointer()
                .on_click(move |_, _window, cx| {
                    choice_app.update(cx, |this, cx| {
                        on_color(this, choice, cx);
                    });
                })
                .into_any_element()
        }))
        .into_any_element()
}
