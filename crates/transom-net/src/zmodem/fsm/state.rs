//! ZMODEM 会话状态机核心执行逻辑与文件 I/O 驱动。

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::sync::atomic::Ordering;

use transom_core::{ZmodemDirection, ZmodemEvent, ZmodemFinished};

use super::super::attach::{Shared, ZmodemConfig};
use super::super::files::{self, DownloadWriter, UploadFile};
use super::super::frames::{ZFILE, ZRINIT, ZRQINIT, ZSINIT, cancel_sequence};
use super::super::receiver::{Receiver, RecvAction, RecvPhase};
use super::super::scanner::{HeaderEvent, HeaderScanner, SubpacketEvent, SubpacketScanner};
use super::super::sender::{BLOCK_SIZE, SendAction, Sender};
use super::types::{FsmChannels, Mode, ZmodemCommand};

/// 协议驱动线程的核心状态（与线程壳分离，便于用合成字节流直接测试）。
pub(crate) struct Fsm {
    pub(crate) mode: Mode,
    pub(crate) receiver: Receiver,
    pub(crate) sender: Sender,
    header: HeaderScanner,
    subpacket: SubpacketScanner,
    /// 下载方向当前写入的文件（64 KiB 分块 + 周期同步）。
    download_file: Option<DownloadWriter>,
    /// 上传方向当前读取的文件句柄（按动作携带的偏移 seek）。
    upload_file: Option<std::fs::File>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    pub(crate) events: flume::Sender<ZmodemEvent>,
    pub(crate) replay_tx: flume::Sender<Vec<u8>>,
    pub(crate) raw_rx: flume::Receiver<Vec<u8>>,
    pub(crate) ctrl_rx: flume::Receiver<ZmodemCommand>,
    pub(crate) shared: Arc<Shared>,
    config: ZmodemConfig,
    pub(crate) started: bool,
}

impl Fsm {
    /// 以空闲阶段装配状态机；调用方负责在驱动线程里跑 [`super::driver::fsm_loop`]。
    pub(crate) fn new(channels: FsmChannels, config: ZmodemConfig) -> Self {
        Self {
            mode: Mode::Idle,
            receiver: Receiver::new(),
            sender: Sender::new(0),
            header: HeaderScanner::new(),
            subpacket: SubpacketScanner::new(),
            download_file: None,
            upload_file: None,
            writer: channels.writer,
            events: channels.events,
            replay_tx: channels.replay_tx,
            raw_rx: channels.raw_rx,
            ctrl_rx: channels.ctrl_rx,
            shared: channels.shared,
            config,
            started: false,
        }
    }

    pub(crate) fn consume_block(&mut self, block: Vec<u8>) {
        match self.mode {
            Mode::Idle | Mode::Deciding => self.consume_probe_block(&block),
            Mode::Download => self.consume_download_block(&block),
            Mode::AwaitUpload => {
                // 等待期间对端 rz 继续轮询 ZRINIT，原始字节忽略。
                self.header.reset();
            }
            Mode::Upload => self.consume_upload_block(&block),
        }
    }

