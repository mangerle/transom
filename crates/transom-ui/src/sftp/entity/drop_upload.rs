//! 实体侧的拖放上传入口：松手到具体远端列。

use super::*;
use crate::sftp::SftpRemotePaneRef;

impl SftpPanelView {
    /// 外部文件松手到指定远端列：上传目标为该列当前目录与连接。
    pub(crate) fn drop_files_to_pane(
        &mut self,
        paths: &[std::path::PathBuf],
        target: SftpRemotePaneRef,
        cx: &mut Context<Self>,
    ) {
        if paths.is_empty() {
            return;
        }
        let Some(main_connection) = self.state.main_connection else {
            return;
        };
        let Some(connection_id) = self.state.remote_connection(target, Some(main_connection))
        else {
            return;
        };
        let Some(remote_dir) = self.state.remote_pane(target).map(|pane| pane.path.clone()) else {
            return;
        };
        if let Err(message) =
            self.enqueue_uploads_to(self.tab_id, paths, &remote_dir, connection_id, cx)
        {
            self.set_pane_error(target, message, cx);
            return;
        }
        log::info!(
            "拖拽上传已入队，窗格: {}，文件数: {}，目标目录: {remote_dir}",
            self.tab_id,
            paths.len()
        );
        self.show_drop_overlay(cx);
    }

    /// 拖入即显示浮层：展开明细并撤掉待收起定时器。
    pub(crate) fn show_drop_overlay(&mut self, cx: &mut Context<Self>) {
        // 面板已经展开时不弹浮层：队列区就在旁边。
        if self.state.open {
            return;
        }
        self.state.queue.drop_overlay.visible = true;
        self.state.queue.drop_overlay.expanded = true;
        self.state.queue.drop_overlay.settle_task = None;
        cx.notify();
    }
}
