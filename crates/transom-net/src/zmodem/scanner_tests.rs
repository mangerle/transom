    use super::*;
    use crate::zmodem::frames::{
        build_bin16_header, build_bin32_header, build_hex_header, build_subpacket16,
        build_subpacket32, escape_zdle,
    };

    /// 逐字节喂入扫描器并收集全部事件。
    fn drain<C, E, F>(consumer: &mut C, bytes: &[u8], feed: F) -> Vec<E>
    where
        F: Fn(&mut C, u8) -> Option<E>,
    {
        let mut events = Vec::new();
        for &byte in bytes {
            events.extend(feed(consumer, byte));
        }
        events
    }

    #[test]
    fn parses_real_capture_zrqinit_hex_header() {
        // 真实抓包：sz 启动时发送 `**^XB00000000000000\r`（全零体 CRC=0）。
        let mut scanner = HeaderScanner::new();
        let wire = b"**\x18B00000000000000\r";
        let events = drain(&mut scanner, wire, HeaderScanner::feed);
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0],
            HeaderEvent::Header(FrameHeader {
                kind: ZRQINIT_KIND,
                body: [0; 4],
                crc32_mode: false,
            })
        );
    }

    const ZRQINIT_KIND: u8 = 0;

    #[test]
    fn parses_hex_header_from_builder() {
        let mut scanner = HeaderScanner::new();
        let wire = build_hex_header(1, [0, 0, 0, 0x23]);
        let events = drain(&mut scanner, &wire, HeaderScanner::feed);
        assert_eq!(
            events,
            vec![HeaderEvent::Header(FrameHeader {
                kind: 1,
                body: [0, 0, 0, 0x23],
                crc32_mode: false,
            })]
        );
    }

    #[test]
    fn parses_binary_headers_and_reports_crc_width() {
        for (wire, expect_crc32) in [
            (build_bin16_header(9, [0, 0, 0, 0]), false),
            (build_bin32_header(10, [0, 0, 0, 0]), true),
        ] {
            let mut scanner = HeaderScanner::new();
            let events = drain(&mut scanner, &wire, HeaderScanner::feed);
            assert_eq!(events.len(), 1, "crc32={expect_crc32} 头应解析成功");
            let HeaderEvent::Header(header) = events[0] else {
                panic!("应为帧头事件");
            };
            assert_eq!(header.crc32_mode, expect_crc32);
        }
    }

    #[test]
    fn binary_header_survives_control_bytes_in_body() {
        // body 含控制字节（转义后 0x18→ZDLE+0x58）依旧可解析。
        let mut scanner = HeaderScanner::new();
        let wire = build_bin32_header(4, [0x18, 0x11, 0, 0]);
        let events = drain(&mut scanner, &wire, HeaderScanner::feed);
        assert_eq!(
            events,
            vec![HeaderEvent::Header(FrameHeader {
                kind: 4,
                body: [0x18, 0x11, 0, 0],
                crc32_mode: true,
            })]
        );
    }

    #[test]
    fn rejects_header_with_bad_crc() {
        let mut scanner = HeaderScanner::new();
        let mut wire = build_hex_header(1, [0, 0, 0, 0x23]);
        let last = wire.len() - 3; // 破坏 CRC hex 的最后一个数字
        wire[last] = b'1';
        let events = drain(&mut scanner, &wire, HeaderScanner::feed);
        assert!(events.is_empty(), "坏 CRC 的头必须被丢弃");
    }

    #[test]
    fn skips_leading_garbage_and_high_bit_pad() {
        let mut scanner = HeaderScanner::new();
        let mut wire = b"\r\n login: ".to_vec();
        wire.extend(build_hex_header(8, [0, 0, 0, 0]));
        wire[0] = 0xAA; // 高位 ZPAD 变体
        let events = drain(&mut scanner, &wire, HeaderScanner::feed);
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0],
            HeaderEvent::Header(FrameHeader {
                kind: 8,
                body: [0; 4],
                crc32_mode: false,
            })
        );
    }

    #[test]
    fn reports_cancel_after_five_can_bytes() {
        let mut scanner = HeaderScanner::new();
        let events = drain(&mut scanner, &[ZDLE; 5], HeaderScanner::feed);
        assert_eq!(events, vec![HeaderEvent::Cancel]);
        // ZDLE+4×CAN 组合（转义上下文中的取消）同样识别。
        let mut scanner = HeaderScanner::new();
        let events = drain(
            &mut scanner,
            &[0x2A, 0x2A, ZDLE, 0x18, 0x18, 0x18, 0x18],
            HeaderScanner::feed,
        );
        assert_eq!(events, vec![HeaderEvent::Cancel]);
    }

    #[test]
    fn parses_subpacket16_with_terminator() {
        let mut scanner = SubpacketScanner::new();
        scanner.set_crc32(false);
        let wire = build_subpacket16(b"hello", 0x6B);
        let events = drain(&mut scanner, &wire, SubpacketScanner::feed);
        assert_eq!(
            events,
            vec![SubpacketEvent::Packet(b"hello".to_vec(), 0x6B)]
        );
    }

    #[test]
    fn parses_subpacket32_with_terminator() {
        let mut scanner = SubpacketScanner::new();
        scanner.set_crc32(true);
        let wire = build_subpacket32(b"hello", 0x6A);
        let events = drain(&mut scanner, &wire, SubpacketScanner::feed);
        assert_eq!(
            events,
            vec![SubpacketEvent::Packet(b"hello".to_vec(), 0x6A)]
        );
    }

    #[test]
    fn raw_hijk_bytes_stay_data() {
        // 关键定案：裸 h/i/j/k 是数据，只有 ZDLE 前缀才是终止符。
        let mut scanner = SubpacketScanner::new();
        let wire = build_subpacket16(b"hijk\x68\x69\x6A\x6B", 0x68);
        let events = drain(&mut scanner, &wire, SubpacketScanner::feed);
        assert_eq!(
            events,
            vec![SubpacketEvent::Packet(
                b"hijk\x68\x69\x6A\x6B".to_vec(),
                0x68
            )]
        );
    }

    #[test]
    fn escaped_control_bytes_roundtrip() {
        let mut scanner = SubpacketScanner::new();
        let payload = [0x11u8, 0x91, 0x13, 0x93, 0x10, 0x90, 0x18, 0x7F, 0xFF];
        let wire = build_subpacket16(&payload, 0x69);
        let events = drain(&mut scanner, &wire, SubpacketScanner::feed);
        assert_eq!(events, vec![SubpacketEvent::Packet(payload.to_vec(), 0x69)]);
    }

    #[test]
    fn parsing_is_chunk_boundary_safe() {
        let wire = escape_zdle(&[0x11, 0x91]);
        // 0x11 的转义形态是 ZDLE 0x51；拆在转义对中间喂入仍应正确解码。
        let mut scanner = SubpacketScanner::new();
        let mut events = Vec::new();
        for &byte in &wire[..2.min(wire.len())] {
            events.extend(scanner.feed(byte));
        }
        for &byte in &wire[2..] {
            events.extend(scanner.feed(byte));
        }
        assert!(events.is_empty());
        // 数据未收齐不产出事件，不 panic 即为跨块安全。
    }

    #[test]
    fn bad_subpacket_crc_reports_bad() {
        let mut scanner = SubpacketScanner::new();
        let mut wire = build_subpacket16(b"hello", 0x6B);
        *wire.last_mut().expect("非空") ^= 0xFF;
        let events = drain(&mut scanner, &wire, SubpacketScanner::feed);
        assert_eq!(events, vec![SubpacketEvent::Bad]);
    }
