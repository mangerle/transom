//! ZMODEM 发送状态机（rz 上传方向）：纯逻辑转换，文件 I/O 由调用方执行。
//!
//! 设计原理与接收机一致：状态机产出动作，attach 泵执行并回填结果。触发
//! 契约：远端 rz 以 ZRINIT 轮询（lrzsz rz 不发 ZRQINIT），监督层据此弹出
//! 上传决策；本状态机从"已决定上传"开始，直接发送 ZFILE。行为对齐 lrzsz
//! `sz`：
//! - ZFILE 用 hex 头 + 文件信息子包（ZCRCW 结尾）；
//! - ZRPOS 给出续传偏移：偏移小于文件大小则从该处续发，大于等于文件大小
//!   直接 ZEOF；
//! - 远端 ZRINIT 声明 CANFC32 时使用 bin32 头与 CRC-32 子包，否则 CRC-16；
//! - 远端未声明 CANOVIO 时逐块 ZCRCW 同步，防止对端接收缓冲溢出；声明了
//!   则按 lrzsz 默认用 ZCRCG 连发、末块 ZCRCE；
//! - ZEOF 后等 ZRINIT 进入下一文件；全部完成后 ZFIN 互答，我方发 "OO"。

use std::collections::VecDeque;
use std::path::PathBuf;

use super::frames::{
    CANFC32, CANOVIO, ZACK, ZCRCE, ZCRCG, ZCRCW, ZDATA, ZEOF, ZFILE, ZFIN, ZNAK, ZRINIT, ZRPOS,
    ZSKIP, build_bin32_header, build_hex_header, build_subpacket16, build_subpacket32,
    cancel_sequence,
};
use super::scanner::FrameHeader;
use transom_core::ZmodemFinished;

/// 单块数据大小：lrzsz sz 默认 1024 字节。
pub(crate) const BLOCK_SIZE: usize = 1024;

/// 依 CRC-32 开关选择帧头格式构造（独立函数便于借用约束下复用）。
fn build_header_of(use_crc32: bool, kind: u8, position: u32) -> Vec<u8> {
    let body = position.to_le_bytes();
    if use_crc32 {
        build_bin32_header(kind, body)
    } else {
        build_hex_header(kind, body)
    }
}

/// 构造 ZFILE 的文件信息子包内容。
///
/// 必须按 lrzsz 的线上布局：`名字\0大小 修改时间(八进制) 模式(八进制) 类型 文件数 总字节\0`。
/// 名字段与数字段之间是 **NUL**：真机 `sz` 的抓包首段就是 `a.bin\0`，若改用空格相连，
/// 对端 `rz` 会把整串当成一个超长文件名（其在 NUL 处截断取名）。
fn file_info(name: &str, size: u64, mtime: Option<u32>, files: u32) -> String {
    format!(
        "{name}\0{size} {:o} 100644 0 {files} {size}\0",
        mtime.unwrap_or(0)
    )
}

/// 依 CRC-32 开关选择子包校验构造。
fn build_subpacket_of(use_crc32: bool, data: &[u8], terminator: u8) -> Vec<u8> {
    if use_crc32 {
        build_subpacket32(data, terminator)
    } else {
        build_subpacket16(data, terminator)
    }
}

/// 发送方向单步动作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SendAction {
    /// 写入连接线路。
    Write(Vec<u8>),
    /// 打开待上传文件（调用方回填 `notify_file_ready` 或 `open_failed`）。
    OpenFile(PathBuf),
    /// 请求从指定偏移读取当前文件下一块（至多 BLOCK_SIZE 字节）。
    ///
    /// 偏移由状态机维护（ZRPOS 续传起点 + 已发字节数），调用方按它 seek，
    /// 状态机自身不持有文件句柄。
    ReadFile { offset: u64 },
    /// 传输进度上报。
    Progress {
        name: String,
        transferred: u64,
        total: u64,
    },
    /// 会话结束。
    Finished(ZmodemFinished),
}

