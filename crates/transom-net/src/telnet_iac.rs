//! Telnet 协议命令流解析（RFC 854/855 最小实现）。
//!
//! 设计原理（Why）：
//! - 实现初衷：Telnet 在 NVT 字节流中内嵌协议命令（IAC 引导），终端仿真层
//!   只应看到数据；读方向需要把命令剥离、按协商策略应答，写方向需要把
//!   数据中的 0xFF 转义，两端各自一次完成，不让仿真层感知协议存在；
//! - 核心优势：状态机为纯逻辑（无 I/O），应答经回调注入写出方，可对
//!   合成字节流逐条断言；只实现终端客户端必需的选项（ECHO/SGA/TTYPE/NAWS），
//!   未协商能力一律拒绝，远端不会误用本端不支持的特性；
//! - 代价与局限：不支持行模式（LINEMODE）、认证（AUTH）等扩展选项，
//!   登录交由终端内交互完成；子协商缓冲设有上限，恶意远端的超长子协商
//!   会被截断丢弃而不是无限占用内存。

/// 协议命令与选项常量（RFC 854/855/1073/1091）。
pub(crate) const IAC: u8 = 255;
pub(crate) const WILL: u8 = 251;
pub(crate) const WONT: u8 = 252;
pub(crate) const DO: u8 = 253;
pub(crate) const DONT: u8 = 254;
pub(crate) const SB: u8 = 250;
pub(crate) const SE: u8 = 240;
pub(crate) const NOP: u8 = 241;
const IS: u8 = 0;
const SEND: u8 = 1;

const OPT_ECHO: u8 = 1;
pub(crate) const OPT_SGA: u8 = 3;
pub(crate) const OPT_TTYPE: u8 = 24;
pub(crate) const OPT_NAWS: u8 = 31;

/// 上报给远端的终端类型；与 SSH 通道 `request_pty` 的取值保持一致。
const TERMINAL_TYPE: &str = "xterm-256color";

/// 子协商缓冲上限：TTYPE/NAWS 等已知选项不超过几十字节，64 KiB 已是
/// 极端余量；超过视为远端协议异常，截断丢弃防止内存被恶意灌满。
const MAX_SUBNEGOTIATION: usize = 64 * 1024;

/// IAC 命令流解析状态机。
///
/// 一次 `feed` 消费任意长度的原始字节，命令剥离与应答生成在内部完成；
/// 状态跨 `feed` 调用保留，命令或子协商序列被读块截断时自动续接。
pub(crate) struct IacParser {
    state: State,
    /// 子协商内容累积缓冲（首字节为选项）；仅在 SB 与 SE 之间活跃。
    subneg: Vec<u8>,
}

/// 解析器状态：地面（数据直通）、IAC 已见、等待协商选项、
/// 子协商收集中、子协商内 IAC 已见。
enum State {
    Ground,
    IacSeen,
    Pending(u8),
    Subneg,
    SubnegIac,
}

impl IacParser {
    pub(crate) fn new() -> Self {
        Self {
            state: State::Ground,
            subneg: Vec::with_capacity(64),
        }
    }

