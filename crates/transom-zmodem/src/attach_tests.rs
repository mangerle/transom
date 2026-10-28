    use super::*;
    use crate::frames::{
        ZCRCE, ZCRCG, ZCRCW, ZDLE, ZEOF, ZFILE, ZFIN, ZRINIT, ZRPOS, ZSINIT, build_bin16_header,
        build_bin32_header, build_hex_header, build_subpacket16, build_subpacket32, receiver_flags,
    };
    use crate::fsm::Mode;
    use std::io::Cursor;
    use transom_core::ZmodemDirection;

    /// 测试事件通道容量：进度按子包粒度产出，64 足以平滑突发。
    const EVENT_CAPACITY: usize = 64;

    /// 写线路测试桩：收集全部字节。
    struct SharedSink(Arc<Mutex<Vec<u8>>>);

    impl Write for SharedSink {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().expect("测试桩不中毒").extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// 按预设分块返回数据的底层读取桩（跨块标记测试用）。
    struct StepReader {
        chunks: Vec<Vec<u8>>,
        index: usize,
    }

    impl Read for StepReader {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let Some(chunk) = self.chunks.get(self.index) else {
                return Ok(0);
            };
            self.index += 1;
            let take = chunk.len().min(buf.len());
            buf[..take].copy_from_slice(&chunk[..take]);
            Ok(take)
        }
    }

    struct Harness {
        reader: ZmodemReader,
        fsm: Fsm,
        written: Arc<Mutex<Vec<u8>>>,
        events: flume::Receiver<ZmodemEvent>,
        ctrl_tx: flume::Sender<ZmodemCommand>,
        replay_tx: flume::Sender<Vec<u8>>,
    }

    fn harness(download_dir: PathBuf, auto_receive: bool) -> Harness {
        let shared = Arc::new(Shared {
            active: AtomicBool::new(false),
            terminated: AtomicBool::new(false),
        });
        let written = Arc::new(Mutex::new(Vec::new()));
        let writer: Arc<Mutex<Box<dyn Write + Send>>> =
            Arc::new(Mutex::new(Box::new(SharedSink(Arc::clone(&written)))));
        let (raw_tx, raw_rx) = flume::bounded::<Vec<u8>>(RAW_CAPACITY);
        let (replay_tx, replay_rx) = flume::bounded::<Vec<u8>>(REPLAY_CAPACITY);
        let (ctrl_tx, ctrl_rx) = flume::bounded::<ZmodemCommand>(COMMAND_CAPACITY);
        let (event_tx, event_rx) = flume::bounded::<ZmodemEvent>(EVENT_CAPACITY);
        let reader = ZmodemReader::new(
            Box::new(Cursor::new(Vec::<u8>::new())),
            Arc::clone(&shared),
            raw_tx,
            replay_rx,
        );
        let fsm = Fsm::new(
            FsmChannels {
                writer: Arc::clone(&writer),
                events: event_tx,
                replay_tx: replay_tx.clone(),
                raw_rx,
                ctrl_rx,
                shared: Arc::clone(&shared),
            },
            ZmodemConfig {
                download_dir,
                auto_receive,
            },
        );
        Harness {
            reader,
            fsm,
            written,
            events: event_rx,
            ctrl_tx,
            replay_tx,
        }
    }

    impl Harness {
        fn ctrl(&mut self, command: ZmodemCommand) -> bool {
            if self.ctrl_tx.try_send(command).is_ok() {
                if let Ok(command) = self.fsm.ctrl_rx.try_recv() {
                    self.fsm.handle_command(command);
                }
                return true;
            }
            false
        }

        fn wire(&mut self) -> Vec<u8> {
            std::mem::take(&mut *self.written.lock().expect("测试桩不中毒"))
        }

        fn collect_events(&mut self) -> Vec<ZmodemEvent> {
            std::iter::from_fn(|| self.events.try_recv().ok()).collect()
        }
    }

    #[test]
    fn reader_forwards_plain_data_until_mark() {
        let mut harness = harness(PathBuf::from("/tmp"), true);
        harness.reader.inner = Box::new(Cursor::new(b"hello world".to_vec()));
        let mut out = [0u8; 64];
        let count = harness.reader.read(&mut out).expect("读取成功");
        assert_eq!(&out[..count], b"hello world");
        assert!(!harness.reader.shared.active.load(Ordering::Acquire));
    }

    /// 会话结束后驱动线程回注的残留字节必须先于底层数据交付终端，
    /// 否则被协议层吞掉的最后一段输出会永久丢失（终端显示缺字）。
    #[test]
    fn replayed_tail_precedes_inner_reader() {
        let mut harness = harness(PathBuf::from("/tmp"), true);
        harness
            .replay_tx
            .try_send(b"shell$ ".to_vec())
            .expect("回注通道有界可用");
        harness.reader.inner = Box::new(Cursor::new(b"prompt".to_vec()));
        let mut out = [0u8; 32];
        let count = harness.reader.read(&mut out).expect("读取成功");
        assert_eq!(&out[..count], b"shell$ ", "回注字节优先输出");
        let count = harness.reader.read(&mut out).expect("读取成功");
        assert_eq!(&out[..count], b"prompt", "回注排空后底层数据照常透传");
    }

    /// 对端把 ZFIN/"OO" 与后续 shell 提示符挤在同一次 burst 送达时，
    /// 收尾之后的字节必须回到终端通路，不能被协议层当作残余吞掉。
    #[test]
    fn tail_of_finishing_block_returns_to_terminal() {
        let mut harness = harness(PathBuf::from("/tmp"), true);
        harness
            .fsm
            .consume_block(b"**\x18B00000000000000\r".to_vec());
        let _ = harness.wire();
        harness.fsm.mode = Mode::Download;
        let block = [build_bin16_header(ZFIN, [0, 0, 0, 0]), b"OO$ ".to_vec()].concat();
        harness.fsm.consume_block(block);
        assert!(matches!(harness.fsm.mode, Mode::Idle));
        let mut out = [0u8; 32];
        let count = harness.reader.read(&mut out).expect("读取成功");
        assert_eq!(&out[..count], b"$ ", "会话结束后的同块残余字节回注终端");
    }

    #[test]
    fn reader_activates_on_sz_mark_and_hides_protocol_bytes() {
        let mut harness = harness(PathBuf::from("/tmp"), true);
        let payload = b"rz\r\n**\x18B00000000000000\r".to_vec();
        harness.reader.inner = Box::new(Cursor::new(payload));
        let mut out = [0u8; 64];
        let count = harness.reader.read(&mut out).expect("读取成功");
        assert_eq!(&out[..count], b"rz\r\n", "标记前的终端回显照常透传");
        assert!(harness.reader.shared.active.load(Ordering::Acquire));
        // 协议字节已全部进通道，底层 EOF。
        let count = harness.reader.read(&mut out).expect("读取成功");
        assert_eq!(count, 0);
        let block = harness.fsm.raw_rx.recv().expect("标记块已入通道");
        assert!(block.starts_with(b"**\x18B"));
    }

    #[test]
    fn reader_mark_split_across_chunks_is_detected() {
        let full = b"**\x18B0100000023".to_vec();
        let mut harness = harness(PathBuf::from("/tmp"), true);
        harness.reader.inner = Box::new(StepReader {
            chunks: vec![full[..3].to_vec(), full[3..].to_vec()],
            index: 0,
        });
        // 第一块只有标记前半：整体进入尾部缓存，不激活。
        harness.reader.pump().expect("读取成功");
        assert!(!harness.reader.shared.active.load(Ordering::Acquire));
        assert_eq!(harness.reader.tail, b"**\x18");
        // 第二块补齐标记：激活并整体转发。
        harness.reader.pump().expect("读取成功");
        assert!(harness.reader.shared.active.load(Ordering::Acquire));
        let block = harness.fsm.raw_rx.recv().expect("标记块已入通道");
        assert_eq!(block, full);
    }

    #[test]
    fn probe_zrqinit_answers_zrinit() {
        let mut harness = harness(PathBuf::from("/tmp"), true);
        harness
            .fsm
            .consume_block(b"**\x18B00000000000000\r".to_vec());
        let sent = harness.wire();
        assert_eq!(sent, build_hex_header(ZRINIT, [0, 0, 0, receiver_flags()]));
        assert!(matches!(harness.fsm.mode, Mode::Deciding));
    }

    #[test]
    fn probe_zrinit_poll_triggers_await_upload() {
        let mut harness = harness(PathBuf::from("/tmp"), true);
        // lrzsz rz 轮询头：** ^X B 01 00 00 00 23 + CRC(be50)。
        harness
            .fsm
            .consume_block(b"**\x18B0100000023be50\r".to_vec());
        assert!(matches!(harness.fsm.mode, Mode::AwaitUpload));
        assert!(matches!(
            harness.collect_events().first(),
            Some(ZmodemEvent::AwaitUpload)
        ));
    }

    #[test]
    fn full_sz_download_session_lands_file() {
        let dir = std::env::temp_dir().join(format!("transom-zmodem-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("临时目录创建成功");
        let mut harness = harness(dir.clone(), true);
        // 1) sz 的 ZRQINIT → 应答 ZRINIT。
        harness
            .fsm
            .consume_block(b"**\x18B00000000000000\r".to_vec());
        let _ = harness.wire();
        harness.fsm.mode = Mode::Download;
        // 2) ZSINIT 头 + Attn 子包 → 应答 ZACK(1)。
        let zsinit = build_bin16_header(ZSINIT, [0, 0, 0, 0]);
        let attn = build_subpacket16(b"\x18\x18", ZCRCW);
        harness.fsm.consume_block([zsinit, attn].concat());
        let sent = harness.wire();
        assert!(
            sent.ends_with(&build_hex_header(3, [1, 0, 0, 0])),
            "ZSINIT 应答 ZACK 位置 1"
        );
        // 3) ZFILE 头 + 文件信息子包 → 创建文件并应答 ZRPOS(0)。
        let zfile = build_bin16_header(ZFILE, [0, 0, 0, 0]);
        let info = build_subpacket16(b"hello.txt 5 0 644 1 5", ZCRCW);
        harness.fsm.consume_block([zfile, info].concat());
        let sent = harness.wire();
        assert!(sent.ends_with(&build_hex_header(ZRPOS, 0u32.to_le_bytes())));
        // 4) ZDATA（bin32/CRC-32 子包）+ ZEOF → 写盘并应答 ZRINIT。
        let zdata = build_bin32_header(10, [0, 0, 0, 0]);
        let first = build_subpacket32(b"hello", ZCRCG);
        let last = build_subpacket32(b"", ZCRCE);
        let zeof = build_bin32_header(ZEOF, 5u32.to_le_bytes());
        harness
            .fsm
            .consume_block([zdata, first, last, zeof].concat());
        let sent = harness.wire();
        assert!(sent.ends_with(&build_hex_header(ZRINIT, [0, 0, 0, receiver_flags()])));
        // 5) 对端 ZFIN → 应答 ZFIN，随后 "OO" 收尾。
        harness
            .fsm
            .consume_block(build_bin16_header(ZFIN, [0, 0, 0, 0]));
        assert!(
            harness
                .wire()
                .ends_with(&build_hex_header(ZFIN, [0, 0, 0, 0]))
        );
        harness.fsm.consume_block(b"OO".to_vec());
        assert!(matches!(harness.fsm.mode, Mode::Idle));

        let content = std::fs::read(dir.join("hello.txt")).expect("下载文件已落盘");
        assert_eq!(content, b"hello");
        std::fs::remove_file(dir.join("hello.txt")).expect("清理");
        std::fs::remove_dir(&dir).expect("清理");

        let events = harness.collect_events();
        assert!(events.contains(&ZmodemEvent::Started {
            direction: ZmodemDirection::Download
        }));
        assert!(
            events.contains(&ZmodemEvent::Finished(ZmodemFinished::Completed {
                files: 1
            }))
        );
    }

    #[test]
    fn upload_decision_with_missing_file_aborts() {
        let mut harness = harness(PathBuf::from("/tmp"), true);
        harness
            .fsm
            .consume_block(b"**\x18B0100000023be50\r".to_vec());
        let _ = harness.wire();
        assert!(harness.ctrl(ZmodemCommand::Upload(vec![PathBuf::from(
            "/nonexistent.bin"
        )])));
        // 文件打不开且无后续文件：取消序列 + 失败收尾。
        let sent = harness.wire();
        assert!(sent.starts_with(&[ZDLE; 8][..]));
        assert!(matches!(harness.fsm.mode, Mode::Idle,));
        assert!(matches!(
            harness.collect_events().last(),
            Some(ZmodemEvent::Finished(ZmodemFinished::Failed(_)))
        ));
    }

    #[test]
    fn cancel_during_download_writes_can_sequence() {
        let mut harness = harness(PathBuf::from("/tmp"), true);
        harness
            .fsm
            .consume_block(b"**\x18B00000000000000\r".to_vec());
        let _ = harness.wire();
        harness.fsm.mode = Mode::Download;
        assert!(harness.ctrl(ZmodemCommand::Cancel));
        assert!(harness.wire().starts_with(&[ZDLE; 8][..]));
        assert!(matches!(
            harness.collect_events().last(),
            Some(ZmodemEvent::Finished(ZmodemFinished::Cancelled))
        ));
    }

    #[test]
    fn find_mark_matches_hex_and_bin32_only() {
        assert_eq!(find_mark(b"**\x18B01"), Some(0));
        assert_eq!(find_mark(b"x**\x18C0100"), Some(1));
        assert_eq!(find_mark(b"**\x18A0100"), None, "非 hex/bin32 格式不触发");
        assert_eq!(find_mark(b"*\x18B01"), None);
    }

    /// 快扫改造后的边界：不足标记长度、标记贴尾、跳过不合法候选。
    #[test]
    fn find_mark_scan_boundaries() {
        assert_eq!(find_mark(b""), None);
        assert_eq!(find_mark(b"**\x18"), None, "长度不足不得越界");
        assert_eq!(find_mark(b"abc**\x18B"), Some(3), "标记贴尾仍应命中");
        assert_eq!(
            find_mark(b"**x**\x18B01"),
            Some(3),
            "跳过不合法候选取首个合法标记"
        );
    }

    /// 下载首帧前必须把缺失的落盘目录建出来：否则每个 ZFILE 建文件都失败，
    /// 会话会退化成逐文件 ZSKIP 并报"完成 0 个文件"。
    #[test]
    fn download_creates_missing_directory_before_answering() {
        let base =
            std::env::temp_dir().join(format!("transom-zmodem-mkdir-{}", std::process::id()));
        let nested = base.join("nested/sub");
        let mut harness = harness(nested.clone(), true);
        harness
            .fsm
            .consume_block(b"**\x18B00000000000000\r".to_vec());
        let _ = harness.wire();
        let zfile = build_bin16_header(ZFILE, [0, 0, 0, 0]);
        let info = build_subpacket16(b"landed.txt 3 0 644 1 3", ZCRCW);
        harness.fsm.consume_block([zfile, info].concat());
        assert!(matches!(harness.fsm.mode, Mode::Download));
        assert!(
            nested.join("landed.txt").is_file(),
            "多级缺失目录应在 ZFILE 应答前创建"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// 落盘目录不可创建（路径被普通文件占用）时整场会话无望：直接取消并给出
    /// 失败结局，而不是让对端误判为传输成功。
    #[test]
    fn download_aborts_when_directory_unusable() {
        let blocker =
            std::env::temp_dir().join(format!("transom-zmodem-blocker-{}", std::process::id()));
        std::fs::write(&blocker, b"x").expect("阻塞文件写入成功");
        let mut harness = harness(blocker.join("sub"), true);
        harness
            .fsm
            .consume_block(b"**\x18B00000000000000\r".to_vec());
        let _ = harness.wire();
        harness
            .fsm
            .consume_block(build_bin16_header(ZFILE, [0, 0, 0, 0]));
        let sent = harness.wire();
        assert!(
            sent.starts_with(&[ZDLE; 8][..]),
            "目录不可建时先发出取消序列"
        );
        assert!(matches!(harness.fsm.mode, Mode::Idle));
        assert!(matches!(
            harness.collect_events().last(),
            Some(ZmodemEvent::Finished(ZmodemFinished::Failed(_)))
        ));
        std::fs::remove_file(&blocker).expect("清理阻塞文件");
    }

    /// 驱动线程退出后过滤层必须停止接管字节流：否则协议通道无人消费，
    /// 终端表现为输出冻结且键入全部丢失。
    #[test]
    fn lifeline_drop_restores_passthrough() {
        let mut harness = harness(PathBuf::from("/tmp"), true);
        harness.reader.shared.active.store(true, Ordering::Release);
        drop(FsmLifeline {
            shared: Arc::clone(&harness.reader.shared),
            events: harness.fsm.events.clone(),
        });
        assert!(!harness.reader.shared.active.load(Ordering::Acquire));
        assert!(harness.reader.shared.terminated.load(Ordering::Acquire));
        harness.reader.inner = Box::new(Cursor::new(b"**\x18B00000000000000\r".to_vec()));
        let mut out = [0u8; 64];
        let count = harness.reader.read(&mut out).expect("读取成功");
        assert_eq!(
            &out[..count],
            b"**\x18B00000000000000\r",
            "线程终止后协议字节应原样回到终端通路"
        );
        assert!(!harness.reader.shared.active.load(Ordering::Acquire));
    }

    /// panic 展开同样要复位共享状态并上报失败结局，界面才能清除"传输中"徽章。
    #[test]
    fn lifeline_reports_failure_when_thread_panics() {
        let (event_tx, event_rx) = flume::bounded::<ZmodemEvent>(4);
        let shared = Arc::new(Shared {
            active: AtomicBool::new(true),
            terminated: AtomicBool::new(false),
        });
        let shared_for_thread = Arc::clone(&shared);
        let handle = std::thread::spawn(move || {
            let _lifeline = FsmLifeline {
                shared: shared_for_thread,
                events: event_tx,
            };
            panic!("测试用驱动线程故障");
        });
        assert!(handle.join().is_err(), "子线程应以 panic 结束");
        assert!(
            !shared.active.load(Ordering::Acquire),
            "panic 展开须复位激活标志"
        );
        assert!(shared.terminated.load(Ordering::Acquire));
        let event = event_rx.recv().expect("panic 展开应上报失败结局");
        assert!(matches!(
            event,
            ZmodemEvent::Finished(ZmodemFinished::Failed(_))
        ));
    }

    /// 决策窗口过期之后才提交的上传必须回报可见失败：否则用户的观感只是
    /// "文件对话框关掉、什么都没发生"。
    #[test]
    fn upload_outside_wait_window_reports_failure_to_ui() {
        let mut harness = harness(PathBuf::from("/tmp"), true);
        assert!(harness.ctrl(ZmodemCommand::Upload(vec![PathBuf::from("/tmp/a")])));
        assert!(matches!(
            harness.collect_events().last(),
            Some(ZmodemEvent::Finished(ZmodemFinished::Failed(_)))
        ));
        assert!(
            matches!(harness.fsm.mode, Mode::Idle),
            "过期决策不应改变会话阶段"
        );
    }

    /// 连接断开必须给在途会话一个结局：否则徽章停在"传输中"，Esc 也会发给
    /// 一个已经不存在的会话。
    #[test]
    fn disconnect_reports_outcome_for_in_flight_session() {
        let mut harness = harness(PathBuf::from("/tmp"), true);
        harness.fsm.started = true;
        harness.fsm.mode = Mode::Download;
        harness.reader.shared.active.store(true, Ordering::Release);
        harness.fsm.abort_in_flight();
        assert!(matches!(harness.fsm.mode, Mode::Idle));
        assert!(!harness.reader.shared.active.load(Ordering::Acquire));
        assert!(matches!(
            harness.collect_events().last(),
            Some(ZmodemEvent::Finished(ZmodemFinished::Failed(_)))
        ));
    }

    /// 从未开始的会话不需要编造结局：无徽章即无上报。
    #[test]
    fn disconnect_without_session_stays_silent() {
        let mut harness = harness(PathBuf::from("/tmp"), true);
        harness.fsm.abort_in_flight();
        assert!(harness.collect_events().is_empty());
    }

    /// 假标记激活后迟迟没有合法帧：超时必须释放过滤层并回注已吞字节，
    /// 否则终端永久无输出、无输入，只能断线恢复。
    #[test]
    fn idle_timeout_restores_terminal_forwarding() {
        let mut harness = harness(PathBuf::from("/tmp"), true);
        harness
            .reader
            .shared
            .active
            .store(true, Ordering::Release);
        harness
            .reader
            .raw_tx
            .try_send(b"not a zmodem frame".to_vec())
            .expect("原始通道有界可用");

        harness.fsm.abandon_unstarted("测试超时");

        assert!(!harness.reader.shared.active.load(Ordering::Acquire));
        assert_eq!(
            harness
                .reader
                .replay_rx
                .try_recv()
                .expect("未消费的块应回注终端"),
            b"not a zmodem frame".to_vec()
        );
        assert!(
            harness.collect_events().is_empty(),
            "未开始的会话不应上报结局"
        );
    }

    /// 待命期取消同样静默复位：用户没看到传输开始，不应出现失败徽章。
    #[test]
    fn cancel_while_idle_is_silent() {
        let mut harness = harness(PathBuf::from("/tmp"), true);
        harness
            .reader
            .shared
            .active
            .store(true, Ordering::Release);

        assert!(harness.ctrl(ZmodemCommand::Cancel));

        assert!(!harness.reader.shared.active.load(Ordering::Acquire));
        assert!(harness.collect_events().is_empty());
    }
