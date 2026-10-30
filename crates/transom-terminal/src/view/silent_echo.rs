//! 注入命令回显的客户端过滤器。
//!
//! 设计原理（Why）：远端 Shell 会把写进标准输入的集成脚本原样回显；仅靠命令
//! 末尾的清行序列，用户仍会先看到整段脚本闪一下。这里在读取线程用一对标记界定
//! 注入命令：从开始标记到结束标记之间的所有字节都不进网格，不管行编辑器把命令
//! 折成多少行、插了什么 ANSI；命令照常执行，清行序列再去收拾提示符行。
//!
//! 代价与局限：等待标记期间会暂存输出；如果对端关闭 echo，看到首个 OSC 输出或
//! 超过缓冲上限就放行并停止过滤，不会卡住集成握手。标记本身若被拆进 ANSI 序列，
//! 匹配会失败，退化为远端清行兜底。

/// 注入命令开头的唯一标记；由 `true <标记>` 作为无副作用的命令承载。
pub(crate) const MARKER: &str = "__transom_si_echo__";

/// 注入命令末尾的结束标记；用于跨行界定整段回显。
pub(crate) const END_MARKER: &str = "__transom_si_echo_end__";

/// 远端清行命令 `printf '\033[1A\033[2K'` 实际输出的字节。
const CLEANUP_SEQUENCE: &[u8] = b"\x1b[1A\x1b[2K";

/// 等待标记时最多暂存的一行字节数；超过后放行，避免对端无输出时卡住。
const LINE_LIMIT: usize = 4096;

/// 丢弃跨行回显时的上限；超过说明结束标记没有出现，停止过滤防止吞掉正常输出。
const ECHO_LIMIT: usize = 16 * 1024;

/// 单条待隐藏注入命令的匹配状态。
#[derive(Debug, Default)]
pub(crate) struct SilentEcho {
    /// 开始标记字节，空表示未启用。
    target: Vec<u8>,
    /// 结束标记字节。
    end_target: Vec<u8>,
    /// 当前行暂存；命中开始标记后进入丢弃状态。
    line: Vec<u8>,
    /// 正在丢弃开始标记与结束标记之间的回显，可能跨多行。
    discarding_echo: bool,
    /// 已过结束标记，正在丢弃该行剩余内容直到换行。
    discarding_line: bool,
    /// 结束标记可能跨读到达，暂存前缀。
    end_pending: Vec<u8>,
    /// 本轮已丢弃的字节数，防止结束标记缺失时无限吞输出。
    discarded: usize,
    /// 远端清行命令应输出的上移次数。
    cleanup_moves: usize,
    /// 还剩多少次远端清行输出需要丢弃。
    cleanup_remaining: usize,
    /// 清行序列可能跨读到达，暂存前缀。
    cleanup_pending: Vec<u8>,
}

impl SilentEcho {
    /// 开始等待一条注入命令的回显；`cleanup_moves` 是远端清行命令的上移次数。
    pub(crate) fn arm(&mut self, target: &[u8], end_target: &[u8], cleanup_moves: usize) {
        self.target.clear();
        self.target.extend_from_slice(target);
        self.end_target.clear();
        self.end_target.extend_from_slice(end_target);
        self.line.clear();
        self.discarding_echo = false;
        self.discarding_line = false;
        self.end_pending.clear();
        self.discarded = 0;
        self.cleanup_moves = cleanup_moves;
        self.cleanup_remaining = 0;
        self.cleanup_pending.clear();
    }

    /// 放弃匹配；暂存行与清行状态全部丢弃（写入通道失败时调用）。
    pub(crate) fn disarm(&mut self) {
        self.target.clear();
        self.end_target.clear();
        self.line.clear();
        self.discarding_echo = false;
        self.discarding_line = false;
        self.end_pending.clear();
        self.discarded = 0;
        self.cleanup_remaining = 0;
        self.cleanup_pending.clear();
    }

