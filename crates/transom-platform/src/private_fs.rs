//! 用户私有文件与目录的创建、权限收紧。
//!
//! 设计原理（Why）：审计、录制、运行日志与配置文件都可能包含主机名、路径、
//! 命令行甚至终端原文。它们散落在三个 crate 里各自 `create_dir_all` +
//! `OpenOptions`，权限完全依赖进程 umask；多用户主机上会退化成 0644/0755。
//! 把“私有目录 0700、私有文件 0600”收敛到这里，调用方只表达“这是私有资源”，
//! 不再各自复述一遍 Unix 权限细节。
//!
//! 平台口径：
//! - Unix：目录 0700、文件 0600；
//! - Windows：为新建目录和文件应用受保护 DACL，仅允许当前用户、SYSTEM 与
//!   内置管理员访问；
//! - 其它平台：没有可移植的权限模型，保持原行为。

use std::fs::File;
#[cfg(not(windows))]
use std::fs::OpenOptions;
use std::io;
use std::path::Path;

#[cfg(windows)]
pub(crate) mod windows;

/// 创建目录（含父目录），并在 Unix 上设为 0700、Windows 上应用私有 DACL。
///
/// 已存在的目录不会被改权限：调用方可能传入 `/tmp` 或 `~/.config` 这类共享
/// 父目录，静默收紧会影响其它程序。文件本身仍由 [`restrict_file_permissions`]
/// 收紧到 0600，因此旧版本留下的 0755 配置目录也不会泄露文件内容。
///
/// # Errors
/// 创建失败或目标不是目录时返回底层 IO 错误。
pub fn ensure_private_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;

        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
    }
    #[cfg(not(unix))]
    #[cfg(not(windows))]
    {
        std::fs::create_dir_all(dir)?;
    }
    #[cfg(windows)]
    {
        windows::ensure_private_dir(dir)?;
    }
    let metadata = std::fs::metadata(dir)?;
    if !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            format!("私有路径不是目录: {}", dir.display()),
        ));
    }
    Ok(())
}

/// 把已打开文件的权限收紧到私有主体（Unix 0600 / Windows 私有 DACL）。
///
/// 用句柄而不是路径设置权限，避免“检查路径后文件被替换”的竞态。
///
/// # Errors
/// Unix 或 Windows 上权限设置失败时返回底层 IO 错误；其它平台为空操作。
pub fn restrict_file_permissions(file: &File) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    #[cfg(not(windows))]
    {
        let _ = file;
    }
    #[cfg(windows)]
    {
        windows::restrict_file_permissions(file)?;
    }
    Ok(())
}

/// 以追加方式打开私有文件；不存在时创建，并保证权限为 0600。
///
/// Unix 上在 open 时直接带 0600 模式；Windows 上创建文件时携带私有安全
/// 描述符，二者都从第一刻起限制访问。随后仍对句柄收紧一次，兜底旧版本
/// 留下的宽权限文件。
///
/// # Errors
/// 打开或权限设置失败时返回底层 IO 错误。
pub fn open_private_append(path: &Path) -> io::Result<File> {
    #[cfg(windows)]
    {
        windows::open_private_append(path)
    }
    #[cfg(not(windows))]
    {
        let mut options = OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(path)?;
        restrict_file_permissions(&file)?;
        Ok(file)
    }
}

/// 以截断方式创建/打开私有文件，并保证权限为 0600。
///
/// 与 [`open_private_append`] 相同：创建时即带 0600，创建后再兜底收紧。
///
/// # Errors
/// 打开或权限设置失败时返回底层 IO 错误。
pub fn open_private_truncate(path: &Path) -> io::Result<File> {
    #[cfg(windows)]
    {
        windows::open_private_truncate(path)
    }
    #[cfg(not(windows))]
    {
        let mut options = OpenOptions::new();
        options.create(true).write(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(path)?;
        restrict_file_permissions(&file)?;
        Ok(file)
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::*;

    #[cfg(unix)]
    #[test]
    fn private_dir_and_files_are_owner_only() {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().expect("临时目录应可创建");
        let dir = root.path().join("private/logs");
        ensure_private_dir(&dir).expect("私有目录应可创建");
        let mode = std::fs::metadata(&dir)
            .expect("目录元数据应可读")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700);

        let append = dir.join("append.log");
        let mut file = open_private_append(&append).expect("追加文件应可打开");
        file.write_all(b"line\n").expect("写入应成功");
        let mode = std::fs::metadata(&append)
            .expect("文件元数据应可读")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);

        let truncate = dir.join("truncate.log");
        let _file = open_private_truncate(&truncate).expect("截断文件应可打开");
        let mode = std::fs::metadata(&truncate)
            .expect("文件元数据应可读")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[cfg(windows)]
    #[test]
    fn private_dir_and_files_have_protected_windows_dacl() {
        use super::*;
        use std::io::Write;

        let root = tempfile::tempdir().expect("临时目录应可创建");
        let dir = root.path().join("private/logs");
        ensure_private_dir(&dir).expect("私有目录应可创建");
        super::windows::assert_private_acl(&dir, true);

        let append = dir.join("append.log");
        let mut file = open_private_append(&append).expect("追加文件应可打开");
        file.write_all(b"line\n").expect("写入应成功");
        super::windows::assert_private_acl(&append, false);

        let truncate = dir.join("truncate.log");
        let _file = open_private_truncate(&truncate).expect("截断文件应可打开");
        super::windows::assert_private_acl(&truncate, false);
    }
}
