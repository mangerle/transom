use russh::client::Msg;
use russh::{Channel, ChannelStream};
use std::io::{self, Read, Write};
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// 把 SSH 通道适配为 `AsyncRead + AsyncWrite` 的传输流。
///
/// 设计原理（Why）：ProxyJump 需要把跳板机上 `direct-tcpip` 通道的下层字节流
/// 直接交给 `russh::client::connect_stream` 作为目标节点的传输层，从而在
/// 同一条加密链路上再叠一层 SSH 握手。
/// - 核心优势：底层 `ChannelStream` 已完成读缓冲残留与基于通道窗口的写背压，
///   这里只做一层透明转发，不再自行维护缓冲区；
/// - 代价与局限：关闭语义跟随通道（EOF/Close），无法像 TCP 那样半关闭后继续
///   读；这对隧道场景是正确行为——通道关闭即整条链路结束。
pub(crate) struct AsyncChannelStream {
    inner: ChannelStream<Msg>,
}

impl AsyncChannelStream {
    pub(crate) fn new(channel: Channel<Msg>) -> Self {
        Self {
            inner: channel.into_stream(),
        }
    }
}

impl AsyncRead for AsyncChannelStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for AsyncChannelStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), io::Error>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<Result<usize, io::Error>> {
        Pin::new(&mut self.inner).poll_write_vectored(cx, bufs)
    }
}

pub(crate) struct ChannelReader {
    receiver: flume::Receiver<Vec<u8>>,
    buffer: Vec<u8>,
    offset: usize,
}

impl ChannelReader {
    pub fn new(receiver: flume::Receiver<Vec<u8>>) -> Self {
        Self {
            receiver,
            buffer: Vec::new(),
            offset: 0,
        }
    }
}

impl Read for ChannelReader {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        loop {
            if self.offset < self.buffer.len() {
                let available = &self.buffer[self.offset..];
                let count = available.len().min(output.len());
                output[..count].copy_from_slice(&available[..count]);
                self.offset += count;
                return Ok(count);
            }
            match self.receiver.recv() {
                Ok(bytes) => {
                    self.buffer = bytes;
                    self.offset = 0;
                }
                Err(_) => return Ok(0),
            }
        }
    }
}

pub(crate) struct ChannelWriter {
    sender: flume::Sender<Vec<u8>>,
}

impl ChannelWriter {
    pub fn new(sender: flume::Sender<Vec<u8>>) -> Self {
        Self { sender }
    }
}

impl Write for ChannelWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.sender
            .send(bytes.to_vec())
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "终端通道已关闭"))?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_round_trips_bytes() {
        let (tx, rx) = flume::bounded(4);
        let mut writer = ChannelWriter::new(tx);
        let mut reader = ChannelReader::new(rx);

        writer.write_all(b"hello ").unwrap();
        writer.write_all(b"world").unwrap();

        let mut output = [0u8; 11];
        reader.read_exact(&mut output).unwrap();
        assert_eq!(&output, b"hello world");
    }
}
