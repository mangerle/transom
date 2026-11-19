//! 工作区标签的公共类型与构造器。

use super::{
    ContainerTab, GitTab, MonitorTab, PlayerTab, SessionPaneState, SessionTab, TabGroupOwner,
};
use crate::cast_player::CastPlayerView;
use crate::container::ContainerWorkspace;
use crate::git::GitWorkspace;
use gpui_kit::Entity;
use transom_core::{AppError, ContainerEngineProfile, Session, SessionId, TabId};

/// 标签栏的展示状态。
///
/// 该状态只服务标签栏、纵向标签与状态栏，不用于判断标签是否拥有终端会话。
#[derive(Debug)]
pub(crate) enum TabStatus {
    Connecting,
    Connected,
    Failed(AppError),
    Disconnected,
}

/// 工作区标签的公共外壳。
///
/// 设计原理（Why）：所有标签共享稳定 ID、标题、展示状态和内容枚举；会话专属
/// 字段放到 [`SessionTab`] 后，Git、容器和后续 API 标签无需伪造连接即可复用
/// 标签栏、拖拽、分组和并排能力。
pub(crate) struct WorkspaceTab {
    pub(crate) id: TabId,
    pub(crate) title: String,
    pub(crate) status: TabStatus,
    pub(crate) content: TabContent,
}

/// 标签内容的具体类型。
pub(crate) enum TabContent {
    Session(Box<SessionTab>),
    Player(PlayerTab),
    Monitor(Box<MonitorTab>),
    Containers(Box<ContainerTab>),
    Git(GitTab),
}

/// 标签关闭时由外壳执行的控制器动作。
pub(crate) enum TabCloseAction {
    CloseTerminal { tab_id: TabId },
    DisconnectContainer { tab_id: TabId },
    DropOnly,
}

/// 会话专属右键菜单数据；非会话标签不提供。
#[derive(Clone)]
pub(crate) struct SessionMenuData {
    pub(crate) session_id: SessionId,
    pub(crate) is_local: bool,
    pub(crate) host: Option<String>,
}

impl WorkspaceTab {
    /// 构造处于连接等待中的终端标签。
    pub(crate) fn new_session(id: TabId, session: Session) -> Self {
        Self {
            id,
            title: String::new(),
            status: TabStatus::Connecting,
            content: TabContent::Session(Box::new(SessionTab {
                session_id: session.id,
                session,
                connection_id: None,
                cwd: None,
                pane: SessionPaneState::default(),
            })),
        }
    }

    /// 构造录像回放标签。
    pub(crate) fn new_player(id: TabId, title: String, player: Entity<CastPlayerView>) -> Self {
        Self {
            id,
            title,
            status: TabStatus::Connected,
            content: TabContent::Player(PlayerTab { player }),
        }
    }

    /// 构造独立监控标签。
    pub(crate) fn new_monitor(
        id: TabId,
        title: String,
        target_tab_id: TabId,
        target_session: Session,
    ) -> Self {
        Self {
            id,
            title,
            status: TabStatus::Connected,
            content: TabContent::Monitor(Box::new(MonitorTab {
                target_tab_id,
                target_session_id: target_session.id,
                target_session,
            })),
        }
    }

    /// 构造容器工作区标签。
    pub(crate) fn new_container(
        id: TabId,
        title: String,
        profile: ContainerEngineProfile,
        workspace: Entity<ContainerWorkspace>,
    ) -> Self {
        Self {
            id,
            title,
            status: TabStatus::Connecting,
            content: TabContent::Containers(Box::new(ContainerTab { profile, workspace })),
        }
    }

    /// 构造 Git 工作区标签。
    pub(crate) fn new_git(id: TabId, title: String, workspace: Entity<GitWorkspace>) -> Self {
        Self {
            id,
            title,
            status: TabStatus::Connected,
            content: TabContent::Git(GitTab { workspace }),
        }
    }

    /// 返回标签组归属键。
    pub(crate) fn group_owner(&self) -> TabGroupOwner {
        match &self.content {
            TabContent::Session(session) => TabGroupOwner::Session(session.session_id),
            TabContent::Monitor(monitor) => TabGroupOwner::Session(monitor.target_session_id),
            TabContent::Player(_) | TabContent::Containers(_) | TabContent::Git(_) => {
                TabGroupOwner::Workspace(self.id)
            }
        }
    }

    /// 返回关闭标签时需要执行的控制器动作。
    pub(crate) fn close_action(&self) -> TabCloseAction {
        match &self.content {
            TabContent::Session(_) => TabCloseAction::CloseTerminal { tab_id: self.id },
            TabContent::Containers(_) => TabCloseAction::DisconnectContainer { tab_id: self.id },
            TabContent::Player(_) | TabContent::Monitor(_) | TabContent::Git(_) => {
                TabCloseAction::DropOnly
            }
        }
    }

    /// 只有真实会话标签可以分屏。
    pub(crate) fn can_split(&self) -> bool {
        self.is_session()
    }
}