    /// 消费一段原始字节。
    ///
    /// 数据字节追加到 `clean`；需要应答的协商通过 `respond` 回调即时写出
    /// （回调内部为单次短临界区写线路，不会重入解析器）。
    pub(crate) fn feed(
        &mut self,
        input: &[u8],
        clean: &mut Vec<u8>,
        respond: &mut dyn FnMut(&[u8]),
    ) {
        for &byte in input {
            match self.state {
                State::Ground => {
                    if byte == IAC {
                        self.state = State::IacSeen;
                    } else {
                        clean.push(byte);
                    }
                }
                State::IacSeen => match byte {
                    // IAC IAC：数据中的转义 0xFF。
                    IAC => {
                        clean.push(IAC);
                        self.state = State::Ground;
                    }
                    WILL | WONT | DO | DONT => self.state = State::Pending(byte),
                    SB => {
                        self.subneg.clear();
                        self.state = State::Subneg;
                    }
                    // 其余单字节命令（NOP/BRK/AYT 等）按协议吞掉即可。
                    _ => self.state = State::Ground,
                },
                State::Pending(command) => {
                    respond(&negotiation_response(command, byte));
                    self.state = State::Ground;
                }
                State::Subneg => {
                    if byte == IAC {
                        self.state = State::SubnegIac;
                    } else if self.subneg.len() < MAX_SUBNEGOTIATION {
                        self.subneg.push(byte);
                    }
                }
                State::SubnegIac => match byte {
                    IAC => {
                        if self.subneg.len() < MAX_SUBNEGOTIATION {
                            self.subneg.push(IAC);
                        }
                        self.state = State::Subneg;
                    }
                    SE => {
                        self.dispatch_subnegotiation(respond);
                        self.state = State::Ground;
                    }
                    // 协议异常：子协商内出现非法 IAC 组合，宽容重同步到地面。
                    _ => self.state = State::Ground,
                },
            }
        }
    }

    /// 处理已完整接收的子协商；仅 TTYPE 的 SEND 查询需要应答。
    fn dispatch_subnegotiation(&mut self, respond: &mut dyn FnMut(&[u8])) {
        let Some((&option, payload)) = self.subneg.split_first() else {
            return;
        };
        if option == OPT_TTYPE && payload.first() == Some(&SEND) {
            let mut response = Vec::with_capacity(TERMINAL_TYPE.len() + 6);
            response.extend_from_slice(&[IAC, SB, OPT_TTYPE, IS]);
            response.extend_from_slice(TERMINAL_TYPE.as_bytes());
            response.extend_from_slice(&[IAC, SE]);
            respond(&response);
        }
        self.subneg.clear();
    }
}

/// 两字节协商命令的策略应答。
///
/// 接受远端的回显与抑制 GO-AHEAD（字符模式终端的标准组合），申报支持
/// 终端类型与窗口尺寸；其余能力一律拒绝，避免远端启用本端未实现的选项。
fn negotiation_response(command: u8, option: u8) -> [u8; 3] {
    match command {
        DO => match option {
            OPT_TTYPE | OPT_NAWS => [IAC, WILL, option],
            _ => [IAC, WONT, option],
        },
        DONT => [IAC, WONT, option],
        WILL => match option {
            OPT_ECHO | OPT_SGA => [IAC, DO, option],
            _ => [IAC, DONT, option],
        },
        WONT => [IAC, DONT, option],
        // 仅在完整命令上调用；非法值不应到达，拒绝处理。
        _ => [IAC, WONT, option],
    }
}

/// 构造 NAWS 子协商报文（列数与行数，各 2 字节大端）。
pub(crate) fn naws_subnegotiation(cols: u16, rows: u16) -> Vec<u8> {
    let mut out = Vec::with_capacity(9);
    out.extend_from_slice(&[IAC, SB, OPT_NAWS]);
    out.extend_from_slice(&cols.to_be_bytes());
    out.extend_from_slice(&rows.to_be_bytes());
    out.extend_from_slice(&[IAC, SE]);
    out
}

