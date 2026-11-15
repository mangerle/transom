//! 标签页集合与分割树的纯数据结构操作：建组、会话快照回填与焦点回退查询。
//!
//! 设计原理（Why）：本目录只维护 `WorkspaceState` 与 `AppState` 上的纯标签集合
//! 读写与查询，不直接持有应用外壳；跨面板/控制面的打开、激活与关闭协调位于
//! `app/tab_coordination.rs`。

mod close_ops;

use crate::app::{AppState, TabGroup};
use crate::workspace::WorkspaceState;
use transom_core::{Connection, GroupId, Session, SessionId, TabId};

impl WorkspaceState {
    /// 标签所属分组的索引；标签不属于任何分组时返回 `None`。
    ///
    /// 纯工作区查询，不依赖 GPUI 上下文，也不读取会话目录。
    pub(crate) fn group_index_for_tab(&self, tab_id: TabId) -> Option<usize> {
        self.group_index_of(tab_id)
    }

    /// 新建标签组并返回组 ID；调用方随后把首个窗格加入 `tabs`。
    pub(crate) fn create_group(&mut self, tab_id: TabId, session_id: SessionId) -> GroupId {
        let serial = self
            .groups
            .iter()
            .filter(|group| group.session_id == session_id)
            .map(|group| group.serial)
            .max()
            .map_or(0, |max| max + 1);
        let group = TabGroup::new(tab_id, session_id, serial);
        let id = group.id;
        self.groups.push(group);
        id
    }

    /// 新建本地终端标签组，序号按"本地终端的基础标题"分配。
    pub(crate) fn create_local_group(
        &mut self,
        tab_id: TabId,
        session_id: SessionId,
        base_title: &str,
    ) -> GroupId {
        let serial = self
            .groups
            .iter()
            .filter_map(|group| {
                let tab = self.tabs.iter().find(|tab| tab.id == group.active_pane)?;
                if !matches!(tab.session.connection, Connection::Local(_)) {
                    return None;
                }
                (tab.session.name == base_title).then_some(group.serial)
            })
            .max()
            .map_or(0, |max| max + 1);
        let id = self.create_group(tab_id, session_id);
        if let Some(group) = self.groups.iter_mut().find(|group| group.id == id) {
            group.serial = serial;
        }
        id
    }
}

impl AppState {
    pub(crate) fn refresh_tab_sessions(&mut self) {
        for index in 0..self.workspace.tabs.len() {
            let session_id = self.workspace.tabs[index].session_id;
            // 先结束对 catalog 的只读借用，再写回标签：两个字段同属
            // `self.state`，同时读写会被当成对整体的借用冲突。
            let resolved = self
                .catalog
                .sessions
                .iter()
                .find(|session| session.id == session_id)
                .map(|session| self.catalog.resolved_session(session));
            if let Some(session) = resolved {
                self.workspace.tabs[index].session = session;
            }
        }
    }

    /// 取窗格应当使用的会话配置：优先目录里的最新记录，退回标签上的快照。
    pub(crate) fn session_for_tab(&self, tab_id: TabId) -> Option<Session> {
        let index = self.workspace.tab_index(tab_id)?;
        let session_id = self.workspace.tabs[index].session_id;
        Some(
            self.catalog
                .sessions
                .iter()
                .find(|session| session.id == session_id)
                .cloned()
                .unwrap_or_else(|| self.workspace.tabs[index].session.clone()),
        )
    }

    /// 计算当前标签被移除或替换后应激活的窗格。
    ///
    /// 优先级：显式替换目标 → 并排网格当前焦点组 → 移除前的分组焦点 →
    /// 第一个分组。关闭标签、删除会话与复制标签共用同一顺序，避免几处
    /// 各写一条 `or_else` 链后逐渐漂移。
    pub(crate) fn fallback_active_pane(
        &self,
        replacement: Option<TabId>,
        previous_tiled_focus: Option<GroupId>,
    ) -> Option<TabId> {
        replacement
            .or_else(|| {
                self.active_tiling()
                    .and_then(|tiled| tiled.focused_group())
                    .and_then(|group| self.workspace.group(group).map(|item| item.active_pane))
            })
            .or_else(|| {
                previous_tiled_focus
                    .and_then(|group| self.workspace.group(group).map(|item| item.active_pane))
            })
            .or_else(|| self.workspace.groups.first().map(|group| group.active_pane))
    }
}