    /// 过滤一段输出；返回应继续交给终端引擎的字节。
    pub(crate) fn filter(&mut self, input: &[u8]) -> Vec<u8> {
        if self.target.is_empty()
            && !self.discarding_echo
            && !self.discarding_line
            && self.cleanup_remaining == 0
        {
            return input.to_vec();
        }
        let mut output = Vec::with_capacity(input.len());
        for &byte in input {
            if self.discarding_line {
                // 结束标记之后到行尾是清行命令文本；换行处本机清一次提示符行。
                if byte == b'\n' {
                    self.discarding_line = false;
                    self.target.clear();
                    self.end_target.clear();
                    output.extend_from_slice(b"\r\x1b[2K");
                    self.cleanup_remaining = self.cleanup_moves;
                }
                continue;
            }
            if self.discarding_echo {
                self.discarded += 1;
                self.end_pending.push(byte);
                loop {
                    if self.end_target.starts_with(&self.end_pending) {
                        if self.end_pending.len() == self.end_target.len() {
                            self.end_pending.clear();
                            self.discarding_echo = false;
                            self.discarding_line = true;
                        }
                        break;
                    }
                    // 不是结束标记前缀：直接丢掉。
                    self.end_pending.remove(0);
                    if self.end_pending.is_empty() {
                        break;
                    }
                }
                if self.discarded > ECHO_LIMIT {
                    // 结束标记没有出现，停止过滤，避免把正常输出一起吞掉。
                    self.end_pending.clear();
                    self.discarding_echo = false;
                    self.target.clear();
                    self.end_target.clear();
                    self.discarded = 0;
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
                output.append(&mut self.line);
                output.push(byte);
                self.target.clear();
                self.end_target.clear();
                continue;
            }
            self.line.push(byte);
            if self.line_contains_marker() {
                // 开始标记可能在 ANSI/重绘之后；命中后开始丢弃整段回显。
                self.line.clear();
                self.target.clear();
                self.discarding_echo = true;
                self.end_pending.clear();
                self.discarded = 0;
                continue;
            }
            if byte == b'\n' {
                // 这一行没有开始标记，说明不是我们的注入回显，原样放行。
                output.append(&mut self.line);
                self.target.clear();
                self.end_target.clear();
                continue;
            }
            if self.line.len() > LINE_LIMIT {
                output.append(&mut self.line);
                self.target.clear();
                self.end_target.clear();
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
    use super::{CLEANUP_SEQUENCE, END_MARKER, MARKER, SilentEcho};

    fn wrapper() -> String {
        format!(" true {MARKER}")
    }

    fn end() -> String {
        format!("; true {END_MARKER}")
    }

    #[test]
    fn removes_marked_echo_and_remote_cleanup() {
        let mut filter = SilentEcho::default();
        filter.arm(MARKER.as_bytes(), END_MARKER.as_bytes(), 2);
        let mut input = format!("{};echo script{}\r\n\x1b[?2004l\r", wrapper(), end());
        input.push_str(&String::from_utf8_lossy(CLEANUP_SEQUENCE));
        input.push_str(&String::from_utf8_lossy(CLEANUP_SEQUENCE));
        input.push_str("\x1b[?2004h[user@host ~]$ ");
        // 整段回显和两行远端清行都被摘掉，只在本机清一次提示符行。
        assert_eq!(
            filter.filter(input.as_bytes()),
            b"\r\x1b[2K\x1b[?2004l\r\x1b[?2004h[user@host ~]$ "
        );
        assert_eq!(filter.filter(b"next"), b"next");
    }

    #[test]
    fn removes_multiline_echo_after_editor_wrap() {
        let mut filter = SilentEcho::default();
        filter.arm(MARKER.as_bytes(), END_MARKER.as_bytes(), 1);
        // 行编辑器把同一条注入命令折成两行：开始和结束标记之间必须整段丢弃。
        let input = format!(
            "{}; if [ -n ... ]\r\n continuation{}\r\n{}ready",
            wrapper(),
            end(),
            String::from_utf8_lossy(CLEANUP_SEQUENCE)
        );
        assert_eq!(filter.filter(input.as_bytes()), b"\r\x1b[2Kready");
    }

    #[test]
    fn removes_echo_when_editor_inserts_ansi_before_marker() {
        let mut filter = SilentEcho::default();
        filter.arm(MARKER.as_bytes(), END_MARKER.as_bytes(), 1);
        let input = format!("\x1b[32mtrue\x1b[0m {MARKER};script{}\r\nready", end());
        assert_eq!(
            filter.filter(input.as_bytes()),
            b"\r\x1b[2Kready",
            "ANSI 高亮不能阻止整段摘除"
        );
    }

    #[test]
    fn removes_marked_echo_split_across_reads() {
        let mut filter = SilentEcho::default();
        filter.arm(MARKER.as_bytes(), END_MARKER.as_bytes(), 1);
        assert_eq!(filter.filter(b"prefix  true __transom"), Vec::<u8>::new());
        let rest = format!(
            "_si_echo__;script{}\r\n{}ready",
            end(),
            String::from_utf8_lossy(CLEANUP_SEQUENCE)
        );
        assert_eq!(filter.filter(rest.as_bytes()), b"\r\x1b[2Kready");
    }

    #[test]
    fn passes_output_through_when_marker_never_appears() {
        let mut filter = SilentEcho::default();
        filter.arm(MARKER.as_bytes(), END_MARKER.as_bytes(), 3);
        assert_eq!(filter.filter(b"hello world\n"), b"hello world\n");
        assert_eq!(filter.filter(b"next"), b"next");
    }

    #[test]
    fn flushes_on_real_osc_when_echo_is_off() {
        let mut filter = SilentEcho::default();
        filter.arm(MARKER.as_bytes(), END_MARKER.as_bytes(), 1);
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
        filter.arm(MARKER.as_bytes(), END_MARKER.as_bytes(), 1);
        filter.disarm();
        assert_eq!(filter.filter(b"visible"), b"visible");
    }
}
