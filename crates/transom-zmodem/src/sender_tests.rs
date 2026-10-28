    use super::*;
    use crate::frames::{CANFDX, ZDLE};

    fn remote(cap: u8) -> FrameHeader {
        FrameHeader {
            kind: ZRINIT,
            body: [0, 0, 0, cap],
            crc32_mode: false,
        }
    }

    /// 指定续传偏移的 ZRPOS 帧头（位置字段低位在前）。
    fn rpos(offset: u32) -> FrameHeader {
        FrameHeader {
            kind: ZRPOS,
            body: offset.to_le_bytes(),
            crc32_mode: false,
        }
    }

    /// 指定帧类型的空位置帧头。
    fn plain(kind: u8) -> FrameHeader {
        FrameHeader {
            kind,
            body: [0; 4],
            crc32_mode: false,
        }
    }

    fn ready(tx: &mut Sender) -> Vec<SendAction> {
        tx.set_current_file("report.txt".to_string());
        tx.notify_file_ready(2200, Some(1_700_000_000))
    }

    #[test]
    fn full_single_file_upload_flow() {
        let mut tx = Sender::new(CANFDX | CANOVIO | CANFC32);
        let actions = tx.begin(vec![PathBuf::from("/tmp/report.txt")]);
        assert_eq!(
            actions,
            vec![SendAction::OpenFile(PathBuf::from("/tmp/report.txt"))]
        );
        let actions = ready(&mut tx);
        assert_eq!(actions.len(), 2);
        assert_eq!(
            actions[0],
            SendAction::Write(build_hex_header(ZFILE, [0, 0, 0, 0]))
        );
        // 信息子包按 lrzsz 布局：`名字\0` 开头，随后十进制大小与八进制时间，
        // 模式固定 100644、类型 0、批次 1 个文件（CRC 字节非文本，只在 ASCII
        // 前缀上断言）。
        let SendAction::Write(info) = &actions[1] else {
            panic!("应为子包写入");
        };
        assert!(
            info.starts_with(b"report.txt\x002200 "),
            "信息子包内容: {info:?}"
        );
        let tail = b" 100644 0 1 2200\x00";
        assert!(
            info.windows(tail.len()).any(|w| w == tail),
            "信息子包内容: {info:?}"
        );

        // ZRPOS(0) 触发读块。
        let actions = tx.on_header(FrameHeader {
            kind: ZRPOS,
            body: [0; 4],
            crc32_mode: false,
        });
        assert_eq!(actions, vec![SendAction::ReadFile { offset: 0 }]);

        // 非末块：ZDATA 头（bin32）+ ZCRCG 子包 + 进度 + 继续读。
        let actions = tx.push_data(&[0xAB; BLOCK_SIZE], false);
        assert_eq!(actions.len(), 4);
        let SendAction::Write(data_head) = &actions[0] else {
            panic!("应为 ZDATA 头");
        };
        assert_eq!(
            &data_head[..4],
            &[
                super::super::frames::ZPAD,
                super::super::frames::ZPAD,
                super::super::frames::ZDLE,
                super::super::frames::ZBIN32
            ]
        );
        assert!(matches!(
            actions[2],
            SendAction::Progress {
                transferred: 1024,
                total: 2200,
                ..
            }
        ));
        assert_eq!(actions[3], SendAction::ReadFile { offset: 1024 });

        // 第二块（非末）：不再重复 ZDATA 头。
        let actions = tx.push_data(&[0xCD; BLOCK_SIZE], false);
        assert_eq!(actions.len(), 3, "ZDATA 头只在块序列开始时发一次");
        assert_eq!(actions[2], SendAction::ReadFile { offset: 2048 });

        // 末块（152 字节，凑足 2200）：ZCRCE 子包 + ZEOF(文件大小)。
        let actions = tx.push_data(&[0xCD; 152], true);
        assert_eq!(actions.len(), 3);
        assert_eq!(
            actions[2],
            SendAction::Write(build_header_of(true, ZEOF, 2200))
        );

        // ZRINIT 确认 EOF，队列空 → ZFIN。
        let actions = tx.on_header(remote(CANFDX | CANOVIO | CANFC32));
        assert_eq!(
            actions,
            vec![SendAction::Write(build_hex_header(ZFIN, [0, 0, 0, 0]))]
        );

        // 对端 ZFIN → "OO" + 完成。
        let actions = tx.on_header(FrameHeader {
            kind: ZFIN,
            body: [0; 4],
            crc32_mode: false,
        });
        assert_eq!(
            actions,
            vec![
                SendAction::Write(b"OO".to_vec()),
                SendAction::Finished(ZmodemFinished::Completed { files: 1 }),
            ]
        );
    }

    #[test]
    fn zskip_advances_to_next_file() {
        let mut tx = Sender::new(CANFDX | CANOVIO);
        let actions = tx.begin(vec![PathBuf::from("/tmp/a"), PathBuf::from("/tmp/b")]);
        assert_eq!(actions.len(), 1);
        // 打开 a 成功，随后 rz 拒绝（ZSKIP）→ 打开 b。
        tx.set_current_file("a".to_string());
        assert_eq!(tx.notify_file_ready(10, None).len(), 2);
        let actions = tx.on_header(FrameHeader {
            kind: ZSKIP,
            body: [0; 4],
            crc32_mode: false,
        });
        assert_eq!(actions, vec![SendAction::OpenFile(PathBuf::from("/tmp/b"))]);
    }

    #[test]
    fn zrinit_during_wait_rpos_resends_file_info() {
        let mut tx = Sender::new(CANFDX | CANOVIO);
        tx.begin(vec![PathBuf::from("/tmp/a")]);
        tx.set_current_file("a".to_string());
        tx.notify_file_ready(10, None);
        let actions = tx.on_header(remote(CANFDX | CANOVIO));
        assert_eq!(actions.len(), 2, "ZRINIT 轮询应触发 ZFILE 重发");
    }

    /// 数据阶段收到 ZNAK 必须转入恢复：停留原状态会吞掉对端随后的续传请求，
    /// 上传只能挂到超时才结束。
    #[test]
    fn znak_during_sending_reoffers_and_waits_for_rpos() {
        let mut tx = Sender::new(CANFDX | CANOVIO);
        tx.begin(vec![PathBuf::from("/tmp/a")]);
        ready(&mut tx);
        tx.on_header(rpos(0));
        assert!(matches!(tx.state, SendState::Sending));
        let actions = tx.on_header(plain(ZNAK));
        assert!(
            matches!(tx.state, SendState::WaitRpos),
            "ZNAK 后应退回等待续传点"
        );
        assert_eq!(actions.len(), 2, "应以 ZFILE + 文件信息重新 offer");
    }

    /// 数据阶段的 ZRPOS 是按新偏移续传的唯一入口，必须受理。
    #[test]
    fn zrpos_during_sending_resumes_at_requested_offset() {
        let mut tx = Sender::new(CANFDX | CANOVIO);
        tx.begin(vec![PathBuf::from("/tmp/a")]);
        ready(&mut tx);
        tx.on_header(rpos(0));
        let actions = tx.on_header(rpos(2048));
        assert_eq!(actions, vec![SendAction::ReadFile { offset: 2048 }]);
        assert!(matches!(tx.state, SendState::Sending));
    }

    #[test]
    fn zero_size_file_directly_eofs() {
        let mut tx = Sender::new(CANFDX | CANOVIO);
        tx.begin(vec![PathBuf::from("/tmp/empty")]);
        tx.set_current_file("empty".to_string());
        tx.notify_file_ready(0, None);
        let actions = tx.on_header(FrameHeader {
            kind: ZRPOS,
            body: [0; 4],
            crc32_mode: false,
        });
        assert_eq!(
            actions,
            vec![SendAction::Write(build_header_of(false, ZEOF, 0))]
        );
    }

    #[test]
    fn no_canovio_forces_zcrcw_sync() {
        let mut tx = Sender::new(CANFDX);
        tx.begin(vec![PathBuf::from("/tmp/a")]);
        tx.set_current_file("a".to_string());
        tx.notify_file_ready(2048, None);
        tx.on_header(FrameHeader {
            kind: ZRPOS,
            body: [0; 4],
            crc32_mode: false,
        });
        let actions = tx.push_data(&[1; BLOCK_SIZE], false);
        // 无 CANOVIO：ZCRCW 子包且不自动继续读，等 ZACK。
        let SendAction::Write(packet) = &actions[1] else {
            panic!("应为子包写入");
        };
        assert!(packet.windows(2).any(|w| w == [ZDLE, ZCRCW]));
        assert_eq!(actions.len(), 3, "同步模式不产生 ReadFile");
        let actions = tx.on_ack();
        assert_eq!(actions, vec![SendAction::ReadFile { offset: 1024 }]);
    }

    #[test]
    fn cancel_writes_can_sequence() {
        let mut tx = Sender::new(CANFDX);
        tx.begin(vec![PathBuf::from("/tmp/a")]);
        let actions = tx.cancel();
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[1], SendAction::Finished(ZmodemFinished::Cancelled));
    }

    #[test]
    fn oversize_file_is_skipped() {
        let mut tx = Sender::new(CANFDX | CANOVIO);
        tx.begin(vec![PathBuf::from("/tmp/huge"), PathBuf::from("/tmp/ok")]);
        tx.set_current_file("huge".to_string());
        let actions = tx.notify_file_ready(u64::from(u32::MAX) + 1, None);
        // 先 ZSKIP，再打开下一个文件。
        assert_eq!(
            actions[0],
            SendAction::Write(build_hex_header(ZSKIP, [0, 0, 0, 0]))
        );
        assert_eq!(actions[1], SendAction::OpenFile(PathBuf::from("/tmp/ok")));
    }

    #[test]
    fn timeout_resends_file_info_and_gives_up() {
        let mut tx = Sender::new(CANFDX | CANOVIO);
        tx.begin(vec![PathBuf::from("/tmp/a")]);
        tx.set_current_file("a".to_string());
        tx.notify_file_ready(10, None);
        for _ in 0..5 {
            assert_eq!(tx.on_timeout().len(), 2, "前五次超时重发 ZFILE");
        }
        let actions = tx.on_timeout();
        assert_eq!(
            actions[1],
            SendAction::Finished(ZmodemFinished::Failed("对端长时间无响应".to_string()))
        );
    }

    /// ZACK 丢失时必须原样重发同一帧（含首块 ZDATA 头），且等待窗口指数退避。
    #[test]
    fn ack_timeout_retransmits_last_frame_with_exponential_backoff() {
        let mut tx = Sender::new(CANFDX);
        tx.begin(vec![PathBuf::from("/tmp/a")]);
        ready(&mut tx);
        tx.on_header(rpos(0));
        let actions = tx.push_data(&[0x5A; BLOCK_SIZE], false);
        let SendAction::Write(header) = &actions[0] else {
            panic!("首块应带 ZDATA 头");
        };
        let SendAction::Write(packet) = &actions[1] else {
            panic!("应写入子包");
        };
        let mut expected = header.clone();
        expected.extend_from_slice(packet);

        let delay = tx.ack_timeout().expect("等待 ZACK 应有退避窗口");
        assert!(delay >= Duration::from_secs(2));
        let retry = tx.on_timeout();
        assert_eq!(retry, vec![SendAction::Write(expected.clone())]);
        let next = tx.ack_timeout().expect("仍在等待 ZACK");
        assert!(
            next > delay,
            "重传后等待窗口必须变长: {next:?} <= {delay:?}"
        );

        // 其余四次重试仍重发同一帧，第六次超时判定失败。
        for _ in 0..4 {
            assert_eq!(tx.on_timeout(), vec![SendAction::Write(expected.clone())]);
        }
        let actions = tx.on_timeout();
        assert!(matches!(
            actions.last(),
            Some(SendAction::Finished(ZmodemFinished::Failed(_)))
        ));
    }

    /// 收到合法帧即说明对端在线，退避计数必须归零。
    #[test]
    fn receiving_any_header_resets_ack_backoff() {
        let mut tx = Sender::new(CANFDX);
        tx.begin(vec![PathBuf::from("/tmp/a")]);
        ready(&mut tx);
        tx.on_header(rpos(0));
        tx.push_data(&[1; BLOCK_SIZE], false);
        tx.on_timeout();
        assert!(tx.ack_timeout().expect("等待窗口") > Duration::from_secs(2));
        tx.on_header(plain(ZACK));
        assert_eq!(tx.ack_timeout(), None, "确认后离开 ZACK 窗口");
    }
