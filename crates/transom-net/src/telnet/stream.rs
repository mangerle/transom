//! Telnet 线路写出端与读写 I/O 适配器。

use crate::telnet_iac::{IacParser, escape_iac};
use std::io::{self, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;
use transom_core::{TerminalIo, TextEncoding};

/// 读取超时；同时决定关闭请求的最长响应延迟（与串口同口径）。
pub(crate) const READ_TIMEOUT: Duration = Duration::from_millis(100);
/// 读块大小；编码解码后最多膨胀 3 倍（GBK 双字节到 UTF-8 三字节）。
pub(crate) const READ_CHUNK: usize = 4096;

/// 已建立的 Telnet 终端通道。
pub struct TelnetSession {
    pub io: TerminalIo,
    /// 读取端退出（对端断开、关闭或读错误）时触发。
    pub exit: tokio::sync::oneshot::Receiver<()>,
}

/// 线路写出端：用户数据、协商应答、NAWS 与保活 NOP 的唯一出口。
pub(crate) struct TelnetWire {
    pub(crate) inner: Mutex<Box<dyn Write + Send>>,
}

impl TelnetWire {
    pub(crate) fn send_raw(&self, bytes: &[u8]) -> io::Result<()> {
        self.lock().write_all(bytes)
    }

    /// 统一处理锁中毒：数据本体是 TCP 流，panic 不会破坏其内部一致性，
    /// 直接取回内部值继续使用，不向调用方扩散毒化错误。
    fn lock(&self) -> MutexGuard<'_, Box<dyn Write + Send>> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Telnet 读取端：剥离协议命令、按会话编码转码为 UTF-8。
pub(crate) struct TelnetReader {
    pub(crate) stream: Box<dyn Read + Send>,
    pub(crate) wire: Arc<TelnetWire>,
    pub(crate) parser: IacParser,
    /// `None` 表示 UTF-8 直通；其余编码持有跨块保留部分序列状态的解码器。
    pub(crate) decoder: Option<encoding_rs::Decoder>,
    /// 协议剥离后的数据字节（远端编码），跨轮次复用避免高频重分配。
    pub(crate) clean: Vec<u8>,
    /// 转码后的 UTF-8 字节及其读取偏移；单次 read 取不完时留下次续取。
    pub(crate) pending: Vec<u8>,
    pub(crate) offset: usize,
    /// 解码用的字符串缓冲，与 `pending` 同生命周期复用。
    pub(crate) decoded: String,
    pub(crate) closed: Arc<AtomicBool>,
    pub(crate) exit: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Read for TelnetReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.closed.load(Ordering::Acquire) {
                return Ok(0);
            }
            if self.offset < self.pending.len() {
                let count = buffer.len().min(self.pending.len() - self.offset);
                buffer[..count].copy_from_slice(&self.pending[self.offset..self.offset + count]);
                self.offset += count;
                if self.offset == self.pending.len() {
                    self.pending.clear();
                    self.offset = 0;
                }
                return Ok(count);
            }

            let mut chunk = [0u8; READ_CHUNK];
            match self.stream.read(&mut chunk) {
                Ok(0) => return Ok(0),
                Ok(count) => {
                    let wire = self.wire.clone();
                    self.parser
                        .feed(&chunk[..count], &mut self.clean, &mut |bytes| {
                            if let Err(error) = wire.send_raw(bytes) {
                                log::warn!("写入 Telnet 协议应答失败: {error}");
                            }
                        });
                    if self.clean.is_empty() {
                        continue;
                    }
                    self.decode_clean();
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::TimedOut
                            | io::ErrorKind::WouldBlock
                            | io::ErrorKind::Interrupted
                    ) => {}
                Err(error) => return Err(error),
            }
        }
    }
}

impl TelnetReader {
    fn decode_clean(&mut self) {
        match self.decoder.as_mut() {
            Some(decoder) => {
                self.decoded.clear();
                let mut consumed = 0;
                while consumed < self.clean.len() {
                    let (result, read, _) =
                        decoder.decode_to_string(&self.clean[consumed..], &mut self.decoded, false);
                    consumed += read;
                    if matches!(result, encoding_rs::CoderResult::OutputFull) {
                        self.decoded.reserve(self.clean.len() * 3 + 8);
                    } else {
                        break;
                    }
                }
                self.pending.extend_from_slice(self.decoded.as_bytes());
            }
            None => self.pending.extend_from_slice(&self.clean),
        }
        self.clean.clear();
    }
}

impl Drop for TelnetReader {
    fn drop(&mut self) {
        if let Some(exit) = self.exit.take() {
            let _ = exit.send(());
        }
    }
}

/// Telnet 写入端：按会话编码转码并转义 0xFF 后写线路。
pub(crate) struct TelnetWriter {
    pub(crate) wire: Arc<TelnetWire>,
    /// `None` 表示 UTF-8 直通；其余编码持有跨写入复用的编码器。
    pub(crate) encoder: Option<encoding_rs::Encoder>,
    /// 编码产物与转义产物，跨轮次复用。
    pub(crate) encoded: Vec<u8>,
    pub(crate) escaped: Vec<u8>,
}

impl Write for TelnetWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        match self.encoder.as_mut() {
            Some(encoder) => {
                let text = std::str::from_utf8(buffer).map_err(|_| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "终端输入不是合法 UTF-8，无法按会话编码转码",
                    )
                })?;
                self.encoded.clear();
                self.encoded.reserve(text.len() * 3 + 8);
                encode_input(encoder, text, &mut self.encoded);
                self.escaped.clear();
                escape_iac(&self.encoded, &mut self.escaped);
            }
            None => {
                self.escaped.clear();
                escape_iac(buffer, &mut self.escaped);
            }
        }
        self.wire.send_raw(&self.escaped)?;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn encode_input(encoder: &mut encoding_rs::Encoder, text: &str, out: &mut Vec<u8>) {
    let mut consumed = 0;
    while consumed < text.len() {
        let (result, read, _had_unmappable) =
            encoder.encode_from_utf8_to_vec(&text[consumed..], out, false);
        consumed += read;
        match result {
            encoding_rs::CoderResult::InputEmpty => break,
            encoding_rs::CoderResult::OutputFull => out.reserve(text.len() * 3 + 8),
        }
    }
}

pub(crate) fn encoding_for(encoding: TextEncoding) -> Option<&'static encoding_rs::Encoding> {
    match encoding {
        TextEncoding::Utf8 => None,
        TextEncoding::Gbk => Some(encoding_rs::GBK),
        TextEncoding::Big5 => Some(encoding_rs::BIG5),
    }
}
