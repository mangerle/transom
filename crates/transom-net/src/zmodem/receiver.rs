//! ZMODEM 接收状态机（sz 下载方向）：纯逻辑转换，文件 I/O 由调用方执行。
//!
//! 设计原理：状态机不持有文件句柄、不做时钟，只把"帧头/子包事件"转换为
//! 动作序列，attach 泵执行动作并把结果回填（`notify_*`）。这样全部传输
//! 决策可用合成字节流直接测试。行为对齐 lrzsz `tryz`/`rzfile`：
//! - 收到 ZSINIT 后读 Attn 子包并回 ZACK（位置 1），后续错误重同步先发
//!   Attn 再发 ZRPOS，使 sz 立即回退重传；
//! - ZFILE 的文件信息子包以 ZCRCW 结尾，应答 ZRPOS(已收偏移)；
//! - 数据子包按终止符语义分别处理（ZCRCQ 应答 ZACK、ZCRCW 应答
//!   ZACK|0x80、ZCRCG/ZCRCE 静默）；
//! - ZEOF 校验偏移后关文件并回 ZRINIT 进入下一文件；ZFIN 应答后吞 "OO"。

use super::frames::{
    ZACK, ZCRCE, ZCRCG, ZCRCQ, ZCRCW, ZDATA, ZEOF, ZFILE, ZFIN, ZNAK, ZRINIT, ZRPOS, ZRQINIT,
    ZSINIT, ZSKIP, build_hex_header, cancel_sequence, receiver_flags,
};
use super::scanner::FrameHeader;
use transom_core::ZmodemFinished;

/// 接收方向单步动作，顺序执行；文件类动作由调用方落实。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RecvAction {
    /// 写入连接线路（帧头/子包/取消序列）。
    Write(Vec<u8>),
    /// 在下载目录创建文件；失败时调用 `notify_create_failed`。
    CreateFile(String),
    /// 向当前文件追加数据。
    WriteFile(Vec<u8>),
    /// 关闭当前文件并固定大小。
    CloseFile { size: u64 },
    /// 传输进度上报。
    Progress {
        name: String,
        transferred: u64,
        total: Option<u64>,
    },
    /// 会话结束。
    Finished(ZmodemFinished),
}

/// 接收状态机状态。
enum RecvState {
    /// 等待 ZSINIT/ZFILE（start 已发出 ZRINIT 能力宣告）。
    WaitFirst,
    /// 接收 ZSINIT 的 Attn 子包。
    ZsinitData,
    /// 接收 ZFILE 的文件信息子包。
    FileInfo,
    /// 等 ZDATA/ZEOF/ZFIN 帧头。
    WaitDataHeader,
    /// ZDATA 子包流接收中。
    InData,
    /// ZFIN 已应答，等待对端 "OO" 收尾。
    WaitOo,
}

/// 接收方向对输入流的消费相位。
///
/// 驱动层据此把原始字节路由到帧头扫描器、子包扫描器或 "OO" 吞除逻辑，
/// 一个字节流在同一时刻只有一个消费者，避免双解析器争抢。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecvPhase {
    /// 等待帧头。
    Head,
    /// 子包流接收窗口（ZSINIT/ZFILE 信息/数据子包）。
    Subpacket,
    /// ZFIN 已应答，等待对端 "OO" 收尾。
    FinishOo,
}

/// ZMODEM 接收状态机。
pub(crate) struct Receiver {
    state: RecvState,
    files_done: usize,
    current_name: String,
    received: u64,
    /// 文件信息中声明的总大小（未知为 None）。
    total: Option<u64>,
    /// 对端 Attn 序列（ZSINIT 子包内容），重同步时先行发送。
    attn: Vec<u8>,
    /// 连续坏子包计数，超限即中止。
    bad_streak: usize,
    /// "OO" 收尾字节计数（跨字节保持，两连 O 即完成）。
    oo_seen: usize,
}

impl Receiver {
    pub(crate) fn new() -> Self {
        Self {
            state: RecvState::WaitFirst,
            files_done: 0,
            current_name: String::new(),
            received: 0,
            total: None,
            attn: Vec::new(),
            bad_streak: 0,
            oo_seen: 0,
        }
    }

