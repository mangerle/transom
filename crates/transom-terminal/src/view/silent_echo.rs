//! 注入命令回显的客户端过滤器。
//!
//! 设计原理（Why）：远端 Shell 会把写进标准输入的集成脚本原样回显；仅靠命令
//! 末尾的清行序列，用户仍会先看到整段脚本闪一下。这里在读取线程按“整行缓冲 +
//! 标记命中”摘掉回显：只要这一行里出现注入标记，整行都不进网格；命令照常执行，
//! 清行序列再去收拾提示符行。
//!
//! 不按前缀精确匹配，是因为不同 Shell 的行编辑器会在命令前面插提示、重绘或
//! ANSI 高亮；前缀一旦不连续，整段校验就会失配。
//!
//! 代价与局限：等待标记期间会暂存当前行；如果对端关闭 echo，看到首个 OSC
//! 输出或超过缓冲上限就放行并停止过滤，不会卡住集成握手。

/// 注入命令开头的唯一标记；由 `true <标记>` 作为无副作用的命令承载。
pub(crate) const MARKER: &str = "__transom_si_echo__";

/// 远端清行命令 `printf '\033[1A\033[2K'` 实际输出的字节。
const CLEANUP_SEQUENCE: &[u8] = b"\x1b[1A\x1b[2K";

/// 等待标记时最多暂存的一行字节数；超过后放行，避免对端无输出时卡住。
const LINE_LIMIT: usize = 4096;

/// 单条待隐藏注入命令的匹配状态。
#[derive(Debug, Default)]
pub(crate) struct SilentEcho {
    /// 要识别的标记字节，空表示未启用。
    target: Vec<u8>,
    /// 当前行暂存；命中标记时整行丢弃。
    line: Vec<u8>,
    /// 已命中标记，正在丢弃这一行回显直到换行。
    discarding_line: bool,
    /// 远端清行命令应输出的上移次数。
    cleanup_moves: usize,
    /// 还剩多少次远端清行输出需要丢弃。
    cleanup_remaining: usize,
    /// 清行序列可能跨读到达，暂存前缀。
    cleanup_pending: Vec<u8>,
}

impl SilentEcho {
    /// 开始等待一条注入命令的回显；`cleanup_moves` 是远端清行命令的上移次数。
    pub(crate) fn arm(&mut self, target: &[u8], cleanup_moves: usize) {
        self.target.clear();
        self.target.extend_from_slice(target);
        self.line.clear();
        self.discarding_line = false;
        self.cleanup_moves = cleanup_moves;
        self.cleanup_remaining = 0;
        self.cleanup_pending.clear();
    }

    /// 放弃匹配；暂存行与清行状态全部丢弃（写入通道失败时调用）。
    pub(crate) fn disarm(&mut self) {
        self.target.clear();
        self.line.clear();
        self.discarding_line = false;
        self.cleanup_remaining = 0;
        self.cleanup_pending.clear();
    }

    /// 过滤一段输出；返回应继续交给终端引擎的字节。
    pub(crate) fn filter(&mut self, input: &[u8]) -> Vec<u8> {
        if self.target.is_empty() && !self.discarding_line && self.cleanup_remaining == 0 {
            return input.to_vec();
        }
        let mut output = Vec::with_capacity(input.len());
        for &byte in input {
            if self.discarding_line {
                // 回显行整个丢掉，包括行尾 CR/LF；换行处改为本机清掉提示符行。
                if byte == b'\n' {
                    self.discarding_line = false;
                    self.target.clear();
                    output.extend_from_slice(b"\r\x1b[2K");
                    self.cleanup_remaining = self.cleanup_moves;
                }
                continue;
            }
            if self.cleanup_remaining > 0 {
                self.cleanup_pending.push(byte);
                loop {
                    if CLEANUP_SEQUENCE.starts_with(&self.cleanup_pending) {
                        if self.cleanup_pending.len() == CLEANUP_SEQUENCE.len() {
                            self.cleanup_pending.clear();
                            self.cleanup_remaining -= 1;
                        }
                        break;
                    }
                    // 不是清行序列：原样放行（如 bracketed paste 开关）。
                    output.push(self.cleanup_pending.remove(0));
                    if self.cleanup_pending.is_empty() {
                        break;
                    }
                }
                continue;
            }
            if self.target.is_empty() {
                output.push(byte);
                continue;
            }

            // 对端没有回显时，真正的 OSC 输出会先出现；立即放行并停止过滤，
            // 否则 Shell 集成握手会被一直扣在缓冲区里。
            if byte == b']' && self.line.last() == Some(&0x1b) {
                output.extend(self.line.drain(..));
                output.push(byte);
                self.target.clear();
                continue;
            }
            self.line.push(byte);
            if self.line_contains_marker() {
                // 标记可能在 ANSI/重绘之后；命中后整行都不可见。
                self.line.clear();
                self.target.clear();
                self.discarding_line = true;
                continue;
            }
            if byte == b'\n' {
                // 这一行没有标记，说明不是我们的注入回显，原样放行。
                output.extend(self.line.drain(..));
                self.target.clear();
                continue;
            }
            if self.line.len() > LINE_LIMIT {
                output.extend(self.line.drain(..));
                self.target.clear();
            }
        }
        output
    }

