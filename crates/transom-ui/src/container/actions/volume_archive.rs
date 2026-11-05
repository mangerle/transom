//! 数据卷（Volume）归档备份与快照恢复交互动作（Backup / Restore）。
//!
//! 设计原理（Why）：
//! - 数据卷备份为 Tar 压缩包（Backup）：通过原生系统文件保存对话框指定目标路径，
//!   流式将数据卷全部内容读取为本地 Tar 归档，保留文件层级与权限；
//! - 数据卷从本地 Tar 恢复（Restore）：选取本地 Tar 归档文件，直接流式推送到目标数据卷中解压；
//!   若目标卷不存在则自动创建，实现冷备份快照还原。

use super::*;
use std::path::Path;

fn volume_name_from_path(path: &Path) -> String {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("restored_volume");
    let name = name
        .strip_suffix(".tar.gz")
        .or_else(|| name.strip_suffix(".tgz"))
        .or_else(|| name.strip_suffix(".tar"))
        .unwrap_or(name);
    if name.is_empty() {
        "restored_volume".to_string()
    } else {
        name.to_string()
    }
}

impl AppShell {
    /// 备份指定数据卷为本地 Tar 压缩包。
    pub(crate) fn backup_volume(&mut self, tab_id: TabId, volume: String, cx: &mut Context<Self>) {
        let default_name = format!("{}.tar", volume.replace(['/', ':'], "_"));
        let Some(local_path) = rfd::FileDialog::new()
            .set_file_name(&default_name)
            .add_filter("Tar Archive", &["tar"])
            .save_file()
        else {
            return;
        };

        let request_id = ContainerRequestId::new();
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.notice = Some(rust_i18n::t!("container.transfer.start").to_string());
        }
        self.send_tracked(
            tab_id,
            request_id,
            ContainerCommand::BackupVolume {
                tab_id,
                request_id,
                volume,
                local_path,
            },
        );
        cx.notify();
    }

    /// 从本地 Tar 压缩包还原到数据卷中（若目标卷未指定，默认采用文件名作为卷名）。
    pub(crate) fn restore_volume(
        &mut self,
        tab_id: TabId,
        target_volume: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(local_path) = rfd::FileDialog::new()
            .add_filter("Tar Archive", &["tar", "tar.gz", "tgz"])
            .pick_file()
        else {
            return;
        };

        let volume = target_volume.unwrap_or_else(|| volume_name_from_path(&local_path));
        let request_id = ContainerRequestId::new();
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.notice = Some(rust_i18n::t!("container.transfer.start").to_string());
        }
        self.send_tracked(
            tab_id,
            request_id,
            ContainerCommand::RestoreVolume {
                tab_id,
                request_id,
                volume,
                local_path,
            },
        );
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::volume_name_from_path;
    use std::path::Path;

    #[test]
    fn test_volume_name_from_path() {
        assert_eq!(
            volume_name_from_path(Path::new("/home/user/my_db_data.tar")),
            "my_db_data"
        );
        assert_eq!(
            volume_name_from_path(Path::new("/home/user/backup.tar.gz")),
            "backup"
        );
        assert_eq!(
            volume_name_from_path(Path::new("/home/user/redis.tgz")),
            "redis"
        );
    }
}