    /// 探测阶段的块处理：识别 ZRQINIT/ZRINIT 并分流；sz 跟进的
    /// ZSINIT/ZFILE 在此切换进下载会话。
    ///
    /// 用下标遍历而非直接迭代：一旦切换进下载会话，本块剩余字节属于子包
    /// 通路，必须按偏移整体转交 [`Self::consume_download_block`]。
    fn consume_probe_block(&mut self, block: &[u8]) {
        let mut offset = 0usize;
        while offset < block.len() {
            let byte = block[offset];
            offset += 1;
            match self.header.feed(byte) {
                Some(HeaderEvent::Cancel) => {
                    self.finish(ZmodemFinished::Cancelled);
                    return;
                }
                Some(HeaderEvent::Header(header)) => match header.kind {
                    ZRQINIT => {
                        // sz 发起：立即宣告接收能力，进入分流等待。
                        self.mode = Mode::Deciding;
                        let actions = self.receiver.start();
                        self.run_recv(actions);
                    }
                    ZRINIT => {
                        // lrzsz 的 rz 以 ZRINIT 轮询等待 ZFILE：转上传决策。
                        self.enter_await_upload(header.flags());
                    }
                    ZSINIT | ZFILE => {
                        // 对端确为 sz：进入下载会话，按头格式切换子包 CRC。
                        if !self.config.auto_receive {
                            log::info!("ZMODEM 自动接收已禁用，取消对端下载请求");
                            self.write_wire(&cancel_sequence());
                            self.finish(ZmodemFinished::Failed(
                                "ZMODEM 自动接收已禁用，传输已取消".to_string(),
                            ));
                            return;
                        }
                        // 落盘目录可能尚不存在（用户自定义路径或系统无标准下载
                        // 目录）。在应答前一次性建好：若等到每个 ZFILE 建文件时
                        // 才失败，只会得到逐文件 ZSKIP 与"完成 0 个文件"的误导结论。
                        if let Err(error) = std::fs::create_dir_all(&self.config.download_dir) {
                            log::error!(
                                "ZMODEM 下载目录创建失败: {}，原因: {error}",
                                self.config.download_dir.display()
                            );
                            self.write_wire(&cancel_sequence());
                            self.finish(ZmodemFinished::Failed(format!(
                                "ZMODEM 下载目录无法创建: {}",
                                self.config.download_dir.display()
                            )));
                            return;
                        }
                        self.mode = Mode::Download;
                        self.subpacket.set_crc32(header.crc32_mode);
                        let actions = self.receiver.on_header(header);
                        self.run_recv(actions);
                        // 头之后同块剩余字节是文件信息/Attn 子包：帧头解析器覆盖
                        // 不到子包，若继续按头逐字节解析会把它们当垃圾吞掉，
                        // 而 sz 的 ZRQINIT→ZSINIT→ZFILE 常在一次 burst 里到达。
                        if offset < block.len() {
                            self.consume_download_block(&block[offset..]);
                        }
                        return;
                    }
                    _ => {}
                },
                None => {}
            }
        }
    }

    pub(crate) fn enter_await_upload(&mut self, remote_flags: u8) {
        self.sender = Sender::new(remote_flags);
        self.started = false;
        self.mode = Mode::AwaitUpload;
        let _ = self.events.send(ZmodemEvent::AwaitUpload);
    }

    /// 下载态字节流处理。
    ///
    /// 用下标遍历：会话常在一块数据的中途收尾（对端把 ZFIN 与后续 shell
    /// 提示符挤在同一次 burst 里送达），此时本块剩余字节是终端数据而非协议
    /// 字节，必须回注读线程，否则终端会丢显示最后一段输出。
    fn consume_download_block(&mut self, block: &[u8]) {
        let mut offset = 0usize;
        while offset < block.len() {
            let byte = block[offset];
            offset += 1;
            match self.receiver.phase() {
                RecvPhase::Head => match self.header.feed(byte) {
                    Some(HeaderEvent::Cancel) => {
                        self.finish(ZmodemFinished::Cancelled);
                    }
                    Some(HeaderEvent::Header(header)) => {
                        // 每个帧头都开启全新的子包序列：清掉上一帧可能中途放弃的
                        // 残留缓冲（重同步退回帧头相位时尤其必要），否则半包数据
                        // 会混进下一个子包。
                        self.subpacket.reset();
                        self.subpacket.set_crc32(header.crc32_mode);
                        let actions = self.receiver.on_header(header);
                        self.run_recv(actions);
                    }
                    None => {}
                },
                RecvPhase::Subpacket => match self.subpacket.feed(byte) {
                    Some(SubpacketEvent::Packet(data, terminator)) => {
                        let actions = self.receiver.on_subpacket(data, terminator);
                        self.run_recv(actions);
                    }
                    Some(SubpacketEvent::Bad) => {
                        let actions = self.receiver.on_bad_subpacket();
                        self.run_recv(actions);
                    }
                    Some(SubpacketEvent::Cancel) => {
                        self.finish(ZmodemFinished::Cancelled);
                    }
                    None => {}
                },
                RecvPhase::FinishOo => {
                    let actions = self.receiver.on_raw(byte);
                    self.run_recv(actions);
                }
            }
            // 收尾（取消/ZFIN/"OO"/建目录失败）都可能发生在本次字节内：
            // 会话既已结束，剩余字节交由下方回注，不再按协议解析。
            if matches!(self.mode, Mode::Idle) {
                break;
            }
        }
        if offset < block.len() {
            let _ = self.replay_tx.try_send(block[offset..].to_vec());
        }
    }

