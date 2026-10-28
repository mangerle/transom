//! 存储层基础文件读写：JSON 解码与「临时文件 + fsync + rename」原子写入。
//!
//! 设计原理（Why）：所有持久化文件（会话、设置、known_hosts、历史、备份导出）
//! 都必须"要么旧内容、要么新内容"地原子更新，私有文件权限同时收紧到 0600。
//! 这三处写入点曾各自复述整套流程，任何一处漏掉 fsync 都会在断电时产出空
//! 文件或旧内容；收敛到本模块后只维护一份实现。

use serde::{Serialize, de::DeserializeOwned};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use transom_core::{AppError, ErrorCode};
use transom_platform::private_fs;

pub(crate) fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, AppError> {
    match fs::read_to_string(path) {
        Ok(text) => match serde_json::from_str(&text) {
            Ok(value) => Ok(Some(value)),
            Err(error) => {
                let detail = match quarantine(path, "corrupt") {
                    Some(saved) => format!(
                        "配置文件内容损坏，无法解析，原文件已保留到 {}，路径: {}，原因: {error}",
                        saved.display(),
                        path.display()
                    ),
                    None => format!(
                        "配置文件内容损坏，无法解析，路径: {}，原因: {error}",
                        path.display()
                    ),
                };
                Err(AppError::with_detail(ErrorCode::Configuration, detail))
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(AppError::with_detail(
            ErrorCode::Io,
            format!("读取配置文件失败，路径: {}，原因: {error}", path.display()),
        )),
    }
}

/// 把无法解析或版本不适合当前程序的文件改名保留，返回隔离后的路径。
///
/// 设计原理（Why）：加载失败后调用方通常以空配置继续运行，下一次保存会原子
/// 覆盖原文件，原始数据就此永久消失。先改名保留，用户仍能从隔离文件里找回
/// 内容，也便于事后排查；改名失败只告警，不掩盖原始错误。
pub(crate) fn quarantine(path: &Path, reason: &str) -> Option<PathBuf> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();
    let name = path.file_name().map_or_else(
        || "store".to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    let mut target = path.with_file_name(format!("{name}.{reason}-{stamp}"));
    let mut sequence = 1u32;
    while target.exists() {
        target = path.with_file_name(format!("{name}.{reason}-{stamp}-{sequence}"));
        sequence = sequence.saturating_add(1);
    }
    match fs::rename(path, &target) {
        Ok(()) => {
            log::warn!(
                "配置文件已隔离保留，原路径: {}，隔离路径: {}，原因: {reason}",
                path.display(),
                target.display()
            );
            Some(target)
        }
        Err(error) => {
            log::warn!(
                "配置文件隔离失败，原路径: {}，原因: {error}",
                path.display()
            );
            None
        }
    }
}

/// 把 JSON 以私有文件形式原子写盘（临时文件 + fsync + rename）。
pub(crate) fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), AppError> {
    let data = serde_json::to_vec_pretty(value)
        .map_err(|error| AppError::with_detail(ErrorCode::Configuration, error.to_string()))?;
    write_atomic(path, &data)
}

/// 原子写入的完整流程：唯一临时名 -> 写入 -> fsync -> rename 覆盖目标。
///
/// 设计原理（Why）：
/// - 为什么要 fsync：只 flush 是把数据交给内核页缓存，rename 之后断电仍
///   可能得到空文件或旧内容——rename 的原子性只保证"要么旧要么新"，不
///   保证新内容已持久化。会话目录与主机指纹丢一次就要用户重配或重新确认；
/// - 为什么临时名带 pid 与序号：固定 `.tmp` 名在两个写者并发时互相覆盖，
///   可能把交错内容 rename 成正式文件；pid 隔离同机多进程，进程内递增
///   序号隔离同进程多线程；
/// - 为什么显式关闭句柄再 rename：Windows 不允许重命名打开中的文件，
///   提前关闭让两个平台行为一致；
/// - 为什么 rename 后还要 fsync 父目录：rename 只原子替换目录项，目录项
///   本身也是元数据；不同步父目录时，断电后可能仍指向旧文件或丢失新名字。
/// - 代价与局限：每次保存多一次文件 fsync 与一次目录 fsync（毫秒级），
///   写频率低可接受；极端崩溃可能残留唯一命名的 `.tmp` 文件，属无害垃圾，
///   不会被后续写入读到。Windows 没有等价的可移植目录 fsync，且 rename
///   不覆盖已存在文件、只能先删除，因此存在短暂缺失窗口；本实现不承诺
///   Windows 上的掉电持久性，只保证不会留下半份内容。
///
/// # Errors
/// 父目录创建、临时文件打开、写入、同步、重命名或目录同步失败时返回
/// `Io` 错误，错误信息附带目标路径便于定位。
pub fn write_atomic(path: &Path, data: &[u8]) -> Result<(), AppError> {
    let parent = path.parent().ok_or_else(|| {
        AppError::with_detail(ErrorCode::Configuration, "存储路径非法：没有父目录可写")
    })?;
    private_fs::ensure_private_dir(parent).map_err(|error| write_error(path, error))?;

    let temp = temp_sibling(path);
    let mut temp_file =
        private_fs::open_private_truncate(&temp).map_err(|error| write_error(&temp, error))?;
    temp_file
        .write_all(data)
        .map_err(|error| write_error(&temp, error))?;
    // flush 把数据交给内核，sync_all 才保证数据与元数据落到介质。
    temp_file
        .flush()
        .map_err(|error| write_error(&temp, error))?;
    temp_file
        .sync_all()
        .map_err(|error| write_error(&temp, error))?;
    drop(temp_file);
    #[cfg(windows)]
    {
        // Windows rename 不覆盖已存在文件；先删除旧文件。NotFound 表示首次
        // 写入，其它删除失败必须上报，否则 rename 必然失败、用户以为写成功。
        if let Err(error) = fs::remove_file(path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            return Err(write_error(path, error));
        }
    }
    fs::rename(&temp, path).map_err(|error| write_error(path, error))?;
    #[cfg(unix)]
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| write_error(parent, error))?;
    Ok(())
}

