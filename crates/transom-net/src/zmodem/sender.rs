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
use std::time::Duration;

use super::frames::{
    CANFC32, CANOVIO, ZACK, ZCRCE, ZCRCG, ZCRCW, ZDATA, ZEOF, ZFILE, ZFIN, ZNAK, ZRINIT, ZRPOS,
    ZSKIP, build_bin32_header, build_hex_header, build_subpacket16, build_subpacket32,
    cancel_sequence,
};
use super::scanner::FrameHeader;
use transom_core::ZmodemFinished;

/// 单块数据大小：lrzsz sz 默认 1024 字节。
pub(crate) const BLOCK_SIZE: usize = 1024;

/// ZACK 等待窗口的初始退避时长；每次无应答翻倍。
const ACK_BACKOFF_BASE: Duration = Duration::from_secs(2);
/// 退避上限：无限翻倍会让弱网恢复后的等待失去意义。
const ACK_BACKOFF_MAX: Duration = Duration::from_secs(30);
/// 连续无应答的重试上限（不含最后一次判定失败）。
const ACK_RETRY_LIMIT: usize = 5;

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
    /// ZACK/ZEOF/ZFIN 等确认窗口的连续超时次数，驱动指数退避。
    retry_streak: usize,
    /// 最近一次已发出的数据帧（含首块 ZDATA 头），用于确认超时后原样重传。
    ///
    /// 为什么要缓存：ZACK 丢失时唯一正确的恢复动作是重发"同一位移的同一子包"，
    /// 从文件重新读取会引入与已发内容不一致的风险（文件在传输中被改写）。
    last_frame: Option<Vec<u8>>,
    /// 最近一次 ZEOF 帧，等待对端 ZRINIT 超时后原样重发。
    last_eof_frame: Option<Vec<u8>>,
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
            retry_streak: 0,
            last_frame: None,
            last_eof_frame: None,
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
        self.retry_streak = 0;
        self.last_frame = None;
        self.last_eof_frame = None;
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
        // 收到任何合法帧都说明对端仍然在线：确认窗口的退避计数归零。
        self.retry_streak = 0;
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
        let header = first_block.then(|| build_header_of(use_crc32, ZDATA, block_start as u32));
        let packet = build_subpacket_of(use_crc32, chunk, terminator);
        let mut frame = Vec::with_capacity(packet.len() + 24);
        if let Some(header) = &header {
            frame.extend_from_slice(header);
        }
        frame.extend_from_slice(&packet);
        self.last_frame = Some(frame);

        let mut actions = Vec::with_capacity(4);
        if let Some(header) = header {
            actions.push(SendAction::Write(header));
        }
        actions.push(SendAction::Write(packet));
        actions.push(SendAction::Progress {
            name,
            transferred: sent,
            total: size,
        });
        if is_last {
            let eof = build_header_of(use_crc32, ZEOF, size as u32);
            self.last_eof_frame = Some(eof.clone());
            actions.push(SendAction::Write(eof));
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
    ///
    /// 设计原理（Why）：数据块发出后若 ZACK 丢失，必须重发同一帧而不是继续
    /// 推进，否则对端会永久卡在缺口位置。退避由 [`Self::ack_timeout`] 按
    /// 连续超时次数指数增长，避免弱网下高频重传放大拥塞；超过上限即判定
    /// 对端失联，交给上层给出明确失败结局。
    pub(crate) fn on_timeout(&mut self) -> Vec<SendAction> {
        self.retry_streak += 1;
        if self.retry_streak > ACK_RETRY_LIMIT {
            return self.abort("对端长时间无响应");
        }
        match self.state {
            SendState::WaitRpos => self.resend_file_info(),
            SendState::WaitAck => self
                .last_frame
                .clone()
                .map_or_else(Vec::new, |frame| vec![SendAction::Write(frame)]),
            SendState::WaitEofAck => self
                .last_eof_frame
                .clone()
                .map_or_else(Vec::new, |frame| vec![SendAction::Write(frame)]),
            SendState::WaitFin => {
                vec![SendAction::Write(build_hex_header(ZFIN, [0, 0, 0, 0]))]
            }
            _ => Vec::new(),
        }
    }

    /// 当前确认窗口的下一次等待时长；仅 ZACK 窗口启用指数退避。
    ///
    /// 其它阶段沿用调用方的固定静默上限：首帧 offer 与收尾互答本就是
    /// 秒级重试，不需要退避。
    pub(crate) fn ack_timeout(&self) -> Option<Duration> {
        if !matches!(self.state, SendState::WaitAck) {
            return None;
        }
        let shift = self.retry_streak.min(4) as u32;
        Some((ACK_BACKOFF_BASE * (1 << shift)).min(ACK_BACKOFF_MAX))
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
            let eof = build_header_of(use_crc32, ZEOF, size);
            self.last_eof_frame = Some(eof.clone());
            self.state = SendState::WaitEofAck;
            return vec![SendAction::Write(eof)];
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
    include!("sender_tests.rs");
}
