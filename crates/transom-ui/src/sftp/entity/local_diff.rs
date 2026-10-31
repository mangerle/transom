//! 实体侧的本地对比入口：校验选中项后把目标交给外壳打开对比面板。

use super::*;
use crate::diff_viewer::LocalDiffCheck;

impl SftpPanelView {
    /// 从本地栏打开：选中本地文件，与远端当前目录的同名文件比对。
    pub(crate) fn open_local_diff(&mut self, cx: &mut Context<Self>) {
        let app = self.app.clone();
        let Some(check) = read_shell_parts(&app, cx, |shell| {
            shell
                .state
                .local_diff_check_with_panel(self.tab_id, &self.state)
        }) else {
            return;
        };
        match check {
            LocalDiffCheck::Ready(target) => {
                with_shell_parts(&app, cx, |shell, cx| {
                    shell.open_local_diff_with_target(self.tab_id, target, cx);
                });
            }
            LocalDiffCheck::Rejected(reason) => {
                with_shell_parts(&app, cx, |shell, cx| {
                    shell.state.status = Some(reason);
                    cx.notify();
                });
            }
            LocalDiffCheck::NotApplicable => {}
        }
    }
}