    /// 当前输入流消费相位。
    pub(crate) fn phase(&self) -> RecvPhase {
        match self.state {
            RecvState::WaitFirst | RecvState::WaitDataHeader => RecvPhase::Head,
            RecvState::ZsinitData | RecvState::FileInfo | RecvState::InData => RecvPhase::Subpacket,
            RecvState::WaitOo => RecvPhase::FinishOo,
        }
    }

    /// 会话启动：发出 ZRINIT 能力宣告。
    pub(crate) fn start(&mut self) -> Vec<RecvAction> {
        vec![RecvAction::Write(build_hex_header(
            ZRINIT,
            [0, 0, 0, receiver_flags()],
        ))]
    }

    /// 处理帧头事件。
    pub(crate) fn on_header(&mut self, header: FrameHeader) -> Vec<RecvAction> {
        match header.kind {
            ZSINIT => {
                if matches!(self.state, RecvState::WaitFirst) {
                    self.state = RecvState::ZsinitData;
                }
                Vec::new()
            }
            ZFILE => {
                if matches!(self.state, RecvState::WaitFirst) {
                    self.state = RecvState::FileInfo;
                }
                Vec::new()
            }
            ZRQINIT => {
                // 对端重发的启动请求：重新宣告能力（幂等）。
                vec![RecvAction::Write(build_hex_header(
                    ZRINIT,
                    [0, 0, 0, receiver_flags()],
                ))]
            }
            ZDATA => match self.state {
                RecvState::WaitDataHeader => {
                    if u64::from(header.position()) == self.received {
                        self.state = RecvState::InData;
                        Vec::new()
                    } else {
                        self.resync()
                    }
                }
                _ => Vec::new(),
            },
            ZEOF => match self.state {
                RecvState::WaitDataHeader => self.on_eof(header.position()),
                _ => Vec::new(),
            },
            ZFIN => {
                if matches!(self.state, RecvState::WaitDataHeader | RecvState::WaitFirst) {
                    self.state = RecvState::WaitOo;
                    vec![RecvAction::Write(build_hex_header(ZFIN, [0, 0, 0, 0]))]
                } else {
                    Vec::new()
                }
            }
            ZSKIP => {
                // 对端主动放弃（理论上仅发送方会收），按中止收尾。
                self.abort("对端跳过了当前文件")
            }
            _ => Vec::new(),
        }
    }

