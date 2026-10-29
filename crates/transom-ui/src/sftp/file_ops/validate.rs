//! SFTP 文件输入弹窗名称安全性校验与防路径穿越规则。

use crate::sftp::SftpInputTarget;
use std::path::{Component, Path};
use transom_core::is_safe_file_name;

/// 校验输入弹窗中的名称是否可以安全地参与路径拼接，非法时返回本地化提示文本。
///
/// 设计原理（Why）：
/// - 实现初衷：新建/重命名/软链接的名称会与当前目录直接拼接成完整路径，
///   若允许 `/` 或 `..` 输入，操作会逃逸出当前目录（路径穿越），
///   与文件管理器中用户可见的范围不一致，软链接场景影响面尤大；
/// - 核心优势：在唯一提交口（`commit_sftp_input_modal`）统一拦截，
///   新增目标类型不会遗漏校验，也无需在各调用点重复判断；
/// - 代价与局限：本地「新建文件夹」保留 `a/b` 相对多级创建能力，
///   但同样拒绝绝对路径与 `..` 逃逸片段；其余目标严格要求单层名称，
///   重命名等场景不再支持输入含 `/` 的路径。
pub(super) fn name_validation_error(target: &SftpInputTarget, name: &str) -> Option<String> {
    // 本地新建目录允许 "a/b" 这样的多级相对路径，但仍必须拒绝绝对路径、
    // Windows 盘符与任何 .. 片段；反斜杠是否是分隔符由宿主平台决定，
    // 交给 `Path::components` 判断，避免在 Unix 上误杀合法文件名。
    if matches!(target, SftpInputTarget::LocalMkdir) {
        let path = Path::new(name);
        if path.is_absolute()
            || path.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Some(rust_i18n::t!("sftp.file_ops.name_absolute_or_parent").to_string());
        }
        return None;
    }
    if !is_safe_file_name(name) {
        return Some(rust_i18n::t!("sftp.file_ops.name_single_file").to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::name_validation_error;
    use crate::sftp::SftpInputTarget;

    #[test]
    fn test_name_validation_rejects_traversal() {
        let remote_symlink = SftpInputTarget::RemoteSymlink {
            target: "/srv/app".to_string(),
        };
        assert!(name_validation_error(&remote_symlink, "../evil").is_some());
        assert!(name_validation_error(&remote_symlink, "sub/link").is_some());
        assert!(name_validation_error(&remote_symlink, "..").is_some());
        assert!(name_validation_error(&remote_symlink, "app.link").is_none());

        assert!(name_validation_error(&SftpInputTarget::RemoteMkdir, "a/b").is_some());
        assert!(name_validation_error(&SftpInputTarget::RemoteMkdir, "目录").is_none());

        let local_rename = SftpInputTarget::LocalRename {
            old_path: std::path::PathBuf::from("/tmp/old"),
        };
        assert!(name_validation_error(&local_rename, "..").is_some());
        assert!(name_validation_error(&local_rename, "../escape").is_some());
        #[cfg(windows)]
        assert!(name_validation_error(&local_rename, "a\\b").is_some());
        assert!(name_validation_error(&local_rename, "new-name").is_none());

        assert!(name_validation_error(&SftpInputTarget::RemoteMkdir, "/tmp/escape").is_some());
        #[cfg(windows)]
        assert!(name_validation_error(&SftpInputTarget::RemoteMkdir, "a\\b").is_some());
    }

    #[test]
    fn test_name_validation_local_mkdir_keeps_multi_level() {
        assert!(name_validation_error(&SftpInputTarget::LocalMkdir, "a/b").is_none());
        assert!(name_validation_error(&SftpInputTarget::LocalMkdir, "a/../b").is_some());
        assert!(name_validation_error(&SftpInputTarget::LocalMkdir, "..").is_some());
        assert!(name_validation_error(&SftpInputTarget::LocalMkdir, "/tmp/escape").is_some());
        #[cfg(windows)]
        assert!(name_validation_error(&SftpInputTarget::LocalMkdir, "a\\..\\b").is_some());
    }
}