/// 当前文件的上行状态。
struct UploadFile {
    name: String,
    size: u64,
    sent: u64,
    /// ZDATA 帧头是否已发出（块序列开头发一次）。
    data_header_sent: bool,
}

/// 发送状态机状态。
enum SendState {
    /// 尚未开始或已结束。
    Idle,
    /// 等待调用方打开文件。
    WaitOpen,
    /// ZFILE 已发，等待 ZRPOS/ZSKIP/ZRINIT。
    WaitRpos,
    /// 数据块发送中。
    Sending,
    /// ZCRCW 子包已发，等 ZACK。
    WaitAck,
    /// ZEOF 已发，等 ZRINIT。
    WaitEofAck,
    /// ZFIN 已发，等对端 ZFIN。
    WaitFin,
}

/// ZMODEM 发送状态机。
pub(crate) struct Sender {
    state: SendState,
    queue: VecDeque<PathBuf>,
    files_done: usize,
    current: Option<UploadFile>,
    /// 远端 ZRINIT 能力标志（触发上传时记录）。
    remote_flags: u8,
    /// 连续异常计数，超限中止。
    error_streak: usize,
}

impl Sender {
    pub(crate) fn new(remote_flags: u8) -> Self {
        Self {
            state: SendState::Idle,
            queue: VecDeque::new(),
            files_done: 0,
            current: None,
            remote_flags,
            error_streak: 0,
        }
    }

    /// 决定上传：登记文件队列并请求打开第一个文件。
    pub(crate) fn begin(&mut self, files: Vec<PathBuf>) -> Vec<SendAction> {
        self.queue = files.into();
        self.files_done = 0;
        self.open_next()
    }

    /// 用户取消：写取消序列并结束。
    pub(crate) fn cancel(&mut self) -> Vec<SendAction> {
        self.state = SendState::Idle;
        vec![
            SendAction::Write(cancel_sequence()),
            SendAction::Finished(ZmodemFinished::Cancelled),
        ]
    }

    /// 文件打开成功：回填大小并发出 ZFILE 与文件信息子包。
    pub(crate) fn notify_file_ready(&mut self, size: u64, mtime: Option<u32>) -> Vec<SendAction> {
        let Some(current) = self.current.as_mut() else {
            return Vec::new();
        };
        current.size = size;
        current.sent = 0;
        current.data_header_sent = false;
        self.state = SendState::WaitRpos;
        self.error_streak = 0;
        // ZMODEM 位置字段为 32 位：超限文件在发送 ZFILE 前直接跳过，
        // 保证后续偏移算术不截断（不变量由本拦截点维持）。
        if size > u64::from(u32::MAX) {
            let mut actions = vec![SendAction::Write(build_hex_header(ZSKIP, [0, 0, 0, 0]))];
            self.current = None;
            actions.extend(self.open_next());
            return actions;
        }
        // 信息子包按 lrzsz 线上布局发出：名字 + NUL + 大小/时间/模式/类型/文件数/总字节。
        let info = file_info(&current.name, size, mtime, 1);
        vec![
            SendAction::Write(build_hex_header(ZFILE, [0, 0, 0, 0])),
            SendAction::Write(build_subpacket16(info.as_bytes(), ZCRCW)),
        ]
    }

    /// 文件打开失败：跳过并尝试下一个。
    pub(crate) fn notify_open_failed(&mut self) -> Vec<SendAction> {
        self.current = None;
        self.open_next()
    }