    /// 处理子包事件（数据 + 终止符）。
    pub(crate) fn on_subpacket(&mut self, data: Vec<u8>, terminator: u8) -> Vec<RecvAction> {
        match self.state {
            RecvState::ZsinitData => {
                // Attn 序列限长 32 字节（lrzsz ZATTNLEN），防御性截断。
                self.attn = data;
                self.attn.truncate(32);
                self.state = RecvState::WaitFirst;
                // lrzsz 约定：ZSINIT 应答 ZACK 且位置字段为 1。
                vec![RecvAction::Write(build_hex_header(ZACK, [1, 0, 0, 0]))]
            }
            RecvState::FileInfo => {
                self.state = RecvState::WaitDataHeader;
                self.on_file_info(data)
            }
            RecvState::InData => self.on_data_subpacket(data, terminator),
            RecvState::WaitDataHeader => {
                // sz 偶发在 ZCRCW 后等待 ZACK|0x80，我们把滞后到达的数据
                // 子包丢弃（偏移校验由下一帧头兜底）。
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// 子包校验失败：协商子包（Attn/文件信息）阶段按 lrzsz 语义回 ZNAK
    /// 请求重发；数据阶段先发 Attn 打断对端，再以当前偏移请求重传。
    pub(crate) fn on_bad_subpacket(&mut self) -> Vec<RecvAction> {
        self.bad_streak += 1;
        if self.bad_streak >= 3 {
            return self.abort("连续多次子包校验失败");
        }
        match self.state {
            RecvState::ZsinitData | RecvState::FileInfo => {
                self.state = RecvState::WaitFirst;
                vec![RecvAction::Write(build_hex_header(ZNAK, [0, 0, 0, 0]))]
            }
            RecvState::InData | RecvState::WaitDataHeader => {
                // ZRPOS 之后对端重发的是 ZDATA 帧头：相位必须退回帧头解析。
                // 停留 InData（子包相位）会把帧头字节当数据写进文件，直接损坏内容。
                self.state = RecvState::WaitDataHeader;
                self.resync()
            }
            _ => Vec::new(),
        }
    }

    /// 文件创建失败：请求对端跳过当前文件。
    ///
    /// 状态必须回到 `WaitFirst`（与 [`Self::on_eof`] 的正常收尾一致）：sz 收到
    /// ZSKIP 后接着发下一个文件的 ZFILE，而 `on_header` 只在 WaitFirst 接受
    /// ZFILE，若停留 WaitDataHeader 则整批后续文件全部丢失。
    pub(crate) fn notify_create_failed(&mut self) -> Vec<RecvAction> {
        self.state = RecvState::WaitFirst;
        self.current_name.clear();
        self.received = 0;
        self.total = None;
        vec![RecvAction::Write(build_hex_header(ZSKIP, [0, 0, 0, 0]))]
    }

    /// 用户取消：写取消序列并结束。
    pub(crate) fn cancel(&mut self) -> Vec<RecvAction> {
        let mut actions = vec![RecvAction::Write(cancel_sequence())];
        if self.files_done > 0 || !self.current_name.is_empty() {
            actions.push(RecvAction::CloseFile {
                size: self.received,
            });
        }
        actions.push(RecvAction::Finished(ZmodemFinished::Cancelled));
        self.state = RecvState::WaitOo;
        self.current_name.clear();
        actions
    }

    /// FinishOo 相位的原始字节处理：吞掉对端 "OO"，其余字节直接收尾
    ///（shell 提示符等残流内容不再等待，避免卡在收尾状态）。
    pub(crate) fn on_raw(&mut self, byte: u8) -> Vec<RecvAction> {
        if !matches!(self.state, RecvState::WaitOo) {
            return Vec::new();
        }
        if byte == b'O' {
            self.oo_seen += 1;
            if self.oo_seen >= 2 {
                return self.finish_completed();
            }
        } else {
            return self.finish_completed();
        }
        Vec::new()
    }

    fn finish_completed(&mut self) -> Vec<RecvAction> {
        self.state = RecvState::WaitOo;
        let files = self.files_done;
        self.current_name.clear();
        vec![RecvAction::Finished(ZmodemFinished::Completed { files })]
    }

    fn abort(&mut self, reason: &str) -> Vec<RecvAction> {
        let mut actions = vec![RecvAction::Write(cancel_sequence())];
        if !self.current_name.is_empty() {
            actions.push(RecvAction::CloseFile {
                size: self.received,
            });
        }
        actions.push(RecvAction::Finished(ZmodemFinished::Failed(
            reason.to_string(),
        )));
        self.state = RecvState::WaitOo;
        self.current_name.clear();
        actions
    }

    /// 错误重同步：Attn（若有）+ ZRPOS(已收偏移)。
    fn resync(&mut self) -> Vec<RecvAction> {
        let mut wire = std::mem::take(&mut self.attn);
        wire.extend_from_slice(&build_hex_header(
            ZRPOS,
            (self.received as u32).to_le_bytes(),
        ));
        vec![RecvAction::Write(wire)]
    }

    /// 解析 ZFILE 的文件信息子包，取出文件名与声明大小。
    fn on_file_info(&mut self, data: Vec<u8>) -> Vec<RecvAction> {
        let (raw_name, fields) = split_file_info(&data);
        let name = sanitize_file_name(&String::from_utf8_lossy(&raw_name));
        self.total = fields
            .first()
            .and_then(|size| size.parse::<u64>().ok())
            .filter(|size| *size > 0);
        self.received = 0;
        self.bad_streak = 0;
        self.current_name = name.clone();
        let mut actions = vec![RecvAction::CreateFile(name.clone())];
        actions.push(RecvAction::Write(build_hex_header(
            ZRPOS,
            0u32.to_le_bytes(),
        )));
        actions
    }

    fn on_data_subpacket(&mut self, data: Vec<u8>, terminator: u8) -> Vec<RecvAction> {
        self.received = self.received.saturating_add(data.len() as u64);
        self.bad_streak = 0;
        let mut actions = Vec::with_capacity(2);
        actions.push(RecvAction::WriteFile(data));
        actions.push(RecvAction::Progress {
            name: self.current_name.clone(),
            transferred: self.received,
            total: self.total,
        });
        match terminator {
            ZCRCQ => {
                // ZACK 携带已收偏移，供 sz 校验同步点。
                actions.push(RecvAction::Write(build_hex_header(
                    ZACK,
                    (self.received as u32).to_le_bytes(),
                )));
            }
            ZCRCW => {
                // lrzsz 语义：ZACK|0x80 标记帧级确认，随后等待 ZEOF。
                actions.push(RecvAction::Write(build_hex_header(
                    ZACK | 0x80,
                    (self.received as u32).to_le_bytes(),
                )));
                self.state = RecvState::WaitDataHeader;
            }
            ZCRCE => self.state = RecvState::WaitDataHeader,
            // ZCRCG：继续接收，无需应答。
            ZCRCG => {}
            _ => {}
        }
        actions
    }

    fn on_eof(&mut self, position: u32) -> Vec<RecvAction> {
        if self.current_name.is_empty() {
            // 文件已被跳过或从未建立：忽略迟到的 EOF。
            return Vec::new();
        }
        if u64::from(position) != self.received {
            // 偏移不符：忽略等对端重发正确 ZEOF（lrzsz 行为）。
            return Vec::new();
        }
        std::mem::take(&mut self.current_name);
        let size = self.received;
        self.files_done += 1;
        self.received = 0;
        self.total = None;
        self.state = RecvState::WaitFirst;
        vec![
            RecvAction::CloseFile { size },
            RecvAction::Write(build_hex_header(ZRINIT, [0, 0, 0, receiver_flags()])),
        ]
    }
}

/// 拆分 ZFILE 文件信息子包为"文件名 + 数字字段"。
///
/// 线上格式以 lrzsz 实测为准：`名字\0 大小 修改时间 模式 类型 文件数 总字节[\0长路径]`。
/// 名字与紧随的字段之间是 **NUL** 而不是空格，只有字段之间才用空格；若整串按空格
/// 切，名字就会与大小粘成一段（真机抓包首段即 `a.bin\0`）。为兼容省略 NUL 的精简
/// 实现，没有 NUL 时退化为"按空格切、首段作名字"。
///
/// 代价：末段可选的长路径（`..` 前缀的完整路径）被忽略，只用首段的裸文件名，
/// 这与路径防穿越红线一致。
fn split_file_info(data: &[u8]) -> (Vec<u8>, Vec<String>) {
    let text = String::from_utf8_lossy(data);
    match text.find('\0') {
        Some(index) => {
            let name = text[..index].as_bytes().to_vec();
            let fields = text[index + 1..]
                .split('\0')
                .next()
                .unwrap_or_default()
                .split_whitespace()
                .map(str::to_string)
                .collect();
            (name, fields)
        }
        None => {
            let mut words = text.split_whitespace();
            let name = words.next().unwrap_or_default().as_bytes().to_vec();
            (name, words.map(str::to_string).collect())
        }
    }
}

/// 净化对端文件名：仅保留 basename，剔除路径分隔与控制字符。
///
/// 防御性要求（AGENTS.md 路径穿越红线）：对端不可信，文件名必须收敛为
/// 纯文件名后再交给下载目录拼接，杜绝 `../` 逃逸与绝对路径写入。
fn sanitize_file_name(raw: &str) -> String {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = base.chars().filter(|c| !c.is_control()).collect();
    let trimmed = cleaned.trim();
    // "." 与 ".." 不含分隔符却能穿过 download_dir.join 指向目录自身或上级，
    // 属 AGENTS.md 的路径防穿越红线，与空名同样归入兜底名。
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        "unnamed.bin".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
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
}
