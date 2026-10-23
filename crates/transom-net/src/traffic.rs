//! 传输层字节计数流包装器。

use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use transom_core::TrafficCounter;

/// 在 SSH 加密流外层透明统计读写字节数。
///
/// 通过 `russh::client::connect_stream` 注入，可覆盖同一条 SSH 连接上的
/// 所有通道（终端、SFTP、转发），避免逐通道统计带来的遗漏。
pub(crate) struct CountingStream<S> {
    inner: S,
    counter: Arc<TrafficCounter>,
}

impl<S> CountingStream<S> {
    pub(crate) fn new(inner: S, counter: Arc<TrafficCounter>) -> Self {
        Self { inner, counter }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for CountingStream<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.as_mut().get_mut();
        let filled_before = buf.filled().len();
        let poll = Pin::new(&mut this.inner).poll_read(cx, buf);
        if let Poll::Ready(Ok(())) = &poll {
            this.counter
                .add_received(buf.filled().len().saturating_sub(filled_before));
        }
        poll
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for CountingStream<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        let this = self.as_mut().get_mut();
        let poll = Pin::new(&mut this.inner).poll_write(cx, buf);
        if let Poll::Ready(Ok(written)) = &poll {
            this.counter.add_sent(*written);
        }
        poll
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<Result<usize, io::Error>> {
        let this = self.as_mut().get_mut();
        let poll = Pin::new(&mut this.inner).poll_write_vectored(cx, bufs);
        if let Poll::Ready(Ok(written)) = &poll {
            this.counter.add_sent(*written);
        }
        poll
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn counts_bytes_in_both_directions() {
        let (left, right) = tokio::io::duplex(64);
        let counter = Arc::new(TrafficCounter::default());
        let mut counting = CountingStream::new(left, counter.clone());
        let mut peer = right;

        counting.write_all(b"hello").await.expect("写入测试数据");
        let mut buffer = [0u8; 5];
        peer.read_exact(&mut buffer).await.expect("读取测试数据");
        assert_eq!(&buffer, b"hello");
        assert_eq!(counter.totals(), (0, 5));

        peer.write_all(b"world").await.expect("写入回程数据");
        counting
            .read_exact(&mut buffer)
            .await
            .expect("读取回程数据");
        assert_eq!(&buffer, b"world");
        assert_eq!(counter.totals(), (5, 5));
    }
}
