    use super::*;

    fn hex_zrinit() -> RecvAction {
        RecvAction::Write(build_hex_header(ZRINIT, [0, 0, 0, receiver_flags()]))
    }

    /// 构造零位置/零标志的帧头，供相位流转测试复用。
    fn head(kind: u8) -> FrameHeader {
        FrameHeader {
            kind,
            body: [0; 4],
            crc32_mode: false,
        }
    }

    /// ZSKIP 之后批次必须继续：sz 收到 ZSKIP 随即发送下一个文件的 ZFILE，
    /// 相位若停留在"等数据帧头"，整批剩余文件都会被静默丢弃。
    #[test]
    fn create_failed_still_accepts_next_file_in_batch() {
        let mut rx = Receiver::new();
        rx.on_header(head(ZFILE));
        rx.on_subpacket(b"a.bin 4 0 644 1 4".to_vec(), ZCRCW);
        let actions = rx.notify_create_failed();
        assert_eq!(
            actions,
            vec![RecvAction::Write(build_hex_header(ZSKIP, [0, 0, 0, 0]))]
        );
        assert!(rx.on_header(head(ZFILE)).is_empty());
        assert_eq!(rx.phase(), RecvPhase::Subpacket, "应转入文件信息子包相位");
        let actions = rx.on_subpacket(b"b.bin 4 0 644 1 4".to_vec(), ZCRCW);
        assert_eq!(actions[0], RecvAction::CreateFile("b.bin".to_string()));
    }

    /// 数据相位的坏子包重同步：ZRPOS 之后对端重发的是 ZDATA 帧头，相位必须退回
    /// 帧头解析，否则帧头字节会被当成文件数据写入，直接损坏落盘内容。
    #[test]
    fn bad_subpacket_in_data_falls_back_to_header_phase() {
        let mut rx = Receiver::new();
        rx.on_header(head(ZFILE));
        rx.on_subpacket(b"c.bin 4 0 644 1 4".to_vec(), ZCRCW);
        rx.on_header(head(ZDATA));
        assert_eq!(rx.phase(), RecvPhase::Subpacket);
        let actions = rx.on_bad_subpacket();
        assert!(
            matches!(actions.as_slice(), [RecvAction::Write(_)]),
            "重同步应只发出 Attn+ZRPOS 一次写出"
        );
        assert_eq!(rx.phase(), RecvPhase::Head, "重同步后应回到帧头相位");
    }

    #[test]
    fn zsinit_is_answered_with_zack_one() {
        let mut rx = Receiver::new();
        assert_eq!(rx.start(), vec![hex_zrinit()]);
        assert!(
            rx.on_header(FrameHeader {
                kind: ZSINIT,
                body: [0; 4],
                crc32_mode: false
            })
            .is_empty()
        );
        let actions = rx.on_subpacket(b"\x18\x18".to_vec(), ZCRCW);
        assert_eq!(
            actions,
            vec![RecvAction::Write(build_hex_header(ZACK, [1, 0, 0, 0]))]
        );
    }

    #[test]
    fn file_info_creates_file_and_answers_zrpos() {
        let mut rx = Receiver::new();
        assert!(
            rx.on_header(FrameHeader {
                kind: ZFILE,
                body: [0; 4],
                crc32_mode: false
            })
            .is_empty()
        );
        let info = b"report.txt 1024 1700000000 0100644 1 1024".to_vec();
        let actions = rx.on_subpacket(info, ZCRCW);
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0], RecvAction::CreateFile("report.txt".to_string()));
        assert!(matches!(actions[1], RecvAction::Write(_)));
    }

    /// 真机 `sz` 抓包格式：名字段以 NUL 结束，数字字段之间才用空格。
    #[test]
    fn file_info_splits_at_nul_like_lrzsz() {
        let (name, fields) = split_file_info(b"a.bin\x003000 15267304770 100664 0 1 3000\x00");
        assert_eq!(name, b"a.bin", "名字必须止于 NUL，不能粘上大小字段");
        assert_eq!(fields.len(), 6, "大小/时间/模式/类型/文件数/总字节");
        assert_eq!(fields[0], "3000");
    }

    /// 用真机字节流走一遍：文件名与声明大小都要正确。
    #[test]
    fn real_lrzsz_file_info_creates_named_file_with_size() {
        let mut rx = Receiver::new();
        rx.on_header(head(ZFILE));
        let actions = rx.on_subpacket(
            b"payload.bin\x009000 15267304770 100664 0 1 9000\x00".to_vec(),
            ZCRCW,
        );
        assert_eq!(
            actions[0],
            RecvAction::CreateFile("payload.bin".to_string()),
            "实际动作: {actions:?}"
        );
        assert_eq!(rx.total, Some(9000));
    }

    #[test]
    fn data_subpackets_stream_and_ack_rules() {
        let mut rx = Receiver::new();
        // lrzsz 时序：sz 先发 ZSINIT（Attn），再发 ZFILE（文件信息）。
        assert!(
            rx.on_header(FrameHeader {
                kind: ZSINIT,
                body: [0; 4],
                crc32_mode: false
            })
            .is_empty()
        );
        let actions = rx.on_subpacket(b"ATTN".to_vec(), ZCRCW);
        assert_eq!(
            actions,
            vec![RecvAction::Write(build_hex_header(ZACK, [1, 0, 0, 0]))]
        );
        assert!(
            rx.on_header(FrameHeader {
                kind: ZFILE,
                body: [0; 4],
                crc32_mode: false
            })
            .is_empty()
        );
        rx.on_subpacket(b"f.bin 0".to_vec(), ZCRCW);
        assert!(
            rx.on_header(FrameHeader {
                kind: ZDATA,
                body: [0; 4],
                crc32_mode: false
            })
            .is_empty()
        );

        // ZCRCG：仅写文件与进度。
        let actions = rx.on_subpacket(vec![1; 100], ZCRCG);
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0], RecvAction::WriteFile(vec![1; 100]));

        // ZCRCQ：追加 ZACK(偏移)。
        let actions = rx.on_subpacket(vec![2; 50], ZCRCQ);
        assert_eq!(actions.len(), 3);
        assert_eq!(
            actions[2],
            RecvAction::Write(build_hex_header(ZACK, 150u32.to_le_bytes()))
        );

        // ZCRCW：追加 ZACK|0x80 并回到等头状态。
        let actions = rx.on_subpacket(vec![3; 10], ZCRCW);
        assert_eq!(
            actions[2],
            RecvAction::Write(build_hex_header(ZACK | 0x80, 160u32.to_le_bytes()))
        );
        assert!(matches!(rx.state, RecvState::WaitDataHeader));

        // ZEOF 偏移一致：关文件 + ZRINIT 进入下一文件。
        let actions = rx.on_header(FrameHeader {
            kind: ZEOF,
            body: 160u32.to_le_bytes(),
            crc32_mode: false,
        });
        assert_eq!(actions[0], RecvAction::CloseFile { size: 160 });
        assert_eq!(actions[1], hex_zrinit());
        assert_eq!(rx.files_done, 1);
    }

    #[test]
    fn zdata_offset_mismatch_resyncs_with_attn() {
        let mut rx = Receiver::new();
        // Attn 必须先于 ZFILE 到达（ZSINIT 在时序上最先）。
        assert!(
            rx.on_header(FrameHeader {
                kind: ZSINIT,
                body: [0; 4],
                crc32_mode: false
            })
            .is_empty()
        );
        rx.on_subpacket(b"ATTN".to_vec(), ZCRCW);
        rx.on_header(FrameHeader {
            kind: ZFILE,
            body: [0; 4],
            crc32_mode: false,
        });
        rx.on_subpacket(b"f.bin 0".to_vec(), ZCRCW);
        let actions = rx.on_header(FrameHeader {
            kind: ZDATA,
            body: [9, 0, 0, 0],
            crc32_mode: false,
        });
        assert_eq!(actions.len(), 1);
        let RecvAction::Write(wire) = &actions[0] else {
            panic!("应为线路写入");
        };
        assert!(wire.starts_with(b"ATTN"), "重同步必须先发 Attn");
        assert!(wire.ends_with(&build_hex_header(ZRPOS, 0u32.to_le_bytes())));
    }

    #[test]
    fn zfin_exchanges_and_swallows_oo() {
        let mut rx = Receiver::new();
        rx.on_header(FrameHeader {
            kind: ZFILE,
            body: [0; 4],
            crc32_mode: false,
        });
        rx.on_subpacket(b"f 0".to_vec(), ZCRCW);
        let actions = rx.on_header(FrameHeader {
            kind: ZFIN,
            body: [0; 4],
            crc32_mode: false,
        });
        assert_eq!(
            actions,
            vec![RecvAction::Write(build_hex_header(ZFIN, [0, 0, 0, 0]))]
        );
        assert!(rx.on_raw(b'O').is_empty());
        let actions = rx.on_raw(b'O');
        assert_eq!(
            actions,
            vec![RecvAction::Finished(ZmodemFinished::Completed { files: 0 })]
        );
    }

    #[test]
    fn create_failed_requests_skip() {
        let mut rx = Receiver::new();
        rx.on_header(FrameHeader {
            kind: ZFILE,
            body: [0; 4],
            crc32_mode: false,
        });
        rx.on_subpacket(b"locked.db 5".to_vec(), ZCRCW);
        let actions = rx.notify_create_failed();
        assert_eq!(
            actions,
            vec![RecvAction::Write(build_hex_header(ZSKIP, [0, 0, 0, 0]))]
        );
        // 后续 ZEOF(0) 不应再关闭文件。
        assert!(
            rx.on_header(FrameHeader {
                kind: ZEOF,
                body: [0; 4],
                crc32_mode: false
            })
            .is_empty()
        );
        assert_eq!(rx.files_done, 0);
    }

    #[test]
    fn cancel_writes_can_sequence() {
        let mut rx = Receiver::new();
        let actions = rx.cancel();
        assert_eq!(actions.len(), 2);
        assert!(matches!(actions[0], RecvAction::Write(_)));
        assert_eq!(actions[1], RecvAction::Finished(ZmodemFinished::Cancelled));
    }

    #[test]
    fn file_names_are_sanitized() {
        assert_eq!(sanitize_file_name("../etc/passwd"), "passwd");
        assert_eq!(sanitize_file_name("C:\\tmp\\a.bin"), "a.bin");
        assert_eq!(sanitize_file_name("  "), "unnamed.bin");
        assert_eq!(sanitize_file_name("ok.bin"), "ok.bin");
        assert_eq!(sanitize_file_name(".."), "unnamed.bin");
        assert_eq!(sanitize_file_name("./.."), "unnamed.bin");
        assert_eq!(sanitize_file_name("..\\..\\etc"), "etc");
    }