    /// 处理帧头事件。
    pub(crate) fn on_header(&mut self, header: FrameHeader) -> Vec<SendAction> {
        // 记录远端能力：部分 rz 在每轮轮询时重发 ZRINIT。
        if header.kind == ZRINIT {
            self.remote_flags = header.flags();
        }
        match self.state {
            SendState::WaitRpos => match header.kind {
                ZRPOS => self.on_zrpos(header.position()),
                ZSKIP => self.skip_current(),
                ZRINIT => self.resend_file_info(),
                ZFIN => self.finish_session(),
                _ => Vec::new(),
            },
            SendState::Sending | SendState::WaitAck => match header.kind {
                ZSKIP => self.skip_current(),
                ZFIN => self.finish_session(),
                // ZCRCW 同步模式下对端确认：继续下一块。
                ZACK => self.on_ack(),
                // 续传点：对端任何时候用 ZRPOS 指定"从这里继续"，必须接受，
                // 否则坏包之后双方偏移永久错位，上传只会挂死到超时。
                ZRPOS => self.on_zrpos(header.position()),
                // ZNAK（坏子包）与数据阶段重轮询的 ZRINIT 都表示上一段不被接受：
                // 停止推进管线，退回等 ZRPOS 并重新 offer 文件信息。
                ZNAK | ZRINIT => self.recover_from_error(),
                _ => Vec::new(),
            },
            SendState::WaitEofAck => match header.kind {
                ZRINIT | ZSKIP => {
                    self.files_done += 1;
                    self.current = None;
                    self.open_next_or_finish()
                }
                ZFIN => self.finish_session(),
                _ => Vec::new(),
            },
            SendState::WaitFin => match header.kind {
                ZFIN => {
                    self.state = SendState::Idle;
                    let files = self.files_done;
                    vec![
                        SendAction::Write(b"OO".to_vec()),
                        SendAction::Finished(ZmodemFinished::Completed { files }),
                    ]
                }
                _ => Vec::new(),
            },
            _ => Vec::new(),
        }
    }

    /// 调用方读回数据块，产出子包（或收尾 ZEOF）。
    pub(crate) fn push_data(&mut self, chunk: &[u8], is_last: bool) -> Vec<SendAction> {
        if !matches!(self.state, SendState::Sending | SendState::WaitAck) {
            return Vec::new();
        }
        let Some(current) = self.current.as_mut() else {
            return Vec::new();
        };
        let sync_mode = self.remote_flags & CANOVIO == 0;
        let use_crc32 = self.remote_flags & CANFC32 != 0;
        current.sent = current.sent.saturating_add(chunk.len() as u64);
        let block_start = current.sent - chunk.len() as u64;
        let sent = current.sent;
        let name = current.name.clone();
        let size = current.size;
        let first_block = !current.data_header_sent;
        current.data_header_sent = true;
        let terminator = if is_last {
            ZCRCE
        } else if sync_mode {
            ZCRCW
        } else {
            ZCRCG
        };
        let mut actions = Vec::with_capacity(4);
        if first_block {
            actions.push(SendAction::Write(build_header_of(
                use_crc32,
                ZDATA,
                block_start as u32,
            )));
        }
        actions.push(SendAction::Write(build_subpacket_of(
            use_crc32, chunk, terminator,
        )));
        actions.push(SendAction::Progress {
            name,
            transferred: sent,
            total: size,
        });
        if is_last {
            actions.push(SendAction::Write(build_header_of(
                use_crc32,
                ZEOF,
                size as u32,
            )));
            self.state = SendState::WaitEofAck;
        } else if sync_mode {
            self.state = SendState::WaitAck;
        } else {
            actions.push(SendAction::ReadFile { offset: sent });
        }
        actions
    }

    /// ZACK 到达（ZCRCW 同步模式）：继续读下一块。
    pub(crate) fn on_ack(&mut self) -> Vec<SendAction> {
        if matches!(self.state, SendState::WaitAck) {
            let offset = self.current.as_ref().map(|file| file.sent).unwrap_or(0);
            self.state = SendState::Sending;
            vec![SendAction::ReadFile { offset }]
        } else {
            Vec::new()
        }
    }

    /// 远端无应答超时（attach 层触发）：在等待状态下重发关键帧。
    pub(crate) fn on_timeout(&mut self) -> Vec<SendAction> {
        self.error_streak += 1;
        if self.error_streak >= 5 {
            return self.abort("对端长时间无响应");
        }
        match self.state {
            SendState::WaitRpos => self.resend_file_info(),
            _ => Vec::new(),
        }
    }

