//! MCP 与 ACP 协议输入的有界单行读取。

use std::io;

use thiserror::Error;
use tokio::io::{AsyncBufRead, AsyncBufReadExt};

/// 单条协议消息的最大 UTF-8 字节数。
pub(crate) const MAX_PROTOCOL_LINE_BYTES: usize = 8 * 1024 * 1024;

/// 协议单行读取错误。
#[derive(Debug, Error)]
pub(crate) enum BoundedLineError {
    #[error("协议单行超过 {max_bytes} 字节上限")]
    TooLong { max_bytes: usize },
    #[error("协议输入不是有效 UTF-8，原因: {reason}")]
    InvalidUtf8 { reason: String },
    #[error("读取协议输入失败: {0}")]
    Io(#[from] io::Error),
}

/// 从异步缓冲读取器读取一行，并限制单行最大字节数。
///
/// 使用 `fill_buf` 分块读取可以在换行符缺失时也及时停止，不会像无界
/// `lines()` 一样先把整行读入内存。超限时调用方应关闭对应协议连接。
pub(crate) async fn read_bounded_line<R>(
    reader: &mut R,
    max_bytes: usize,
) -> Result<Option<String>, BoundedLineError>
where
    R: AsyncBufRead + Unpin,
{
    let mut bytes = Vec::with_capacity(max_bytes.min(8 * 1024));
    loop {
        let buffer = reader.fill_buf().await?;
        if buffer.is_empty() {
            if bytes.is_empty() {
                return Ok(None);
            }
            break;
        }
        if let Some(newline) = buffer.iter().position(|byte| *byte == b'\n') {
            if bytes.len().saturating_add(newline) > max_bytes {
                return Err(BoundedLineError::TooLong { max_bytes });
            }
            bytes.extend_from_slice(&buffer[..newline]);
            reader.consume(newline + 1);
            break;
        }
        if bytes.len().saturating_add(buffer.len()) > max_bytes {
            return Err(BoundedLineError::TooLong { max_bytes });
        }
        bytes.extend_from_slice(buffer);
        let consumed = buffer.len();
        reader.consume(consumed);
    }
    if bytes.last() == Some(&b'\r') {
        bytes.pop();
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|error| BoundedLineError::InvalidUtf8 {
            reason: error.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use tokio::io::BufReader;

    use super::{BoundedLineError, read_bounded_line};

    #[tokio::test]
    async fn reads_and_trims_a_normal_line() {
        let data = b"hello\r\nnext\n".to_vec();
        let mut reader = BufReader::new(data.as_slice());
        assert_eq!(
            read_bounded_line(&mut reader, 32).await.expect("读取失败"),
            Some("hello".to_owned())
        );
        assert_eq!(
            read_bounded_line(&mut reader, 32).await.expect("读取失败"),
            Some("next".to_owned())
        );
    }

    #[tokio::test]
    async fn rejects_oversized_lines_without_newline() {
        let data = vec![b'x'; 32];
        let mut reader = BufReader::new(data.as_slice());
        let error = read_bounded_line(&mut reader, 8)
            .await
            .expect_err("超长行必须被拒绝");
        assert!(matches!(error, BoundedLineError::TooLong { .. }));
    }
}
