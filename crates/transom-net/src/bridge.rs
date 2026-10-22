use std::io::{self, Read, Write};

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
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "terminal channel closed"))?;
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
