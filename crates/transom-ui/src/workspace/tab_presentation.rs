//! 标签条目的图标与状态色口径。
//!
//! 设计原理（Why）：横向标签栏与纵向标签面板分别计算图标与状态点，纵向
//! 漏掉 ZMODEM 传输色、横向的图标回落顺序也不同；同一标签在两个入口可能
//! 显示不同颜色。把图标/状态色收敛为纯函数，两个入口只消费结果。
//!
//! 代价与局限：只覆盖视觉映射，标签布局仍各按横向/纵向排版。

use crate::app::{AppShell, TerminalTab};
use crate::app_assets::AppIcon;
use crate::palette::Palette;
use crate::session_appearance::{session_color, session_icon};
use crate::workspace::pane_status_indicator;
use gpui_kit::assets::IconName;
use gpui_kit::*;
use transom_core::{Connection, ContainerEngineKind, GitProject};

/// 标签图标与图标颜色：播放器/监控看板有专属图标，Git 标签跟随仓库项目外观，
/// 其余按会话外观回落连接类型。
pub(crate) fn tab_icon(
    shell: &AppShell,
    tab: &TerminalTab,
    active: bool,
    p: &Palette,
    cx: &App,
) -> (AppIcon, Hsla) {
    if tab.is_player() {
        return (IconName::Disc.into(), p.muted_foreground);
    }
    if tab.is_monitor() {
        return (IconName::Gauge.into(), p.primary);
    }
    if let Some(workspace) = tab.git() {
        let state = &workspace.read(cx).state;
        let icon = git_project_icon(&state.project);
        let color = git_project_color(&state.project).unwrap_or(if active {
            p.sidebar_accent_foreground
        } else {
            p.muted_foreground
        });
        return (icon, color);
    }
    if let Connection::ContainerEngine(profile) = &tab.session.connection {
        let icon = session_icon(profile.icon).unwrap_or_else(|| {
            if profile.engine.kind == ContainerEngineKind::Docker {
                AppIcon::Docker
            } else {
                IconName::Container.into()
            }
        });
        let color = session_color(profile.color).unwrap_or(if active {
            p.sidebar_accent_foreground
        } else {
            p.muted_foreground
        });
        return (icon, color);
    }
    let appearance = shell.state.session_appearance(tab);
    let icon = appearance
        .icon
        .unwrap_or_else(|| crate::connection::icon(&tab.session.connection).into());
    let color = appearance.color.unwrap_or(if active {
        p.sidebar_accent_foreground
    } else {
        p.muted_foreground
    });
    (icon, color)
}

/// 用户为会话配置的环境色；未配置时不画强调条。
pub(crate) fn tab_accent(shell: &AppShell, tab: &TerminalTab, cx: &App) -> Option<Hsla> {
    if let Some(workspace) = tab.git() {
        return git_project_color(&workspace.read(cx).state.project);
    }
    shell.state.session_appearance(tab).color
}

/// Git 标签图标与左侧仓库行保持一致：项目未配置图标时回落到分支图标。
fn git_project_icon(project: &GitProject) -> AppIcon {
    session_icon(project.icon).unwrap_or_else(|| IconName::GitBranch.into())
}

/// Git 项目配置的环境色；未配置时不画强调条。
fn git_project_color(project: &GitProject) -> Option<Hsla> {
    session_color(project.color)
}

/// 标签状态点颜色：ZMODEM 传输中优先用强调色，其余按连接状态。
pub(crate) fn tab_status_dot(tab: &TerminalTab, p: &Palette) -> Hsla {
    if tab.zmodem().is_some() {
        return p.primary;
    }
    pane_status_indicator(&tab.status, p).0
}

/// 标签显示标题：会话标题优先，分屏序号大于零时追加 `#N`。
pub(crate) fn tab_title(tab: &TerminalTab, serial: usize) -> String {
    let title = tab.tab_label();
    if serial > 0 {
        format!("{title} #{serial}")
    } else {
        title
    }
}

/// 标签副标题：连接类型的紧凑描述（本地/SSH/串口等）。
pub(crate) fn tab_subtitle(tab: &TerminalTab) -> String {
    crate::connection::display_compact(&tab.session.connection)
}

#[cfg(test)]
mod tests {
    use super::{git_project_color, git_project_icon};
    use crate::app_assets::AppIcon;
    use gpui_kit::assets::IconName;
    use std::path::PathBuf;
    use transom_core::{GitProject, SessionColor, SessionIcon};

    /// Git 项目未配置专属图标时，标签栏必须使用与仓库行相同的分支图标。
    #[test]
    fn git_project_default_icon_matches_repository_row() {
        let project = GitProject::new(PathBuf::from("/tmp/transom-git-icon"));
        assert!(matches!(
            git_project_icon(&project),
            AppIcon::Named(IconName::GitBranch)
        ));
    }

    /// 用户给仓库配置图标与环境色后，标签栏直接复用同一份外观。
    #[test]
    fn git_project_appearance_is_shared_with_tab() {
        let mut project = GitProject::new(PathBuf::from("/tmp/transom-git-icon"));
        project.icon = SessionIcon::Database;
        project.color = SessionColor::Blue;
        assert!(matches!(
            git_project_icon(&project),
            AppIcon::Named(IconName::Database)
        ));
        assert!(git_project_color(&project).is_some());
    }
}