    /// 上传态字节流处理：只解析帧头（对端 rz 的 ZRPOS/ZDATA 应答等）。
    /// 同 [`Self::consume_download_block`]，会话中途结束时剩余字节回注终端。
    fn consume_upload_block(&mut self, block: &[u8]) {
        let mut offset = 0usize;
        while offset < block.len() {
            let byte = block[offset];
            offset += 1;
            match self.header.feed(byte) {
                Some(HeaderEvent::Cancel) => {
                    self.finish(ZmodemFinished::Cancelled);
                }
                Some(HeaderEvent::Header(header)) => {
                    let actions = self.sender.on_header(header);
                    self.run_send(actions);
                }
                None => {}
            }
            if matches!(self.mode, Mode::Idle) {
                break;
            }
        }
        if offset < block.len() {
            let _ = self.replay_tx.try_send(block[offset..].to_vec());
        }
    }

    pub(crate) fn run_recv(&mut self, actions: Vec<RecvAction>) {
        for action in actions {
            match action {
                RecvAction::Write(wire) => self.write_wire(&wire),
                RecvAction::CreateFile(name) => self.open_download_file(name),
                RecvAction::WriteFile(data) => self.append_download_data(data),
                RecvAction::CloseFile { .. } => self.close_download_file(),
                RecvAction::Progress {
                    name,
                    transferred,
                    total,
                } => {
                    self.emit_started(ZmodemDirection::Download);
                    self.emit_progress(ZmodemEvent::Progress {
                        direction: ZmodemDirection::Download,
                        name,
                        transferred,
                        total,
                    });
                }
                RecvAction::Finished(outcome) => {
                    self.finish(outcome);
                    return;
                }
            }
        }
    }

    pub(crate) fn run_send(&mut self, actions: Vec<SendAction>) {
        for action in actions {
            match action {
                SendAction::Write(wire) => self.write_wire(&wire),
                SendAction::OpenFile(path) => self.open_upload_file(path),
                SendAction::ReadFile { offset } => self.read_upload_block(offset),
                SendAction::Progress {
                    name,
                    transferred,
                    total,
                } => {
                    self.emit_started(ZmodemDirection::Upload);
                    self.emit_progress(ZmodemEvent::Progress {
                        direction: ZmodemDirection::Upload,
                        name,
                        transferred,
                        total: Some(total),
                    });
                }
                SendAction::Finished(outcome) => {
                    self.finish(outcome);
                    return;
                }
            }
        }
    }

    fn emit_started(&mut self, direction: ZmodemDirection) {
        if self.started {
            return;
        }
        self.started = true;
        let _ = self.events.send(ZmodemEvent::Started { direction });
    }

    /// 进度上报：有界通道被打满时丢弃本次快照。
    ///
    /// 界面只需要"最新位置"，少一帧不影响展示；若在此阻塞，慢消费者
    /// （例如模态文件对话框占住主线程的那段时间）会反过来把传输线程本身
    /// 一起卡住，表现为对端超时。
    fn emit_progress(&self, event: ZmodemEvent) {
        if self.events.try_send(event).is_err() {
            log::debug!("ZMODEM 进度事件通道已满，丢弃本次快照");
        }
    }

    /// 打开下载文件；失败按 lrzsz 语义请求对端跳过当前文件。
    fn open_download_file(&mut self, name: String) {
        match DownloadWriter::create(&self.config.download_dir, &name) {
            Ok(writer) => {
                log::info!("ZMODEM 下载开始写入文件: {}", writer.path().display());
                self.download_file = Some(writer);
            }
            Err(error) => {
                log::warn!("ZMODEM 下载文件创建失败，将请求对端跳过: {name}，原因: {error}");
                let actions = self.receiver.notify_create_failed();
                self.run_recv(actions);
            }
        }
    }

    /// 追加写盘；写失败即中止会话（磁盘满或句柄失效都无继续价值）。
    fn append_download_data(&mut self, data: Vec<u8>) {
        let Some(writer) = self.download_file.as_mut() else {
            return;
        };
        if let Err(error) = writer.write(&data) {
            log::error!("ZMODEM 下载数据写盘失败，传输中止: {error}");
            let actions = self.receiver.cancel();
            self.run_recv(actions);
        }
    }

    /// 文件收尾：冲刷缓冲并强制同步，再释放句柄。
    ///
    /// 同步失败只告警不改变协议结局：数据已全部收到，磁盘层面的失败
    /// 不应把一次成功的传输改判为失败，但必须留下排查线索。
    fn close_download_file(&mut self) {
        if let Some(mut writer) = self.download_file.take()
            && let Err(error) = writer.finish()
        {
            log::warn!(
                "ZMODEM 下载文件收尾同步失败: {}，原因: {error}",
                writer.path().display()
            );
        }
    }