    fn line_contains_marker(&self) -> bool {
        if self.target.is_empty() || self.line.len() < self.target.len() {
            return false;
        }
        self.line
            .windows(self.target.len())
            .any(|window| window == self.target)
    }
}

#[cfg(test)]
mod tests {
    use super::{CLEANUP_SEQUENCE, MARKER, SilentEcho};

    fn wrapper() -> String {
        format!(" true {MARKER}")
    }

    #[test]
    fn removes_marked_echo_and_remote_cleanup() {
        let mut filter = SilentEcho::default();
        filter.arm(MARKER.as_bytes(), 2);
        let mut input = format!("{};echo script\r\n\x1b[?2004l\r", wrapper());
        input.push_str(&String::from_utf8_lossy(CLEANUP_SEQUENCE));
        input.push_str(&String::from_utf8_lossy(CLEANUP_SEQUENCE));
        input.push_str("\x1b[?2004h[user@host ~]$ ");
        // 整行回显和两行远端清行都被摘掉，只在本机清一次提示符行。
        assert_eq!(
            filter.filter(input.as_bytes()),
            b"\r\x1b[2K\x1b[?2004l\r\x1b[?2004h[user@host ~]$ "
        );
        assert_eq!(filter.filter(b"next"), b"next");
    }

    #[test]
    fn removes_echo_when_editor_inserts_ansi_before_marker() {
        let mut filter = SilentEcho::default();
        filter.arm(MARKER.as_bytes(), 1);
        let input = format!("\x1b[32mtrue\x1b[0m {MARKER};script\r\nready");
        assert_eq!(
            filter.filter(input.as_bytes()),
            b"\r\x1b[2Kready",
            "ANSI 高亮不能阻止整行摘除"
        );
    }

    #[test]
    fn removes_marked_echo_split_across_reads() {
        let mut filter = SilentEcho::default();
        filter.arm(MARKER.as_bytes(), 1);
        assert_eq!(filter.filter(b"prefix  true __transom"), Vec::<u8>::new());
        let rest = format!(
            "_si_echo__;script\r\n{}ready",
            String::from_utf8_lossy(CLEANUP_SEQUENCE)
        );
        assert_eq!(filter.filter(rest.as_bytes()), b"\r\x1b[2Kready");
    }

    #[test]
    fn passes_output_through_when_marker_never_appears() {
        let mut filter = SilentEcho::default();
        filter.arm(MARKER.as_bytes(), 3);
        assert_eq!(filter.filter(b"hello world\n"), b"hello world\n");
        assert_eq!(filter.filter(b"next"), b"next");
    }

    #[test]
    fn flushes_on_real_osc_when_echo_is_off() {
        let mut filter = SilentEcho::default();
        filter.arm(MARKER.as_bytes(), 1);
        // 没有回显时，探针回报是真正的 ESC] 序列，必须立即放行。
        assert_eq!(
            filter.filter(b"prompt \x1b]777;transom;shell=/bin/bash\x07"),
            b"prompt \x1b]777;transom;shell=/bin/bash\x07"
        );
    }

    #[test]
    fn disarmed_filter_passes_through() {
        let mut filter = SilentEcho::default();
        assert_eq!(filter.filter(b"visible"), b"visible");
        filter.arm(MARKER.as_bytes(), 1);
        filter.disarm();
        assert_eq!(filter.filter(b"visible"), b"visible");
    }
}
