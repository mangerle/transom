//! 跨平台文件系统操作。
//!
//! 设计原理（Why）：符号链接在 Unix 只有一个入口，Windows 却要求区分文件与
//! 目录两个系统调用。把这个差异收敛到平台层，UI 只表达“创建符号链接”，
//! 不在业务模块散落 `cfg(windows)`。

use std::io;
use std::path::Path;

/// 创建指向 `target` 的符号链接 `link`。
///
/// Windows 根据目标当前类型选择 `symlink_file` 或 `symlink_dir`。若目标尚不
/// 存在，则按文件符号链接创建；这是 Windows API 无法省略的类型选择。
/// 创建目录符号链接可能需要开发者模式或 SeCreateSymbolicLinkPrivilege。
///
/// # Errors
/// 目标类型元数据读取失败时仍按文件符号链接尝试；系统拒绝创建、父目录缺失
/// 或链接已存在时返回底层 IO 错误。
pub fn create_symlink(target: &Path, link: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(windows)]
    {
        if target.metadata().is_ok_and(|metadata| metadata.is_dir()) {
            std::os::windows::fs::symlink_dir(target, link)
        } else {
            std::os::windows::fs::symlink_file(target, link)
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (target, link);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "当前平台不支持符号链接",
        ))
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::create_symlink;
    #[cfg(unix)]
    use tempfile::tempdir;

    #[cfg(unix)]
    #[test]
    fn creates_file_symlink_and_preserves_target() {
        let root = tempdir().unwrap();
        let target = root.path().join("target.txt");
        let link = root.path().join("link.txt");
        std::fs::write(&target, b"data").unwrap();
        create_symlink(&target, &link).unwrap();
        assert_eq!(std::fs::read(&link).unwrap(), b"data");
    }
}
