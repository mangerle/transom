//! 横向与纵向标签共享的展示快照。
//!
//! 设计原理（Why）：标签标题、图标、状态点、强调色、可分屏能力和会话菜单必须
//! 横向标签栏与纵向标签面板使用同一口径；在这里一次性解析，避免两个入口各写
//! 一套 `TabContent` 分支后逐渐漂移。
//!
//! 代价与局限：Git 图标与强调色需要读取工作区实体，因此生成快照需要 `App`；
//! 快照本身是拥有型数据，不能跨帧复用，但可在同一标签条目的多个子元素间共享。

use super::{SessionMenuData, TabContent, TabProgress, TabStatus, WorkspaceTab};
use crate::app::AppState;
use crate::palette::Palette;
use crate::session_appearance::{session_color, session_icon};
use crate::workspace::pane_status_indicator;
use gpui_kit::*;
use transom_core::{ContainerEngineKind, GitProject};
use transom_ui_icons::AppIcon;
use transom_ui_icons::{CustomIcon, DomainIcon, UiIcon};

/// 单个标签条目的完整展示数据。
pub(crate) struct TabPresentation {
    pub(crate) title: String,
    pub(crate) subtitle: String,
    pub(crate) icon: AppIcon,
    pub(crate) icon_color: Hsla,
    pub(crate) status: TabStatus,
    pub(crate) status_color: Hsla,
    /// OSC 9;4 上报的任务进度；横向与纵向标签栏按同一口径渲染。
    pub(crate) progress: Option<TabProgress>,
    pub(crate) accent: Option<Hsla>,
    pub(crate) can_split: bool,
    pub(crate) closeable: bool,
    pub(crate) session_menu: Option<SessionMenuData>,
}

impl WorkspaceTab {
    /// 按标签类型生成展示快照。
    pub(crate) fn presentation(
        &self,
        state: &AppState,
        serial: usize,
        active: bool,
        palette: &Palette,
        cx: &App,
    ) -> TabPresentation {
        let (icon, icon_color) = tab_icon(state, self, active, palette, cx);
        let mut title = self.tab_label();
        if serial > 0 {
            title = format!("{title} #{serial}");
        }
        TabPresentation {
            title,
            subtitle: tab_subtitle(self),
            icon,
            icon_color,
            status: self.status.clone(),
            status_color: tab_status_dot(self, palette),
            progress: self.progress(),
            accent: tab_accent(state, self, cx),
            can_split: self.can_split(),
            closeable: true,
            session_menu: self.session_menu(),
        }
    }
}

/// 标签图标与图标颜色：播放器/监控看板有专属图标，Git 标签跟随仓库项目外观，
/// 其余按会话外观回落连接类型。
pub(crate) fn tab_icon(
    state: &AppState,
    tab: &WorkspaceTab,
    active: bool,
    p: &Palette,
    cx: &App,
) -> (AppIcon, Hsla) {
    if tab.is_player() {
        return (AppIcon::Domain(DomainIcon::MONITOR), p.muted_foreground);
    }
    if tab.is_monitor() {
        return (AppIcon::Domain(DomainIcon::MONITOR), p.primary);
    }
    if let Some(workspace) = tab.git_workspace() {
        let workspace_state = &workspace.read(cx).state;
        let icon = git_project_icon(&workspace_state.project);
        let color = git_project_color(&workspace_state.project).unwrap_or(if active {
            p.sidebar_accent_foreground
        } else {
            p.muted_foreground
        });
        return (icon, color);
    }
    if let Some(profile) = tab.container_profile() {
        let icon = session_icon(profile.icon).unwrap_or_else(|| {
            if profile.engine.kind == ContainerEngineKind::Docker {
                CustomIcon::DOCKER.into()
            } else {
                AppIcon::Domain(DomainIcon::CONTAINER)
            }
        });
        let color = session_color(profile.color).unwrap_or(if active {
            p.sidebar_accent_foreground
        } else {
            p.muted_foreground
        });
        return (icon, color);
    }
    let appearance = state.session_appearance(tab);
    let Some(session) = tab.session() else {
        return (
            appearance.icon.unwrap_or(AppIcon::Ui(UiIcon::ERROR)),
            appearance.color.unwrap_or(if active {
                p.sidebar_accent_foreground
            } else {
                p.muted_foreground
            }),
        );
    };
    let icon = appearance
        .icon
        .unwrap_or_else(|| crate::connection::icon(&session.connection));
    let color = appearance.color.unwrap_or(if active {
        p.sidebar_accent_foreground
    } else {
        p.muted_foreground
    });
    (icon, color)
}