    /// 打开待上传文件；失败跳过并推进到下一个。
    fn open_upload_file(&mut self, path: PathBuf) {
        match files::open_upload_file(&path) {
            Ok(opened) => {
                let UploadFile {
                    file,
                    size,
                    mtime,
                    name,
                } = opened;
                self.upload_file = Some(file);
                self.sender.set_current_file(name);
                let actions = self.sender.notify_file_ready(size, mtime);
                self.run_send(actions);
            }
            Err(error) => {
                log::warn!(
                    "ZMODEM 上传文件打开失败，跳过该文件: {}，原因: {error}",
                    path.display()
                );
                let actions = self.sender.notify_open_failed();
                self.run_send(actions);
            }
        }
    }

    /// 从上传文件按状态机给定偏移读取下一块并推送。
    fn read_upload_block(&mut self, offset: u64) {
        let Some(file) = self.upload_file.as_mut() else {
            return;
        };
        let mut chunk = vec![0u8; BLOCK_SIZE];
        match files::read_block(file, offset, &mut chunk) {
            Ok(count) => {
                chunk.truncate(count);
                let actions = self.sender.push_data(&chunk, count < BLOCK_SIZE);
                self.run_send(actions);
            }
            Err(error) => {
                log::error!("ZMODEM 上传文件读写失败，传输中止，偏移: {offset}，原因: {error}");
                let actions = self.sender.cancel();
                self.run_send(actions);
            }
        }
    }

    fn write_wire(&mut self, wire: &[u8]) {
        let mut guard = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        if let Err(error) = guard.write_all(wire) {
            drop(guard);
            log::error!("ZMODEM 线路写入失败，传输中止: {error}");
            self.finish(ZmodemFinished::Failed(
                "线路写入失败，连接可能已断开".to_string(),
            ));
        }
    }

    /// 会话收尾：上报结局、回注残余字节、复位过滤层并回到待命状态。
    fn finish(&mut self, outcome: ZmodemFinished) {
        let _ = self.events.send(ZmodemEvent::Finished(outcome));
        while let Ok(block) = self.raw_rx.try_recv() {
            let _ = self.replay_tx.try_send(block);
        }
        self.receiver = Receiver::new();
        self.sender = Sender::new(0);
        self.header.reset();
        self.subpacket.reset();
        // 异常终止时把已收到的数据冲刷落盘：协议结局是失败，但用户手里
        // 仍应保留中断前已确认的部分文件，而不是丢掉缓冲区里的最后一段。
        self.close_download_file();
        self.upload_file = None;
        self.started = false;
        self.mode = Mode::Idle;
        self.shared.active.store(false, Ordering::Release);
    }

    /// 连接断开时收尾在途会话：界面徽章必须有明确结局，否则会停在"传输中"，
    /// 且 Esc 取消命令会发给一个已经不存在的会话。
    pub(crate) fn abort_in_flight(&mut self) {
        if !self.started {
            return;
        }
        self.finish(ZmodemFinished::Failed(
            "连接已断开，ZMODEM 传输中止".to_string(),
        ));
    }

    pub(crate) fn handle_command(&mut self, command: ZmodemCommand) {
        match command {
            ZmodemCommand::Upload(files) => {
                // 仅在等待决策的窗口接受上传。
                if !matches!(self.mode, Mode::AwaitUpload | Mode::Deciding) {
                    // 文件选择是阻塞式系统弹窗，用户慢慢挑完回来时决策窗口可能
                    // 已经过期。静默丢弃的表现只是"对话框关掉、什么都没发生"，
                    // 因此必须回一条失败结局写进状态栏。
                    log::warn!(
                        "ZMODEM 上传决策已过期，当前阶段 {:?}，待上传文件数 {}",
                        self.mode,
                        files.len()
                    );
                    let _ = self
                        .events
                        .send(ZmodemEvent::Finished(ZmodemFinished::Failed(
                            "ZMODEM 上传等待已超时，请在远端重新发起 rz".to_string(),
                        )));
                    return;
                }
                self.emit_started(ZmodemDirection::Upload);
                self.mode = Mode::Upload;
                let actions = self.sender.begin(files);
                self.run_send(actions);
            }
            ZmodemCommand::Cancel => self.cancel_session(),
        }
    }

    fn cancel_session(&mut self) {
        match self.mode {
            Mode::Download => {
                let actions = self.receiver.cancel();
                self.run_recv(actions);
            }
            Mode::Upload => {
                let actions = self.sender.cancel();
                self.run_send(actions);
            }
            Mode::Deciding | Mode::AwaitUpload => {
                self.write_wire(&cancel_sequence());
                self.finish(ZmodemFinished::Cancelled);
            }
            Mode::Idle => {}
        }
    }
}