    fn on_zrpos(&mut self, offset: u32) -> Vec<SendAction> {
        let use_crc32 = self.remote_flags & CANFC32 != 0;
        let Some(current) = self.current.as_mut() else {
            return Vec::new();
        };
        self.error_streak = 0;
        if u64::from(offset) >= current.size {
            // 偏移已达文件末尾：直接宣告 EOF（0 字节文件同样走此分支）。
            let size = current.size as u32;
            self.state = SendState::WaitEofAck;
            return vec![SendAction::Write(build_header_of(use_crc32, ZEOF, size))];
        }
        current.sent = u64::from(offset);
        current.data_header_sent = false;
        let sent = current.sent;
        self.state = SendState::Sending;
        vec![SendAction::ReadFile { offset: sent }]
    }

    /// 跳过当前文件：lrzsz sz 对 ZSKIP 直接进入下一文件。
    fn skip_current(&mut self) -> Vec<SendAction> {
        self.current = None;
        self.open_next_or_finish()
    }

    fn open_next_or_finish(&mut self) -> Vec<SendAction> {
        if self.current.is_none() && self.queue.is_empty() {
            self.finish_session()
        } else {
            self.open_next()
        }
    }

    fn open_next(&mut self) -> Vec<SendAction> {
        match self.queue.pop_front() {
            Some(path) => {
                self.state = SendState::WaitOpen;
                vec![SendAction::OpenFile(path)]
            }
            None => {
                if self.files_done == 0 {
                    self.abort("没有可上传的文件")
                } else {
                    self.finish_session()
                }
            }
        }
    }

    /// 数据阶段的出错恢复：退回"等 ZRPOS"并重新 offer 文件信息。
    ///
    /// 复用 lrzsz 的标准重对话（ZFILE → 对端 ZRPOS → 从新偏移续传），不引入
    /// 新报文类型；对端持续不应答时由 `on_timeout` 的 streak 上限兜底终止，
    /// 避免 NAK 风暴把上传拖成无限等待。
    fn recover_from_error(&mut self) -> Vec<SendAction> {
        self.error_streak += 1;
        if self.error_streak >= 5 {
            return self.abort("对端持续拒绝上传数据");
        }
        self.state = SendState::WaitRpos;
        self.resend_file_info()
    }

    fn resend_file_info(&mut self) -> Vec<SendAction> {
        let Some(current) = self.current.as_ref() else {
            return Vec::new();
        };
        let info = file_info(&current.name, current.size, None, 1);
        vec![
            SendAction::Write(build_hex_header(ZFILE, [0, 0, 0, 0])),
            SendAction::Write(build_subpacket16(info.as_bytes(), ZCRCW)),
        ]
    }

    fn finish_session(&mut self) -> Vec<SendAction> {
        self.state = SendState::WaitFin;
        vec![SendAction::Write(build_hex_header(ZFIN, [0, 0, 0, 0]))]
    }

    fn abort(&mut self, reason: &str) -> Vec<SendAction> {
        self.state = SendState::Idle;
        vec![
            SendAction::Write(cancel_sequence()),
            SendAction::Finished(ZmodemFinished::Failed(reason.to_string())),
        ]
    }

    /// attach 打开文件成功后登记元数据（在 notify_file_ready 之前调用）。
    pub(crate) fn set_current_file(&mut self, name: String) {
        self.current = Some(UploadFile {
            name,
            size: 0,
            sent: 0,
            data_header_sent: false,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zmodem::frames::{CANFDX, ZDLE};

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
        for _ in 0..4 {
            assert_eq!(tx.on_timeout().len(), 2, "前四次超时重发 ZFILE");
        }
        let actions = tx.on_timeout();
        assert_eq!(
            actions[1],
            SendAction::Finished(ZmodemFinished::Failed("对端长时间无响应".to_string()))
        );
    }
}
