//! 用户私有文件与目录的创建、权限收紧。
//!
//! 设计原理（Why）：审计、录制、运行日志与配置文件都可能包含主机名、路径、
//! 命令行甚至终端原文。它们散落在三个 crate 里各自 `create_dir_all` +
//! `OpenOptions`，权限完全依赖进程 umask；多用户主机上会退化成 0644/0755。
//! 把“私有目录 0700、私有文件 0600”收敛到这里，调用方只表达“这是私有资源”，
//! 不再各自复述一遍 Unix 权限细节。
//!
//! 代价与局限：非 Unix 平台没有等价的 mode 语义，只能依赖用户目录自身的 ACL；
//! 因此这里的收紧是 Unix 专属，非 Unix 保持原有行为。

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

/// 创建目录（含父目录），并在 Unix 上把新建目录的权限设为 0700。
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
    {
        std::fs::create_dir_all(dir)?;
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

/// 把已打开文件的权限收紧到仅属主可读写（Unix 0600）。
///
/// 用句柄而不是路径设置权限，避免“检查路径后文件被替换”的竞态。
///
/// # Errors
/// Unix 上权限设置失败时返回底层 IO 错误；其它平台为空操作。
pub fn restrict_file_permissions(file: &File) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    {
        let _ = file;
    }
    Ok(())
}

/// 以追加方式打开私有文件；不存在时创建，并把权限收紧到 0600。
///
/// # Errors
/// 打开或权限设置失败时返回底层 IO 错误。
pub fn open_private_append(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new().create(true).append(true).open(path)?;
    restrict_file_permissions(&file)?;
    Ok(file)
}

/// 以截断方式创建/打开私有文件，并把权限收紧到 0600。
///
/// # Errors
/// 打开或权限设置失败时返回底层 IO 错误。
pub fn open_private_truncate(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)?;
    restrict_file_permissions(&file)?;
    Ok(file)
}

#[cfg(test)]
mod tests {
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
}
