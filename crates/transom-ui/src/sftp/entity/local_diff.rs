//! 实体侧的本地对比入口：校验选中项后把目标交给外壳打开对比面板。

use super::*;
use crate::diff_viewer::LocalDiffCheck;

impl SftpPanelView {
    /// 从本地栏打开：选中本地文件，与远端当前目录的同名文件比对。
    pub(crate) fn open_local_diff(&mut self, cx: &mut Context<Self>) {
        match crate::diff_viewer::local_diff_check(self.state.main_connection, &self.state) {
            LocalDiffCheck::Ready(target) => {
                cx.emit(crate::sftp::SftpPanelEvent::OpenLocalDiff {
                    tab_id: self.tab_id,
                    target,
                });
                cx.notify();
            }
            LocalDiffCheck::Rejected(reason) => {
                cx.emit(crate::sftp::SftpPanelEvent::SetStatus(reason));
                cx.notify();
            }
            LocalDiffCheck::NotApplicable => {}
        }
    }
}
