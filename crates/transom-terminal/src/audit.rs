//! 会话输出旁路审计写入器：把终端输出的原始字节流同步落到本地文件。
//!
//! 设计原理（Why）：
//! - 实现初衷：合规场景（等保、外包运维）要求"谁在什么机器上看到了什么"可追溯。
//!   录屏与截图都不算证据，落原始字节流才能事后按终端语义复核；
//! - 核心优势：写入点选在读取线程内（已经在专用 OS 线程上，不碰 GPUI 主循环），
//!   并且只追加、不改写，落盘失败也绝不阻断终端本身；文件是可 `tail -f` 的
//!   原始输出，不需要专有播放器；
//! - 代价与局限：记录的是"渲染前的字节流"，因此包含全部控制序列（彩色、进度条
//!   这类高频刷新的输出会产生很大文件），也不含时间轴，无法逐帧回放——
//!   asciinema 式的定时回放需要另一套带时间戳的格式，属于独立功能；
//! - 落盘失败的处理：首次失败即停用本会话的审计并记录一条 warn，而不是逐块
//!   重试。理由是磁盘满/目录被删这类故障通常会持续，逐块重试只会刷满日志与
//!   IO；终端可用性优先于审计完整性，这与"不阻断主流程"的既定原则一致。

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// 刷盘间隔：审计要的是"崩溃最多丢一秒"，而不是每块都同步落盘。
const FLUSH_INTERVAL: Duration = Duration::from_secs(1);

/// 单个会话的输出审计写入器。
pub(crate) struct AuditSink {
    path: PathBuf,
    /// `None` 表示尚未打开（首次写入时创建）或已因故障停用。
    writer: Option<BufWriter<File>>,
    last_flush: Instant,
}

impl AuditSink {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            path,
            writer: None,
            // 首次写入即触发一次 flush 判定，避免"打开后一直不刷"。
            last_flush: Instant::now() - FLUSH_INTERVAL,
        }
    }

    /// 记录一块输出；任何失败都只影响审计本身，不向调用方传播错误。
    pub(crate) fn record(&mut self, bytes: &[u8]) {
        if self.writer.is_none() && !self.open() {
            return;
        }
        let Some(writer) = self.writer.as_mut() else {
            return;
        };
        if let Err(error) = writer.write_all(bytes) {
            self.disable(&error);
            return;
        }
        if self.last_flush.elapsed() >= FLUSH_INTERVAL
            && let Err(error) = writer.flush()
        {
            self.disable(&error);
        }
    }

    /// 会话结束时把缓冲内容刷完（BufWriter 析构也会刷，但失败要留痕）。
    pub(crate) fn finish(&mut self) {
        let Some(mut writer) = self.writer.take() else {
            return;
        };
        if let Err(error) = writer.flush() {
            tracing::warn!(
                "会话审计日志收尾刷盘失败，可能丢失最后一秒输出，路径: {:?}，原因: {error}",
                self.path
            );
        }
    }

    /// 打开（或创建追加）审计文件；失败时记录一次并返回 `false`。
    fn open(&mut self) -> bool {
        if let Some(parent) = self.path.parent()
            && let Err(error) = std::fs::create_dir_all(parent)
        {
            tracing::warn!("创建审计目录失败，本会话不落盘，路径: {parent:?}，原因: {error}");
            return false;
        }
        match OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            Ok(file) => {
                // 64 KiB 缓冲：单次 `cat` 大文件也不会每块一次系统调用。
                self.writer = Some(BufWriter::with_capacity(64 * 1024, file));
                tracing::info!("会话审计日志已开始写入，路径: {:?}", self.path);
                true
            }
            Err(error) => {
                tracing::warn!(
                    "创建会话审计日志失败，本会话不落盘，路径: {:?}，原因: {error}",
                    self.path
                );
                false
            }
        }
    }

    fn disable(&mut self, error: &std::io::Error) {
        tracing::warn!(
            "写入会话审计日志失败，本会话停止审计以保护终端可用性，路径: {:?}，原因: {error}",
            self.path
        );
        self.writer = None;
    }
}

impl Drop for AuditSink {
    fn drop(&mut self) {
        self.finish();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(tag: &str) -> PathBuf {
        static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "transom-audit-{}-{tag}-{seq}.log",
            std::process::id()
        ))
    }

    /// 写入即追加，且未刷盘的内容也必须能在文件里读到（Drop 收尾刷盘）。
    #[test]
    fn records_and_flushes_on_drop() {
        let path = temp_path("basic");
        let mut sink = AuditSink::new(path.clone());
        sink.record(b"ls -la\r\n");
        sink.record(b"total 8\r\n");
        drop(sink);
        let content = std::fs::read(&path).unwrap();
        assert_eq!(content, b"ls -la\r\ntotal 8\r\n");
        let _ = std::fs::remove_file(&path);
    }

    /// 周期性刷盘：超过间隔的下一次写入必须把缓冲落到磁盘，
    /// 这样进程被 kill 时最多丢一秒。
    #[test]
    fn flushes_periodically() {
        let path = temp_path("flush");
        {
            let mut sink = AuditSink::new(path.clone());
            sink.record(b"first\r\n");
            sink.last_flush = Instant::now() - FLUSH_INTERVAL;
            sink.record(b"second\r\n");
            // 不依赖 Drop：此处直接读文件，必须已含第二次写入。
            let content = std::fs::read(&path).unwrap();
            assert!(
                content.windows(6).any(|w| w == b"second"),
                "刷盘后应能读到第二块内容，实际: {content:?}"
            );
        }
        let _ = std::fs::remove_file(&path);
    }

    /// 目录不可创建（父路径是普通文件）时停用而不是 panic，且不阻断后续调用。
    #[test]
    fn disables_silently_when_path_is_unusable() {
        let blocker = temp_path("blocker");
        std::fs::write(&blocker, b"not a dir").unwrap();
        let path = blocker.join("nested").join("audit.log");
        let mut sink = AuditSink::new(path);
        sink.record(b"ignored\r\n");
        sink.record(b"ignored again\r\n");
        drop(sink);
        assert!(!blocker.join("nested").exists());
        let _ = std::fs::remove_file(&blocker);
    }
}