/// 把数据字节中的 0xFF 转义为 IAC IAC（写方向，RFC 854 第 4 节）。
pub(crate) fn escape_iac(input: &[u8], out: &mut Vec<u8>) {
    for &byte in input {
        if byte == IAC {
            out.extend_from_slice(&[IAC, IAC]);
        } else {
            out.push(byte);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 汇总一次 feed 的数据输出与应答，便于断言。
    fn run(parser: &mut IacParser, input: &[u8]) -> (Vec<u8>, Vec<Vec<u8>>) {
        let mut clean = Vec::new();
        let mut responses = Vec::new();
        parser.feed(input, &mut clean, &mut |bytes| {
            responses.push(bytes.to_vec())
        });
        (clean, responses)
    }

    #[test]
    fn plain_data_passes_through() {
        let mut parser = IacParser::new();
        let (clean, responses) = run(&mut parser, b"login: ");
        assert_eq!(clean, b"login: ");
        assert!(responses.is_empty());
    }

    #[test]
    fn escaped_iac_becomes_data_byte() {
        let mut parser = IacParser::new();
        let (clean, responses) = run(&mut parser, &[b'a', IAC, IAC, b'b']);
        assert_eq!(clean, vec![b'a', IAC, b'b']);
        assert!(responses.is_empty());
    }

    #[test]
    fn supported_and_rejected_negotiations() {
        let mut parser = IacParser::new();
        let (clean, responses) = run(
            &mut parser,
            &[
                IAC, DO, OPT_TTYPE, IAC, DO, OPT_NAWS, IAC, DO, 5, IAC, WILL, OPT_ECHO, IAC, WILL,
                70,
            ],
        );
        assert!(clean.is_empty());
        assert_eq!(
            responses,
            vec![
                vec![IAC, WILL, OPT_TTYPE],
                vec![IAC, WILL, OPT_NAWS],
                vec![IAC, WONT, 5],
                vec![IAC, DO, OPT_ECHO],
                vec![IAC, DONT, 70],
            ]
        );
    }

    #[test]
    fn command_split_across_feeds_resumes() {
        let mut parser = IacParser::new();
        let (clean, responses) = run(&mut parser, &[b'x', IAC, DO]);
        assert_eq!(clean, b"x");
        assert!(responses.is_empty());
        let (clean, responses) = run(&mut parser, &[OPT_NAWS, b'y']);
        assert_eq!(clean, b"y");
        assert_eq!(responses, vec![vec![IAC, WILL, OPT_NAWS]]);
    }

    #[test]
    fn terminal_type_send_query_answers_is() {
        let mut parser = IacParser::new();
        let (clean, responses) = run(&mut parser, &[IAC, SB, OPT_TTYPE, SEND, IAC, SE]);
        assert!(clean.is_empty());
        let expected = {
            let mut bytes = vec![IAC, SB, OPT_TTYPE, IS];
            bytes.extend_from_slice(TERMINAL_TYPE.as_bytes());
            bytes.extend_from_slice(&[IAC, SE]);
            bytes
        };
        assert_eq!(responses, vec![expected]);
    }

    #[test]
    fn subnegotiation_split_across_feeds_buffers() {
        let mut parser = IacParser::new();
        let (clean, responses) = run(&mut parser, &[IAC, SB, OPT_TTYPE, SEND]);
        assert!(clean.is_empty() && responses.is_empty());
        let (clean, responses) = run(&mut parser, &[IAC, SE, b'!']);
        assert_eq!(clean, b"!");
        assert_eq!(responses.len(), 1);
    }

    #[test]
    fn escaped_iac_inside_subnegotiation_is_literal() {
        let mut parser = IacParser::new();
        // TTYPE SEND 中夹一个转义 0xFF 不影响应答。
        let (clean, responses) = run(&mut parser, &[IAC, SB, OPT_TTYPE, SEND, IAC, IAC, IAC, SE]);
        assert!(clean.is_empty());
        assert_eq!(responses.len(), 1);
    }

    #[test]
    fn oversize_subnegotiation_is_truncated_not_panic() {
        let mut parser = IacParser::new();
        let mut input = vec![IAC, SB, OPT_TTYPE, SEND];
        input.resize(MAX_SUBNEGOTIATION + 16, b'A');
        input.extend_from_slice(&[IAC, SE]);
        let (_clean, responses) = run(&mut parser, &input);
        // 缓冲截断后仍能正常收尾并应答，不 panic、不吞死。
        assert_eq!(responses.len(), 1);
    }

    #[test]
    fn naws_and_escape_helpers() {
        let naws = naws_subnegotiation(120, 32);
        assert_eq!(naws, vec![IAC, SB, OPT_NAWS, 0, 120, 0, 32, IAC, SE]);

        let mut escaped = Vec::new();
        escape_iac(&[0x01, IAC, 0x02], &mut escaped);
        assert_eq!(escaped, vec![0x01, IAC, IAC, 0x02]);
    }
}
