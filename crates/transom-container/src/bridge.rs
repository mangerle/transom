//! 异步通道与阻塞 IO 之间的桥。
//!
//! 设计原理（Why）：终端视图的读写线程是阻塞式的（与 SSH/PTY 保持一致），
//! 而容器 exec、日志与归档都是异步 IO。桥接层只做有界通道搬运：
//! - 异步侧写满通道即等待，天然背压，不会把内存堆爆；
//! - 阻塞侧 `recv()` 等待数据，不占用异步工作线程；
//! - 错误以带上下文的 `io::Error` 交给阻塞侧，tar 等同步解析器可以正常中断。

use std::io::{self, Read, Write};

/// 桥接通道容量：与终端读通道一致，约 32 个数据块。
pub const BRIDGE_CAPACITY: usize = 32;

/// 桥接数据项：`Err` 表示异步侧已失败，阻塞侧应立即报错。
pub type BridgeItem = Result<Vec<u8>, String>;

/// 把有界通道包装成阻塞 [`Read`]。
pub struct BlockingReader {
    receiver: flume::Receiver<BridgeItem>,
    current: Vec<u8>,
    offset: usize,
}

impl BlockingReader {
    pub fn new(receiver: flume::Receiver<BridgeItem>) -> Self {
        Self {
            receiver,
            current: Vec::new(),
            offset: 0,
        }
    }
}

impl Read for BlockingReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        loop {
            if self.offset < self.current.len() {
                let available = &self.current[self.offset..];
                let count = available.len().min(buf.len());
                buf[..count].copy_from_slice(&available[..count]);
                self.offset += count;
                return Ok(count);
            }
            match self.receiver.recv() {
                Ok(Ok(bytes)) => {
                    self.current = bytes;
                    self.offset = 0;
                }
                Ok(Err(message)) => {
                    return Err(io::Error::other(format!("容器数据流读取失败: {message}")));
                }
                Err(_) => return Ok(0),
            }
        }
    }
}

/// 把阻塞 [`Write`] 包装成有界通道发送端。
pub struct BlockingWriter {
    sender: flume::Sender<Vec<u8>>,
}

impl BlockingWriter {
    pub fn new(sender: flume::Sender<Vec<u8>>) -> Self {
        Self { sender }
    }
}

impl Write for BlockingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.sender
            .send(buf.to_vec())
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "容器数据流已关闭"))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// 创建「异步写 → 阻塞读」通道。
pub fn output_channel(capacity: usize) -> (flume::Sender<BridgeItem>, BlockingReader) {
    let (sender, receiver) = flume::bounded(capacity.max(1));
    (sender, BlockingReader::new(receiver))
}

/// 创建「阻塞写 → 异步读」通道。
pub fn input_channel(capacity: usize) -> (BlockingWriter, flume::Receiver<Vec<u8>>) {
    let (sender, receiver) = flume::bounded(capacity.max(1));
    (BlockingWriter::new(sender), receiver)
}

#[cfg(test)]
mod tests {
    use super::{input_channel, output_channel};
    use std::io::{Read, Write};

    #[test]
    fn output_channel_delivers_bytes_and_eof() {
        let (sender, mut reader) = output_channel(4);
        sender.send(Ok(b"hello".to_vec())).expect("发送必须成功");
        sender.send(Ok(b" world".to_vec())).expect("发送必须成功");
        drop(sender);
        let mut buffer = Vec::new();
        reader.read_to_end(&mut buffer).expect("读取必须成功");
        assert_eq!(buffer, b"hello world");
    }

    #[test]
    fn output_channel_surfaces_async_errors() {
        let (sender, mut reader) = output_channel(4);
        sender
            .send(Err("模拟失败".to_string()))
            .expect("发送必须成功");
        let mut buffer = [0u8; 8];
        let error = reader.read(&mut buffer).expect_err("错误必须向上冒泡");
        assert!(error.to_string().contains("模拟失败"));
    }

    #[test]
    fn input_channel_forwards_writes() {
        let (mut writer, receiver) = input_channel(4);
        writer.write_all(b"abc").expect("写入必须成功");
        assert_eq!(receiver.recv().expect("必须收到数据"), b"abc".to_vec());
    }
}
