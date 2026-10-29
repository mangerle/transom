//! 对比面板的本地文件读写：阻塞实现，统一由后台线程调度。
//!
//! 设计原理（Why）：
//! - 读：先看 `metadata().len()` 再读，超限直接拒绝，避免把超大文件整份拉进内存；
//!   读后仍按 UTF-8 校验，二进制文件在差异视图里没有意义；
//! - 写：同目录临时文件 + 重命名，保证写到一半崩溃不会留下半截文件；
//!   先复制原文件权限位，避免合并后脚本失去执行位。
//!
//! 代价与局限：
//! - 不生成本地备份文件：历史版本交给用户自己的版本控制；
//! - Windows 上重命名覆盖已存在文件会失败，失败时回退为"删除后重命名"，
//!   该回退不是原子操作（本项目主要目标平台为 Linux）。

use std::path::Path;

/// 读取本地 UTF-8 文本文件。
pub(crate) fn read_local_text(path: &Path) -> Result<String, String> {
    let meta = std::fs::metadata(path)
        .map_err(|error| format!("读取本地文件属性失败（{}）: {error}", path.display()))?;
    if !meta.is_file() {
        return Err(format!("不是普通文件：{}", path.display()));
    }
    if meta.len() > super::MAX_DIFF_BYTES {
        return Err(format!(
            "本地文件超过对比上限（{} 字节）：{}",
            super::MAX_DIFF_BYTES,
            path.display()
        ));
    }
    let bytes = std::fs::read(path)
        .map_err(|error| format!("读取本地文件失败（{}）: {error}", path.display()))?;
    if bytes.len() as u64 > super::MAX_DIFF_BYTES {
        // 元数据与真实读取之间文件可能被追加；以真实字节数为准再拦一次。
        return Err(format!(
            "本地文件在读取期间超过对比上限：{}",
            path.display()
        ));
    }
    String::from_utf8(bytes).map_err(|_| format!("本地文件不是 UTF-8 文本：{}", path.display()))
}

/// 原子替换本地文本文件，保留原有权限位。
///
/// 写前重新读取并与 `expected` 比对：对比面板可能开了很久，期间用户或别的程序
/// 改过文件；不比对就覆盖等于静默丢弃别人的修改。比对与替换之间仍有极小的
/// TOCTOU 窗口，但已把"长时间滞留导致的覆盖"这类真实场景挡住。
pub(crate) fn write_local_text(path: &Path, expected: &str, text: &str) -> Result<(), String> {
    let current = read_local_text(path)?;
    if current != expected {
        return Err(format!(
            "本地文件已被其他程序修改（{}），已取消本次合并",
            path.display()
        ));
    }
    let meta = std::fs::metadata(path)
        .map_err(|error| format!("读取本地文件属性失败（{}）: {error}", path.display()))?;
    let parent = path
        .parent()
        .ok_or_else(|| format!("无法确定文件所在目录：{}", path.display()))?;
    let name = path
        .file_name()
        .ok_or_else(|| format!("非法文件路径：{}", path.display()))?
        .to_string_lossy();
    let temp = parent.join(format!(".{name}.transom-tmp-{}", std::process::id()));
    let result = (|| {
        std::fs::write(&temp, text)
            .map_err(|error| format!("写入临时文件失败（{}）: {error}", temp.display()))?;
        std::fs::set_permissions(&temp, meta.permissions())
            .map_err(|error| format!("设置文件权限失败（{}）: {error}", temp.display()))?;
        match std::fs::rename(&temp, path) {
            Ok(()) => Ok(()),
            Err(_) if cfg!(windows) => {
                // Windows 不允许 rename 覆盖既有文件；回退路径牺牲原子性换取可用性。
                std::fs::remove_file(path)
                    .map_err(|error| format!("替换本地文件失败（{}）: {error}", path.display()))?;
                std::fs::rename(&temp, path)
                    .map_err(|error| format!("替换本地文件失败（{}）: {error}", path.display()))
            }
            Err(error) => Err(format!("替换本地文件失败（{}）: {error}", path.display())),
        }
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}
