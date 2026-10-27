//! Telnet 客户端单元测试。

use super::stream::{READ_CHUNK, TelnetReader, TelnetWire, TelnetWriter, encoding_for};
use crate::telnet_iac::{IAC, IacParser, OPT_TTYPE, WILL};
use std::io::{self, Cursor, Read, Write};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use transom_core::TextEncoding;

#[derive(Clone)]
struct SharedSink(Arc<Mutex<Vec<u8>>>);

impl SharedSink {
    fn new() -> (Self, Arc<Mutex<Vec<u8>>>) {
        let shared = Arc::new(Mutex::new(Vec::new()));
        (Self(shared.clone()), shared)
    }
}

impl Write for SharedSink {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn wire_with_sink(sink: SharedSink) -> Arc<TelnetWire> {
    Arc::new(TelnetWire {
        inner: Mutex::new(Box::new(sink)),
    })
}

fn drain_reader(reader: &mut TelnetReader) -> io::Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut buffer = [0u8; READ_CHUNK];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => output.extend_from_slice(&buffer[..count]),
            Err(error) => return Err(error),
        }
    }
    Ok(output)
}

#[test]
fn reader_strips_negotiation_and_decodes_gbk() {
    let source = [&[IAC, 253u8, OPT_TTYPE][..], &[0xb5, 0xc7, 0xc2, 0xbc]].concat();
    let (sink, shared) = SharedSink::new();
    let closed = Arc::new(AtomicBool::new(false));
    let (exit_tx, _exit_rx) = tokio::sync::oneshot::channel::<()>();
    let mut reader = TelnetReader {
        stream: Box::new(Cursor::new(source)),
        wire: wire_with_sink(sink),
        parser: IacParser::new(),
        decoder: Some(encoding_rs::GBK.new_decoder()),
        clean: Vec::new(),
        pending: Vec::new(),
        offset: 0,
        decoded: String::new(),
        closed,
        exit: Some(exit_tx),
    };
    let output = drain_reader(&mut reader).expect("读取不应失败");
    assert_eq!(String::from_utf8_lossy(&output), "登录");
    let written = shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    assert_eq!(written, vec![IAC, WILL, OPT_TTYPE]);
}

struct ByteAtATime {
    data: Vec<u8>,
    offset: usize,
}

impl Read for ByteAtATime {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.offset >= self.data.len() {
            return Ok(0);
        }
        buf[0] = self.data[self.offset];
        self.offset += 1;
        Ok(1)
    }
}

#[test]
fn gbk_sequence_split_across_reads_is_rejoined() {
    let (sink, _shared) = SharedSink::new();
    let (exit_tx, _exit_rx) = tokio::sync::oneshot::channel::<()>();
    let mut reader = TelnetReader {
        stream: Box::new(ByteAtATime {
            data: vec![0xb5, 0xc7, 0xc2, 0xbc],
            offset: 0,
        }),
        wire: wire_with_sink(sink),
        parser: IacParser::new(),
        decoder: Some(encoding_rs::GBK.new_decoder()),
        clean: Vec::new(),
        pending: Vec::new(),
        offset: 0,
        decoded: String::new(),
        closed: Arc::new(AtomicBool::new(false)),
        exit: Some(exit_tx),
    };
    let output = drain_reader(&mut reader).expect("读取不应失败");
    assert_eq!(String::from_utf8_lossy(&output), "登录");
}

#[test]
fn writer_encodes_gbk() {
    let (sink, shared) = SharedSink::new();
    let mut writer = TelnetWriter {
        wire: wire_with_sink(sink),
        encoder: Some(encoding_rs::GBK.new_encoder()),
        encoded: Vec::new(),
        escaped: Vec::new(),
    };
    writer.write_all("密码".as_bytes()).expect("写入不应失败");
    writer
        .write_all("登录".as_bytes())
        .expect("连续写入不应失败");
    let written = shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    assert_eq!(
        written,
        vec![0xc3, 0xdc, 0xc2, 0xeb, 0xb5, 0xc7, 0xc2, 0xbc]
    );
}

#[test]
fn writer_rejects_non_utf8_input() {
    let (sink, _shared) = SharedSink::new();
    let mut writer = TelnetWriter {
        wire: wire_with_sink(sink),
        encoder: Some(encoding_rs::GBK.new_encoder()),
        encoded: Vec::new(),
        escaped: Vec::new(),
    };
    let error = writer.write(&[0xff, 0xfe]).expect_err("非法 UTF-8 应报错");
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn utf8_passthrough_only_escapes() {
    let (sink, shared) = SharedSink::new();
    let mut writer = TelnetWriter {
        wire: wire_with_sink(sink),
        encoder: None,
        encoded: Vec::new(),
        escaped: Vec::new(),
    };
    writer.write_all(b"ok\xff").expect("写入不应失败");
    let written = shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    assert_eq!(written, vec![b'o', b'k', IAC, IAC]);
}

#[test]
fn encoding_for_maps_all_variants() {
    assert!(encoding_for(TextEncoding::Utf8).is_none());
    assert_eq!(encoding_for(TextEncoding::Gbk), Some(encoding_rs::GBK));
    assert_eq!(encoding_for(TextEncoding::Big5), Some(encoding_rs::BIG5));
}
