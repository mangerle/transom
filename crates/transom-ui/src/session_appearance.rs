//! 会话图标与环境色在界面层的映射。
//!
//! 核心层只定义"选哪一个图标、哪一种颜色"（枚举），这里负责把它翻译成界面能
//! 画的东西：图标资产名与色值。映射集中在一处，标签栏、侧栏与新建表单读的是
//! 同一份定义，不会出现"表单里选了数据库、标签栏画成服务器"的分裂。

use crate::app::TerminalTab;
use crate::app_assets::AppIcon;
use gpui_kit::assets::IconName;
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
