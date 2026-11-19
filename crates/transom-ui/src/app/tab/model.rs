//! 工作区标签的公共类型与构造器。

use super::{
    ContainerTab, GitTab, MonitorTab, PlayerTab, SessionPaneState, SessionTab, TabGroupOwner,
};
use transom_core::{AppError, Session, SessionId, TabId};

/// 标签栏的展示状态。
///
/// 该状态只服务标签栏、纵向标签与状态栏，不用于判断标签是否拥有终端会话。
#[derive(Debug, Clone)]
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

/// 标签内容类型；只用于不持有实体即可验证的纯策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TabContentKind {
    Session,
    Player,
    Monitor,
    Container,
    Git,
}

impl TabContent {
    /// 返回内容类型。
    pub(crate) fn kind(&self) -> TabContentKind {
        match self {
            Self::Session(_) => TabContentKind::Session,
            Self::Player(_) => TabContentKind::Player,
            Self::Monitor(_) => TabContentKind::Monitor,
            Self::Containers(_) => TabContentKind::Container,
            Self::Git(_) => TabContentKind::Git,
        }
    }
}

impl TabContentKind {
    /// 只有真实会话标签支持分屏和终端专属副作用。
    pub(crate) const fn is_session(self) -> bool {
        matches!(self, Self::Session)
    }

    /// 是否支持分屏。
    pub(crate) const fn supports_split(self) -> bool {
        self.is_session()
    }
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
        TabCloseAction::for_kind(self.content.kind(), self.id)
    }

    /// 只有真实会话标签可以分屏。
    pub(crate) fn can_split(&self) -> bool {
        self.content.kind().supports_split()
    }
}

impl TabCloseAction {
    pub(crate) fn for_kind(kind: TabContentKind, tab_id: TabId) -> Self {
        match kind {
            TabContentKind::Session => Self::CloseTerminal { tab_id },
            TabContentKind::Container => Self::DisconnectContainer { tab_id },
            TabContentKind::Player | TabContentKind::Monitor | TabContentKind::Git => {
                Self::DropOnly
            }
        }
    }
}
