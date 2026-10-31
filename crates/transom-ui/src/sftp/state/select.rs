//! SFTP 面板的列选择、远端/本地选中集与进度收敛。

use super::*;
use crate::sftp::selection;

impl SftpPanelState {
    /// 按目标取远端列状态。
    pub(crate) fn remote_pane(&self, target: SftpRemotePaneRef) -> Option<&SftpRemotePaneState> {
        match target {
            SftpRemotePaneRef::Main => Some(&self.remote),
            SftpRemotePaneRef::Side(connection_id) => self
                .side_remotes
                .iter()
                .find(|side| side.connection_id == Some(connection_id)),
        }
    }

    /// 按目标取远端列状态的可变借用。
    pub(crate) fn remote_pane_mut(
        &mut self,
        target: SftpRemotePaneRef,
    ) -> Option<&mut SftpRemotePaneState> {
        match target {
            SftpRemotePaneRef::Main => Some(&mut self.remote),
            SftpRemotePaneRef::Side(connection_id) => self
                .side_remotes
                .iter_mut()
                .find(|side| side.connection_id == Some(connection_id)),
        }
    }

    /// 按目标解析实际连接：主远端取标签页当前连接，并列列取列自身的连接。
    pub(crate) fn remote_connection(
        &self,
        target: SftpRemotePaneRef,
        main_connection: Option<ConnectionId>,
    ) -> Option<ConnectionId> {
        match target {
            SftpRemotePaneRef::Main => main_connection,
            SftpRemotePaneRef::Side(connection_id) => Some(connection_id),
        }
    }

    /// 按连接反解远端列身份：主连接对应主列，其余连接对应已挂载的并列列。
    ///
    /// 动作与事件只携带连接，渲染与状态更新需要列身份；反解集中在这里，
    /// 避免每个入口再写一套"先比主连接、再找并列列"的分支。
    pub(crate) fn remote_pane_of_connection(
        &self,
        connection_id: ConnectionId,
        main_connection: Option<ConnectionId>,
    ) -> Option<SftpRemotePaneRef> {
        if main_connection == Some(connection_id) {
            Some(SftpRemotePaneRef::Main)
        } else {
            self.remote_pane(SftpRemotePaneRef::Side(connection_id))
                .map(|_| SftpRemotePaneRef::Side(connection_id))
        }
    }

    /// 选中远端条目并标记对应远端列为活动栏。
    pub(crate) fn select_remote_pane(&mut self, target: SftpRemotePaneRef, path: String) {
        if let Some(pane) = self.remote_pane_mut(target) {
            pane.select(path);
            self.active_pane = target.into();
        }
    }

    /// Ctrl/Cmd 点击切换远端条目的选中状态。
    pub(crate) fn toggle_remote_selection_pane(&mut self, target: SftpRemotePaneRef, path: String) {
        if let Some(pane) = self.remote_pane_mut(target) {
            pane.toggle(path);
            self.active_pane = target.into();
        }
    }

    /// Shift 连选：用可见列表中的一段路径替换远端集合。
    pub(crate) fn select_remote_range_pane(
        &mut self,
        target: SftpRemotePaneRef,
        paths: impl IntoIterator<Item = String>,
        primary: String,
    ) {
        if let Some(pane) = self.remote_pane_mut(target) {
            pane.select_range(paths, primary);
            self.active_pane = target.into();
        }
    }

    /// 返回目标远端列当前需要批量操作的路径。
    pub(crate) fn selected_remote_paths_pane(&self, target: SftpRemotePaneRef) -> Vec<String> {
        self.remote_pane(target)
            .map(SftpRemotePaneState::selected_paths)
            .unwrap_or_default()
    }

    pub(crate) fn clear_remote_selection_pane(&mut self, target: SftpRemotePaneRef) {
        if let Some(pane) = self.remote_pane_mut(target) {
            pane.clear_selection();
        }
    }

    /// 清空主远端列的选中集（终端目录变化时使用）。
    pub(crate) fn clear_remote_selection(&mut self) {
        self.clear_remote_selection_pane(SftpRemotePaneRef::Main);
    }

    /// 单选一个本地条目，同时清空多选集合并重置 Shift 锚点。
    pub(crate) fn select_local(&mut self, path: PathBuf) {
        selection::select(
            &mut self.local.selected,
            &mut self.local.selection,
            &mut self.local.anchor,
            path,
        );
        self.active_pane = SftpActivePane::Local;
    }

    /// 设置本地主项但保留已有多选集合，供右键菜单定位单项操作。
    pub(crate) fn set_local_primary(&mut self, path: PathBuf) {
        selection::set_primary(&mut self.local.selected, &mut self.local.selection, path);
        self.active_pane = SftpActivePane::Local;
    }

    /// Ctrl/Cmd 点击切换单个条目的选中状态。
    pub(crate) fn toggle_local_selection(&mut self, path: PathBuf) {
        selection::toggle(
            &mut self.local.selected,
            &mut self.local.selection,
            &mut self.local.anchor,
            path,
        );
        self.active_pane = SftpActivePane::Local;
    }

    /// Shift 连选：用可见列表中的一段路径替换当前集合。
    pub(crate) fn select_local_range(
        &mut self,
        paths: impl IntoIterator<Item = PathBuf>,
        primary: PathBuf,
    ) {
        selection::select_range(
            &mut self.local.selected,
            &mut self.local.selection,
            paths,
            primary,
        );
        self.active_pane = SftpActivePane::Local;
    }

    /// 返回当前需要批量操作的本地路径；无多选时退化为单选主项。
    pub(crate) fn selected_local_paths(&self) -> Vec<PathBuf> {
        selection::paths(&self.local.selected, &self.local.selection)
    }

    pub(crate) fn clear_local_selection(&mut self) {
        selection::clear(
            &mut self.local.selected,
            &mut self.local.selection,
            &mut self.local.anchor,
        );
    }

    /// 刷新目录后剔除已不存在的选中项，并修正主项与锚点。
    pub(crate) fn sync_local_selection(&mut self, valid: &BTreeSet<PathBuf>) {
        selection::sync(
            &mut self.local.selected,
            &mut self.local.selection,
            &mut self.local.anchor,
            valid,
        );
    }

    /// 汇总所有活动任务的字节进度；无活动任务时清空进度。
    pub(crate) fn recompute_progress(&mut self) {
        if self.queue.active_transfers.is_empty() {
            self.queue.progress = None;
            return;
        }
        let mut transferred = 0u64;
        let mut total = 0u64;
        let mut total_known = true;
        for task in &self.queue.tasks {
            if !self.queue.active_transfers.contains(&task.id) {
                continue;
            }
            transferred = transferred.saturating_add(task.transferred);
            match task.size {
                Some(size) => total = total.saturating_add(size),
                None => total_known = false,
            }
        }
        self.queue.progress = Some((transferred, total_known.then_some(total)));
    }

    /// 是否所有文件列均已处于折叠收起状态。
    pub(crate) fn is_all_collapsed(&self) -> bool {
        if !self.local.collapsed {
            return false;
        }
        if !self.local.only && !self.remote.collapsed {
            return false;
        }
        if self.side_remotes.iter().any(|s| !s.collapsed) {
            return false;
        }
        true
    }

    /// 统计当前折叠为竖条的列数（用于全收起时动态计算紧凑侧边栏宽度）。
    pub(crate) fn collapsed_columns_count(&self) -> usize {
        let mut count = 0;
        if self.local.collapsed {
            count += 1;
        }
        if !self.local.only && self.remote.collapsed {
            count += 1;
        }
        count += self.side_remotes.iter().filter(|s| s.collapsed).count();
        count
    }
}
