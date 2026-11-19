//! 非会话工作区标签的专用内容类型。
//!
//! 设计原理（Why）：这些标签共享标签栏与分组能力，但生命周期和业务来源不同。
//! 播放器持有播放器实体，监控关联真实目标会话，容器持有引擎工作区，Git 持有仓库
//! 工作区。集中定义后，它们都不需要借用通用 `SessionTab` 的终端连接语义。

use crate::cast_player::CastPlayerView;
use crate::container::ContainerWorkspace;
use crate::git::GitWorkspace;
use gpui_kit::Entity;
use transom_core::{ContainerEngineProfile, Session, SessionId, TabId};

/// 录像回放标签只持有自己的播放器实体，不再伪造本地会话。
pub(crate) struct PlayerTab {
    pub(crate) player: Entity<CastPlayerView>,
}

/// 监控标签关联一个真实会话，但自身不参与连接、SFTP 或终端事件。
pub(crate) struct MonitorTab {
    pub(crate) target_tab_id: TabId,
    pub(crate) target_session_id: SessionId,
    pub(crate) target_session: Session,
}

/// 容器标签保存引擎配置与工作区实体；连接状态由工作区实体作为唯一真相源。
pub(crate) struct ContainerTab {
    pub(crate) profile: ContainerEngineProfile,
    pub(crate) workspace: Entity<ContainerWorkspace>,
}

/// Git 标签只持有仓库工作区实体，项目路径与分支状态由实体维护。
pub(crate) struct GitTab {
    pub(crate) workspace: Entity<GitWorkspace>,
}
