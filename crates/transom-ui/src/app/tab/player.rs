//! 录像回放标签。

use super::{TabContent, WorkspaceTab};
use crate::cast_player::CastPlayerView;
use gpui_kit::Entity;
use transom_core::TabId;

/// 录像回放标签只持有自己的播放器实体，不再伪造本地会话。
///
/// 设计原理（Why）：播放器标题、字体和播放状态都由 [`CastPlayerView`] 自己
/// 持有；标签仅保存可共享的展示元数据和实体句柄，因此关闭标签即可由 RAII
/// 释放播放器，不需要制造一个与会话无关的 `Session`。
pub(crate) struct PlayerTab {
    pub(crate) player: Entity<CastPlayerView>,
}

impl WorkspaceTab {
    /// 构造录像回放标签。
    pub(crate) fn new_player(id: TabId, title: String, player: Entity<CastPlayerView>) -> Self {
        Self {
            id,
            title,
            status: super::TabStatus::Connected,
            content: TabContent::Player(PlayerTab { player }),
        }
    }

    /// 获取录像播放器实体句柄。
    pub(crate) fn player(&self) -> Option<Entity<CastPlayerView>> {
        match &self.content {
            TabContent::Player(player) => Some(player.player.clone()),
            _ => None,
        }
    }
}
