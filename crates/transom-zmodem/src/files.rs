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

/// 下载写盘的分块上限：攒够 64 KiB 再落盘，避免每个 1 KiB 子包一次系统调用。
pub(crate) const DOWNLOAD_WRITE_CHUNK: usize = 64 * 1024;
/// 两次 `sync_all` 之间允许累积的未同步字节数（1 MiB）。
///
/// 每个 64 KiB 块都 fsync 会让机械盘上的百兆传输被同步写放大拖慢；
/// 1 MiB 是"崩溃最多丢一秒内数据"与"不拖垮吞吐"之间的折中。
pub(crate) const DOWNLOAD_SYNC_INTERVAL: u64 = 1024 * 1024;

/// 打开上传文件所需的最小元信息，一次 stat 取齐。
pub(crate) struct UploadFile {
    pub(crate) file: std::fs::File,
    pub(crate) size: u64,
    /// 修改时间（秒）；文件系统给不出时为空，由发送端按 0 填。
    pub(crate) mtime: Option<u32>,
    /// 展示给对端的裸文件名（已去掉目录部分）。
    pub(crate) name: String,
}

/// 下载落盘写入器：流式分块 + 周期性同步。
///
/// 设计原理（Why）：ZMODEM 子包只有 1 KiB，逐包 `write_all` 会把大文件传输
/// 变成密集小写；先攒到 64 KiB 再落盘可显著减少系统调用。`sync_all` 按 1 MiB
/// 节流，保证异常断电时最多丢失约一秒的数据，又不让每条 64 KiB 都等待磁盘。
///
/// 代价与局限：进程被强杀时缓冲区内的最后不足 64 KiB 会丢失；正常结束或
/// 文件切换都会显式 [`Self::finish`] flushing，覆盖协议层的所有收尾路径。
pub(crate) struct DownloadWriter {
    file: std::fs::File,
    path: PathBuf,
    buffer: Vec<u8>,
    unsynced: u64,
}

impl DownloadWriter {
    /// 在下载目录下创建待写入的文件。
    ///
    /// `name` 已由接收端净化为裸文件名；这里再挡一道带分隔符的输入，确保落点的父
    /// 目录恰为受控下载目录（AGENTS.md 路径防穿越红线），不让 `join` 直接放行。
    ///
    /// # Errors
    /// 目录不可写、名字越出目录、目标已存在为目录等文件系统错误原样返回。
    pub(crate) fn create(dir: &Path, name: &str) -> std::io::Result<Self> {
        let path = dir.join(name);
        if path.parent() != Some(dir) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "下载文件名越出下载目录",
            ));
        }
        let file = std::fs::File::create(&path)?;
        Ok(Self {
            file,
            path,
            buffer: Vec::with_capacity(DOWNLOAD_WRITE_CHUNK),
            unsynced: 0,
        })
    }

    /// 实际落点，供日志与测试核对。
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// 追加一段数据；攒满 64 KiB 时落盘，并在累积 1 MiB 后同步。
    ///
    /// # Errors
    /// 磁盘满、句柄失效等写盘错误原样返回，由状态机决定中止会话。
    pub(crate) fn write(&mut self, data: &[u8]) -> std::io::Result<()> {
        self.buffer.extend_from_slice(data);
        if self.buffer.len() >= DOWNLOAD_WRITE_CHUNK {
            self.flush_buffer()?;
        }
        Ok(())
    }

    /// 收尾：冲刷缓冲并强制同步到磁盘。
    ///
    /// # Errors
    /// 写入或同步失败原样返回；调用方只记录日志，不再重试。
    pub(crate) fn finish(&mut self) -> std::io::Result<()> {
        self.flush_buffer()?;
        self.unsynced = 0;
        self.file.sync_all()
    }

    fn flush_buffer(&mut self) -> std::io::Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        self.file.write_all(&self.buffer)?;
        self.unsynced = self.unsynced.saturating_add(self.buffer.len() as u64);
        self.buffer.clear();
        if self.unsynced >= DOWNLOAD_SYNC_INTERVAL {
            self.file.sync_all()?;
            self.unsynced = 0;
        }
        Ok(())
    }
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
        let writer = DownloadWriter::create(dir.path(), "ok.bin").expect("正常名字应可创建");
        assert_eq!(writer.path().parent(), Some(dir.path()));
        assert!(DownloadWriter::create(dir.path(), "../escape.bin").is_err());
        assert!(DownloadWriter::create(dir.path(), "a/b.bin").is_err());
    }

    /// 小于分块阈值的数据在 `finish` 前不落盘，收尾后必须完整可见。
    #[test]
    fn download_writer_flushes_on_finish() {
        let dir = tempfile::tempdir().expect("临时目录创建");
        let mut writer = DownloadWriter::create(dir.path(), "small.bin").expect("创建写入器");
        writer.write(b"hello ").expect("写入第一段");
        writer.write(b"world").expect("写入第二段");
        assert_eq!(
            std::fs::read(dir.path().join("small.bin")).expect("读取落盘文件"),
            Vec::<u8>::new(),
            "未到 64 KiB 且未 finish 前不应落盘"
        );
        writer.finish().expect("收尾同步");
        assert_eq!(
            std::fs::read(dir.path().join("small.bin")).expect("读取落盘文件"),
            b"hello world"
        );
    }

    /// 超过分块阈值必须及时落盘，而不是一直攒到文件结束。
    #[test]
    fn download_writer_flushes_at_chunk_boundary() {
        let dir = tempfile::tempdir().expect("临时目录创建");
        let mut writer = DownloadWriter::create(dir.path(), "big.bin").expect("创建写入器");
        let chunk = vec![0x5Au8; DOWNLOAD_WRITE_CHUNK];
        writer.write(&chunk).expect("写入整块");
        let flushed = std::fs::metadata(dir.path().join("big.bin"))
            .expect("读取元数据")
            .len();
        assert_eq!(flushed, DOWNLOAD_WRITE_CHUNK as u64);
        writer.finish().expect("收尾同步");
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
