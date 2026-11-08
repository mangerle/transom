//! 备份弹窗的外壳入口与事件消费。
//!
//! 弹窗状态、文件选择与后台任务都在 [`crate::backup::BackupPromptView`]；
//! 外壳这里只传递打开时机与当前目录快照，并接收实体上报的最终结果。

use crate::app::AppShell;
use crate::backup::BackupPromptEvent;
use crate::palette::localized_error;
use gpui_kit::*;

impl AppShell {
    /// 打开导出弹窗：注入当前目录快照后交给实体。
    pub(crate) fn open_backup_export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let catalog = self.state.catalog.clone();
        let view = self.state.modals.tools.backup_prompt.clone();
        view.update(cx, |view, cx| {
            view.set_catalog(catalog);
            view.open_export(window, cx);
        });
        cx.notify();
    }

    /// 先让用户选文件，再打开导入弹窗。
    pub(crate) fn open_backup_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = rfd::FileDialog::new()
            .set_title(rust_i18n::t!("backup.import_title").to_string())
            .add_filter("Transom Backup", &["json", "yaml", "yml"])
            .pick_file()
        else {
            return;
        };
        let catalog = self.state.catalog.clone();
        let view = self.state.modals.tools.backup_prompt.clone();
        view.update(cx, |view, cx| {
            view.set_catalog(catalog);
            view.open_import(path, window, cx);
        });
        cx.notify();
    }

    /// 备份实体上报的最终结果。
    ///
    /// 事件自带全部数据，处理时不得回读实体。目录合并与状态栏提示在后续
    /// 提交补齐；当前先记录结果，保证实体与外壳的事件通道先跑通。
    pub(crate) fn on_backup_event(&mut self, event: &BackupPromptEvent, cx: &mut Context<Self>) {
        match event {
            BackupPromptEvent::Exported { path, report } => {
                log::debug!(
                    "备份导出完成，路径: {}，会话数: {}，文件夹数: {}，口令数: {}",
                    path.display(),
                    report.sessions,
                    report.folders,
                    report.secrets
                );
            }
            BackupPromptEvent::Imported { catalog, secrets } => {
                let secrets = match secrets {
                    Ok(report) => format!("成功 {} 条", report.restored),
                    Err(error) => localized_error(error),
                };
                log::debug!(
                    "备份导入完成，会话数: {}，口令写回: {}",
                    catalog.sessions.len(),
                    secrets
                );
            }
            BackupPromptEvent::Error { error } => {
                log::warn!(
                    "备份操作失败，错误码: {:?}，原因: {}",
                    error.code,
                    localized_error(error)
                );
            }
        }
        cx.notify();
    }
}
