//! ZMODEM 文件读写的执行层：把系统调用从会话状态机里隔出来。
//!
//! 设计原理（Why）：状态机（[`super::fsm`]）只做"字节/定时器 → 动作"的纯转换，
//! 一旦掺进 `File::create`、`seek`、`metadata` 这些调用，既无法用合成字节流逐帧
//! 断言，也会让"落盘失败"与"协议失败"两类语义在同一个方法里交织。本层只负责执行
//! 并原样返回 `io::Result`，失败如何反馈给对端（ZSKIP／取消／中止）仍由状态机决定。
//!
//! 代价：调用方要多写一次结果匹配，换来的是文件通道相关的分支可以在本文件用真实
//! 临时目录单独测试，状态机的用例则完全不碰磁盘。

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// 打开上传文件所需的最小元信息，一次 stat 取齐。
pub(crate) struct UploadFile {
    pub(crate) file: std::fs::File,
    pub(crate) size: u64,
    /// 修改时间（秒）；文件系统给不出时为空，由发送端按 0 填。
    pub(crate) mtime: Option<u32>,
    /// 展示给对端的裸文件名（已去掉目录部分）。
    pub(crate) name: String,
}

/// 在下载目录下创建待写入的文件，返回句柄与实际落点。
///
/// `name` 已由接收端净化为裸文件名；这里再挡一道带分隔符的输入，确保落点的父目录
/// 恰为受控下载目录（AGENTS.md 路径防穿越红线），不让 `join` 直接放行了事。
///
/// # Errors
/// 目录不可写、名字越出目录、目标已存在为目录等文件系统错误原样返回。
pub(crate) fn create_download_file(
    dir: &Path,
    name: &str,
) -> std::io::Result<(std::fs::File, PathBuf)> {
    let path = dir.join(name);
    if path.parent() != Some(dir) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "下载文件名越出下载目录",
        ));
    }
    let file = std::fs::File::create(&path)?;
    Ok((file, path))
}

/// 追加写盘：调用方持有已创建的句柄。
///
/// # Errors
/// 磁盘满、句柄失效等写盘错误原样返回，由状态机决定中止会话。
pub(crate) fn append(file: &mut std::fs::File, data: &[u8]) -> std::io::Result<()> {
    file.write_all(data)
}

/// 打开待上传文件并一次取齐大小、时间与裸文件名。
///
/// # Errors
/// 文件不存在、无读权限或 stat 失败时原样返回，由状态机跳过该文件。
pub(crate) fn open_upload_file(path: &Path) -> std::io::Result<UploadFile> {
    let file = std::fs::File::open(path)?;
    let meta = file.metadata()?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs() as u32);
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "upload.bin".to_string());
    Ok(UploadFile {
        file,
        size: meta.len(),
        mtime,
        name,
    })
}

/// 按给定偏移读取下一块，返回读到的字节数（0 表示已到文件尾）。
///
/// 偏移由状态机维护（对端 ZRPOS 续传点 + 已发字节数），本层只做 seek + read，
/// 因此对端要求重传任意位置时不需要重新打开句柄。
///
/// # Errors
/// 定位或读取失败（句柄失效、IO 错误）原样返回。
pub(crate) fn read_block(
    file: &mut std::fs::File,
    offset: u64,
    buf: &mut [u8],
) -> std::io::Result<usize> {
    file.seek(SeekFrom::Start(offset))?;
    file.read(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 净化后的裸文件名之外，带分隔符与 `..` 的名字都必须被拒绝落盘。
    #[test]
    fn download_path_stays_inside_directory() {
        let dir = tempfile::tempdir().expect("临时目录创建");
        let (file, path) = create_download_file(dir.path(), "ok.bin").expect("正常名字应可创建");
        assert_eq!(path.parent(), Some(dir.path()));
        drop(file);
        assert!(create_download_file(dir.path(), "../escape.bin").is_err());
        assert!(create_download_file(dir.path(), "a/b.bin").is_err());
    }

    /// 续传读取必须严格按偏移取块，保证坏包后的重传位置正确。
    #[test]
    fn read_block_honours_the_requested_offset() {
        let dir = tempfile::tempdir().expect("临时目录创建");
        let path = dir.path().join("data.bin");
        std::fs::write(&path, b"0123456789").expect("写入测试数据");
        let opened = open_upload_file(&path).expect("打开上传文件");
        assert_eq!(opened.size, 10);
        assert_eq!(opened.name, "data.bin");
        let mut file = opened.file;
        let mut buf = [0u8; 4];
        assert_eq!(read_block(&mut file, 3, &mut buf).expect("读取成功"), 4);
        assert_eq!(&buf, b"3456");
        assert_eq!(read_block(&mut file, 8, &mut buf).expect("读取成功"), 2);
    }
}