/// 用户为会话、Git 项目或监控目标配置的环境色；未配置时不画强调条。
pub(crate) fn tab_accent(state: &AppState, tab: &WorkspaceTab, cx: &App) -> Option<Hsla> {
    if let Some(workspace) = tab.git_workspace() {
        return git_project_color(&workspace.read(cx).state.project);
    }
    state.session_appearance(tab).color
}

/// Git 标签图标与左侧仓库行保持一致，统一使用 Git 领域图标。
fn git_project_icon(_project: &GitProject) -> AppIcon {
    UiIcon::VCS.into()
}

/// Git 项目配置的环境色；未配置时不画强调条。
fn git_project_color(project: &GitProject) -> Option<Hsla> {
    session_color(project.color)
}

/// 标签状态点颜色：ZMODEM 传输中优先用强调色，其余按展示状态。
pub(crate) fn tab_status_dot(tab: &WorkspaceTab, p: &Palette) -> Hsla {
    if tab.zmodem().is_some() {
        return p.primary;
    }
    pane_status_indicator(&tab.status, p).0
}

/// 标签副标题：按标签类型生成稳定语义，而不是把所有非会话标签都显示成本地终端。
pub(crate) fn tab_subtitle(tab: &WorkspaceTab) -> String {
    match &tab.content {
        TabContent::Session(_) => tab
            .session()
            .map(|session| crate::connection::display_compact(&session.connection))
            .unwrap_or_default(),
        TabContent::Player(_) => rust_i18n::t!("tab_subtitle.player").to_string(),
        TabContent::Monitor(_) => rust_i18n::t!("tab_subtitle.monitor").to_string(),
        TabContent::Containers(_) => rust_i18n::t!("tab_subtitle.containers").to_string(),
        TabContent::Git(_) => rust_i18n::t!("tab_subtitle.git").to_string(),
        TabContent::Rdp(_) => rust_i18n::t!("rdp.desktop").to_string(),
    }
}

/// 并排列表所需的静态图标；不读取 Git/容器实体。
pub(crate) fn compact_tab_icon(tab: &WorkspaceTab) -> AppIcon {
    match &tab.content {
        TabContent::Session(_) => tab
            .session()
            .map(|session| crate::connection::icon(&session.connection))
            .unwrap_or_else(|| AppIcon::Ui(UiIcon::ERROR)),
        TabContent::Player(_) | TabContent::Monitor(_) | TabContent::Rdp(_) => {
            AppIcon::Domain(DomainIcon::MONITOR)
        }
        TabContent::Containers(container) => {
            if container.profile.engine.kind == ContainerEngineKind::Docker {
                CustomIcon::DOCKER.into()
            } else {
                AppIcon::Domain(DomainIcon::CONTAINER)
            }
        }
        TabContent::Git(_) => AppIcon::Ui(UiIcon::VCS),
    }
}

#[cfg(test)]
mod tests {
    use super::{git_project_color, git_project_icon};
    use std::path::PathBuf;
    use transom_core::{GitProject, SessionColor, SessionIcon};
    use transom_ui_icons::AppIcon;
    use transom_ui_icons::UiIcon;

    #[test]
    fn git_project_default_icon_matches_repository_row() {
        let project = GitProject::new(PathBuf::from("/tmp/transom-git-icon"));
        assert!(matches!(
            git_project_icon(&project),
            AppIcon::Ui(UiIcon::VCS)
        ));
    }

    #[test]
    fn git_project_icon_stays_unified_while_color_is_configurable() {
        let mut project = GitProject::new(PathBuf::from("/tmp/transom-git-icon"));
        project.icon = SessionIcon::Database;
        project.color = SessionColor::Blue;
        assert!(matches!(
            git_project_icon(&project),
            AppIcon::Ui(UiIcon::VCS)
        ));
        assert_eq!(
            git_project_color(&project),
            super::session_color(SessionColor::Blue)
        );
    }
}
