//! 备份弹窗的外壳职责：打开入口、实体事件消费与目录合并收尾。
//!
//! 弹窗状态、文件选择与后台任务都在 [`crate::backup::BackupPromptView`]；
//! 外壳只做两件跨域的事：把当前目录快照注入实体，以及在导入成功后合并目录、
//! 持久化、刷新标签会话并写状态栏。事件自带全部数据，这里禁止回读实体。

use crate::app::AppShell;
use crate::backup::{BackupPromptEvent, secret_restore_note};
use crate::palette::localized_error;
use gpui_kit::*;
use std::path::Path;
use transom_core::{
    AppError, BackupExportReport, MergePreference, SecretRestoreReport, SessionCatalog,
    merge_catalogs,
};

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
    /// 事件已携带路径、统计与目录快照；这里不得再读备份实体，否则会在
    /// 实体 `emit` 期间形成双重租借。错误提示已由实体写入弹窗，外壳只记日志。
    pub(crate) fn on_backup_event(&mut self, event: &BackupPromptEvent, cx: &mut Context<Self>) {
        match event {
            BackupPromptEvent::Exported { path, report } => {
                self.state.status = Some(exported_status(path, report));
            }
            BackupPromptEvent::Imported { catalog, secrets } => {
                self.apply_imported_catalog(catalog, secrets.clone());
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

    /// 导入成功后的目录合并与提示；保存失败时错误已由 `persist_catalog` 入队。
    fn apply_imported_catalog(
        &mut self,
        catalog: &SessionCatalog,
        secrets: Result<SecretRestoreReport, AppError>,
    ) {
        let (merged, report) =
            merge_catalogs(None, &self.state.catalog, catalog, MergePreference::Remote);
        self.state.catalog = merged;
        let saved = self.state.persist_catalog();
        self.state.refresh_tab_sessions();
        let mut message = rust_i18n::t!(
            "backup.imported",
            added = report.sessions_added,
            updated = report.sessions_updated,
            folders = report.folders_added
        )
        .to_string();
        if report.conflicts > 0 {
            message.push(' ');
            message
                .push_str(rust_i18n::t!("sync.merge_conflicts", count = report.conflicts).as_ref());
        }
        message.push_str(&secret_restore_note(secrets));
        if saved {
            self.state.status = Some(message);
        }
    }
}

/// 导出成功的状态栏文案。
fn exported_status(path: &Path, report: &BackupExportReport) -> String {
    let path = path.display().to_string();
    let mut message = rust_i18n::t!("backup.exported", path = path).to_string();
    if report.secrets > 0 {
        message.push(' ');
        message.push_str(rust_i18n::t!("backup.exported_secrets", count = report.secrets).as_ref());
    }
    message
}