/// 目标文件的唯一临时兄弟名：`<原名>.<pid>.<序号>.tmp`。
fn temp_sibling(path: &Path) -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let name = path.file_name().map_or_else(
        || "store".to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    path.with_file_name(format!("{name}.{}.{seq}.tmp", std::process::id()))
}

/// 原子写路径上的统一 IO 错误描述：附带目标路径便于定位。
fn write_error(path: &Path, error: std::io::Error) -> AppError {
    AppError::with_detail(
        ErrorCode::Io,
        format!("写入文件失败，路径: {}，原因: {error}", path.display()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// 临时名唯一：连续生成不重名，且带进程号（同机多进程不冲突）。
    #[test]
    fn temp_sibling_is_unique_per_call() {
        let path = Path::new("/tmp/demo/settings.json");
        let first = temp_sibling(path);
        let second = temp_sibling(path);
        assert_ne!(first, second, "同一目标的两次临时名不得相同");
        let pid = std::process::id().to_string();
        assert!(
            first.to_string_lossy().contains(&pid),
            "临时名必须携带进程号以隔离多进程"
        );
        assert_eq!(first.parent(), path.parent(), "临时文件必须与目标同目录");
    }

    /// 覆盖写：内容完整替换，成功路径不残留临时文件。
    #[test]
    fn write_atomic_replaces_content_without_leftovers() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("data.json");
        write_atomic(&path, b"first").unwrap();
        write_atomic(&path, b"second").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"second");
        let leftovers = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(leftovers, 0, "成功写入后不得残留临时文件");
    }

    /// Unix 下目标文件权限必须是 0600。
    #[cfg(unix)]
    #[test]
    fn write_atomic_keeps_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().unwrap();
        let path = dir.path().join("secret.json");
        write_atomic(&path, b"{}").unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "私有文件必须仅属主可读写");
    }
}

#[cfg(test)]
mod quarantine_tests {
    use super::read_json;
    use tempfile::tempdir;

    /// 损坏的配置文件必须改名保留：否则界面空启动后的第一次保存会把原数据
    /// 覆盖掉，用户再也没有找回的入口。
    #[test]
    fn corrupt_json_is_quarantined_and_preserved() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("sessions.json");
        std::fs::write(&path, b"{ broken").unwrap();

        let result = read_json::<serde_json::Value>(&path);
        assert!(result.is_err());
        assert!(!path.exists(), "原文件应被改名隔离");

        let quarantined: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".corrupt-"))
            .collect();
        assert_eq!(quarantined.len(), 1, "必须恰好保留一份隔离文件");
        assert_eq!(
            std::fs::read(quarantined[0].path()).unwrap(),
            b"{ broken",
            "隔离文件必须保留原始内容"
        );
    }
}
