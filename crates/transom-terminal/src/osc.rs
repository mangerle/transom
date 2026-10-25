//! 从 PTY 字节流里旁路识别 OSC 序列：命令生命周期（OSC 133）与桌面通知（OSC 777）。
//!
//! 为什么要自己扫：alacritty_terminal 0.26 的 `Event` 枚举没有把 OSC 133/777 透出来
//! （`Term` 只处理它关心的那几个 OSC），而"耗时任务执行完毕"这件事只有 shell 集成自己知道。
//! 反过来用"输出停了很久"这类空闲启发式去猜，会把"程序在等输入""分页器停住"当成任务完成，
//! 在运维终端里这种误报比不报更糟——通知一旦不可信，用户就会把它关掉。
//!
//! 扫描器是**旁路观察者**：不吞掉也不改写任何字节，原始流照旧喂给内核解析器，
//! 因此即使某个程序发了畸形 OSC，也不会影响屏幕内容。

/// OSC 载荷缓冲上限。
///
/// 未闭合的 OSC（例如把二进制直接 dump 到屏幕）会让载荷无限增长，必须有上界：
/// 超限即丢弃当前序列内容、只继续找终止符，避免一次异常输出把内存吃掉。
const MAX_PAYLOAD_BYTES: usize = 4096;

/// OSC 序列的两种合法终止符：ST（`ESC \`）与 BEL。
const TERMINATOR_BEL: u8 = 0x07;
const TERMINATOR_ST: u8 = 0x5c;
const ESCAPE: u8 = 0x1b;
const OSC_INTRO: u8 = 0x5d;

use std::time::Instant;

/// 一条 OSC 序列携带的、上层关心的语义。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OscEvent {
    /// Shell 提示符开始（上一条命令已结束，等待输入）。
    PromptStart,
    /// 命令开始执行（用户已提交输入）。
    CommandStart,
    /// 命令执行结束。退出码与耗时都可能缺失——只有 iTerm2/WezTerm 风格的扩展参数才带。
    CommandFinish {
        exit_code: Option<i32>,
        duration_ms: Option<u64>,
    },
    /// 命令即将执行（`C` 或 WezTerm 的 `E`）；部分集成会把命令文本一并带在参数里。
    PreExec { command: Option<String> },
    /// 采集到的一条完整命令文本，用于命令历史与建议。
    CommandRecorded { command: String },
    /// 程序显式请求的桌面通知（OSC 777）。
    Notify { title: String, body: String },
}

/// 增量扫描状态。
///
/// 用扁平布尔而不是状态枚举：载荷需要在超限时"继续扫描但不收集"，
/// 枚举变体之间会互相复制字段，布尔位反而更直白且没有取出/放回的成本。
#[derive(Debug, Default)]
pub(crate) struct OscScanner {
    /// 上一个字节是 `ESC`，正在确认它后面是不是 `]`。
    awaiting_intro: bool,
    /// 正在收集 OSC 载荷。
    in_payload: bool,
    /// 载荷里已见 `ESC`，下一个字节是 `\` 才算 ST 结束。
    awaiting_st: bool,
    /// 载荷已超长，丢弃内容直到序列结束。
    overflow: bool,
    payload: Vec<u8>,
}

impl OscScanner {
    /// 喂入一段字节流，返回其中识别到的事件。不消费输入，原样透传给内核解析器。
    pub(crate) fn feed(&mut self, bytes: &[u8]) -> Vec<OscEvent> {
        let mut events = Vec::new();
        for &byte in bytes {
            self.push(byte, &mut events);
        }
        events
    }

    fn push(&mut self, byte: u8, events: &mut Vec<OscEvent>) {
        if !self.in_payload {
            self.push_idle(byte);
            return;
        }
        if self.awaiting_st {
            self.awaiting_st = false;
            if byte == TERMINATOR_ST || byte == TERMINATOR_BEL {
                // ST 之外的 `ESC` 后紧跟 BEL 也按结束处理：不能因为一个畸形前缀
                // 让载荷永远闭合不了。
                self.close(events);
            } else if byte == TERMINATOR_ST {
                self.close(events);
            } else {
                // `ESC` 后面不是 `\`：它本身就是载荷内容（例如载荷里出现的转义文本）。
                self.collect(ESCAPE, events);
                self.collect(byte, events);
            }
            return;
        }
        match byte {
            TERMINATOR_BEL => self.close(events),
            ESCAPE => self.awaiting_st = true,
            other => self.collect(other, events),
        }
    }

    fn push_idle(&mut self, byte: u8) {
        if self.awaiting_intro {
            self.awaiting_intro = false;
            if byte == OSC_INTRO {
                self.in_payload = true;
                self.payload.clear();
                self.overflow = false;
            } else if byte == ESCAPE {
                self.awaiting_intro = true;
            }
            return;
        }
        if byte == ESCAPE {
            self.awaiting_intro = true;
        }
    }

    fn collect(&mut self, byte: u8, events: &mut Vec<OscEvent>) {
        if self.overflow {
            return;
        }
        self.payload.push(byte);
        if self.payload.len() > MAX_PAYLOAD_BYTES {
            self.payload.clear();
            self.overflow = true;
            // 已经不可能认得的序列不必再解析，直接当作没有事件。
            let _ = events;
        }
    }

    fn close(&mut self, events: &mut Vec<OscEvent>) {
        let overflowed = self.overflow;
        self.in_payload = false;
        self.awaiting_st = false;
        self.overflow = false;
        if overflowed {
            self.payload.clear();
            return;
        }
        // 复用载荷缓冲而不是每序列重新分配：clear 保留容量，容量已被上限夹住。
        emit(&self.payload, events);
        self.payload.clear();
    }
}

/// 解析一条完整 OSC 载荷。
fn emit(bytes: &[u8], events: &mut Vec<OscEvent>) {
    let Ok(text) = std::str::from_utf8(bytes) else {
        // 载荷不是 UTF-8（二进制 dump 之类）：不是我们要的序列。
        return;
    };
    let Some((code, params)) = text.split_once(';') else {
        return;
    };
    match code {
        "133" => events.extend(parse_command_marker(params)),
        "777" => events.extend(parse_notify(params)),
        _ => {}
    }
}

/// OSC 133 的参数：iTerm2/WezTerm 系 shell 集成用它标记提示符与命令边界。
///
/// 形如 `A`（提示符开始）、`B`（命令输入开始）、`C`（命令输出开始）、
/// `D`、`D;退出码`、`D;退出码;耗时毫秒`，以及 WezTerm 系扩展的
/// `C;<命令>` / `E;<命令>`（把正要执行的命令一并上报）。
fn parse_command_marker(params: &str) -> Option<OscEvent> {
    let mut parts = params.split(';');
    // 标记必须整段等于 A/B/C/D：被吞进垃圾字节的前缀（例如未闭合序列）不允许
    // 靠"首字母相同"就产出一个假事件，误报一次就会让用户关掉整个通知功能。
    let mark = parts.next()?;
    match mark {
        "A" => Some(OscEvent::PromptStart),
        "B" => Some(OscEvent::CommandStart),
        "D" => Some(OscEvent::CommandFinish {
            exit_code: parts.next().and_then(|value| value.parse::<i32>().ok()),
            duration_ms: parts.next().and_then(|value| value.parse::<u64>().ok()),
        }),
        // `C`/`E` 都表示"命令即将执行"：带参数时命令文本已经在手上，
        // 不带参数时交给引擎按网格快照补采。
        "C" | "E" => Some(OscEvent::PreExec {
            command: parts
                .next()
                .map(str::to_string)
                .filter(|text| !text.is_empty()),
        }),
        _ => None,
    }
}

/// OSC 777：`notify;标题;正文` 或 `notify;title=..;body=..;icon=..`。
///
/// 各家实现参数顺序与写法不一致，因此先认 `key=value`，再用位置参数兜底。
fn parse_notify(params: &str) -> Option<OscEvent> {
    let mut parts = params.split(';');
    if parts.next()? != "notify" {
        return None;
    }
    let mut title = String::new();
    let mut body = String::new();
    let mut positional: Vec<&str> = Vec::with_capacity(2);
    for part in parts.take(8) {
        match part.split_once('=') {
            Some(("title", value)) => title = value.to_string(),
            Some(("body", value)) | Some(("message", value)) => body = value.to_string(),
            Some((_, _)) => {}
            None if positional.len() < 2 => positional.push(part),
            None => {}
        }
    }
    if title.is_empty() {
        title = positional.first().copied().unwrap_or_default().to_string();
    }
    if body.is_empty() {
        body = positional.get(1).copied().unwrap_or_default().to_string();
    }
    if title.is_empty() && body.is_empty() {
        return None;
    }
    Some(OscEvent::Notify { title, body })
}

/// 一条已完成命令的归并结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TrackedCommand {
    pub(crate) exit_code: Option<i32>,
    pub(crate) duration_ms: u64,
}

/// 把零散的 OSC 133 标记归并成"一条命令跑了多久"。
///
/// 两种 shell 集成口径都要支持：
/// - iTerm2/WezTerm 在命令结束发 `D;退出码;耗时`，信息齐全；
/// - 只提供 `A`/`B` 的集成（如部分 starship/zsh 配置）没有 `D`，只能靠
///   "命令开始 → 下一次提示符开始"的墙钟差算耗时。
///
/// 因此状态只有一个 `started_at`：见到 `B` 计时，见到 `D` 或下一个 `A` 结算并清零，
/// 保证同一条命令不会被 `D` 与随后的 `A` 报两次。
#[derive(Debug, Default)]
pub(crate) struct CommandTracker {
    started_at: Option<Instant>,
}

impl CommandTracker {
    /// 处理一个 OSC 事件；`now` 由调用方注入，便于测试。
    pub(crate) fn on_event(&mut self, event: &OscEvent, now: Instant) -> Option<TrackedCommand> {
        match event {
            OscEvent::CommandStart => {
                self.started_at = Some(now);
                None
            }
            OscEvent::CommandFinish {
                exit_code,
                duration_ms,
            } => {
                let started_at = self.started_at.take()?;
                let measured = duration_ms.unwrap_or_else(|| {
                    now.checked_duration_since(started_at)
                        .map_or(0, |elapsed| elapsed.as_millis() as u64)
                });
                Some(TrackedCommand {
                    exit_code: *exit_code,
                    duration_ms: measured,
                })
            }
            // 边界与通知类事件不参与"耗时结算"，交由视图自行处理。
            OscEvent::PreExec { .. } | OscEvent::CommandRecorded { .. } => None,
            OscEvent::PromptStart => {
                let started_at = self.started_at.take()?;
                let elapsed = now.checked_duration_since(started_at)?.as_millis() as u64;
                Some(TrackedCommand {
                    exit_code: None,
                    duration_ms: elapsed,
                })
            }
            OscEvent::Notify { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CommandTracker, MAX_PAYLOAD_BYTES, OscEvent, OscScanner, TrackedCommand};
    use std::time::{Duration, Instant};

    /// 按块喂入并汇总事件。
    fn collect(chunks: &[&[u8]]) -> Vec<OscEvent> {
        let mut scanner = OscScanner::default();
        chunks
            .iter()
            .flat_map(|chunk| scanner.feed(chunk))
            .collect()
    }

    fn finish(exit: Option<i32>, duration: Option<u64>) -> Vec<OscEvent> {
        vec![OscEvent::CommandFinish {
            exit_code: exit,
            duration_ms: duration,
        }]
    }

    #[test]
    fn parses_bel_terminated_command_markers() {
        assert_eq!(collect(&[b"\x1b]133;A\x07"]), vec![OscEvent::PromptStart]);
        assert_eq!(collect(&[b"\x1b]133;B\x07"]), vec![OscEvent::CommandStart]);
        // `C` 现在会产出 PreExec：命令文本没带参数时由引擎按网格快照补采。
        assert_eq!(
            collect(&[b"\x1b]133;C\x07"]),
            vec![OscEvent::PreExec { command: None }]
        );
        // 带参上报（WezTerm 的 E、部分集成的 C）直接给出命令文本。
        assert_eq!(
            collect(&[b"\x1b]133;E;git status\x07"]),
            vec![OscEvent::PreExec {
                command: Some("git status".to_string())
            }]
        );
        // 未知标记不能因为"首字母相同"就产出事件。
        assert_eq!(collect(&[b"\x1b]133;Z\x07"]), vec![]);
        assert_eq!(collect(&[b"\x1b]133;D\x07"]), finish(None, None));
        assert_eq!(collect(&[b"\x1b]133;D;0\x07"]), finish(Some(0), None));
        assert_eq!(
            collect(&[b"\x1b]133;D;1;4200\x07"]),
            finish(Some(1), Some(4200))
        );
    }

    /// ST（`ESC \`）同样是合法终止符。
    #[test]
    fn parses_string_terminator() {
        assert_eq!(
            collect(&[b"\x1b]133;A", b"\x1b\\"]),
            vec![OscEvent::PromptStart]
        );
    }

    /// 序列被任意字节边界切碎都必须只产出一次事件：PTY 读数不保证落在序列边界上。
    #[test]
    fn survives_arbitrary_chunk_boundaries() {
        let whole = b"\x1b]133;D;0;10\x07";
        for split in 1..whole.len() {
            let events = collect(&[&whole[..split], &whole[split..]]);
            assert_eq!(events, finish(Some(0), Some(10)), "在 {split} 处切碎");
        }
    }

    #[test]
    fn ignores_other_escape_sequences_and_unknown_osc() {
        assert_eq!(collect(&[b"\x1b[31mred\x1b[0m"]), vec![]);
        assert_eq!(
            collect(&[b"\x1b\x1b]133;A\x07"]),
            vec![OscEvent::PromptStart]
        );
        assert_eq!(collect(&[b"\x1b]0;window title\x07"]), vec![]);
        assert_eq!(collect(&[b"\x1b]133;\xff\x07"]), vec![]);
        assert_eq!(collect(&[b"\x1b]133\x07"]), vec![]);
        assert_eq!(
            collect(&[b"\x1b]133;D;-9;12\x07"]),
            finish(Some(-9), Some(12))
        );
        assert_eq!(collect(&[b"\x1b]133;D;abc;xyz\x07"]), finish(None, None));
        // 未闭合序列吞进来的尾巴不能被当成标记。
        assert_eq!(collect(&[b"\x1b]133;A", b"plain\x07"]), vec![]);
        assert_eq!(collect(&[b"\x1b]133;A1\x07"]), vec![]);
    }

    #[test]
    fn parses_notify_with_keyword_and_positional_forms() {
        assert_eq!(
            collect(&["\x1b]777;notify;构建完成;产物在 out/\x07".as_bytes()]),
            vec![OscEvent::Notify {
                title: "构建完成".into(),
                body: "产物在 out/".into(),
            }]
        );
        assert_eq!(
            collect(&["\x1b]777;notify;title=备份;body=已完成;icon=xterm\x07".as_bytes()]),
            vec![OscEvent::Notify {
                title: "备份".into(),
                body: "已完成".into(),
            }]
        );
        assert_eq!(
            collect(&["\x1b]777;notify;;跑完了\x07".as_bytes()]),
            vec![OscEvent::Notify {
                title: String::new(),
                body: "跑完了".into(),
            }]
        );
        assert_eq!(collect(&[b"\x1b]777;notify\x07"]), vec![]);
        assert_eq!(collect(&[b"\x1b]777;exec;ls\x07"]), vec![]);
    }

    /// 未闭合的超长 OSC 不能把缓冲撑大：丢弃内容，但随后仍能恢复识别。
    #[test]
    fn caps_unterminated_payload_and_recovers() {
        let mut flat: Vec<u8> = Vec::new();
        flat.extend_from_slice(b"\x1b]133;A");
        flat.extend(std::iter::repeat_n(b'x', MAX_PAYLOAD_BYTES + 100));
        flat.push(0x07);
        flat.extend_from_slice(b"\x1b]133;A\x07");
        // 畸形序列被丢弃，紧随其后的合法序列仍要识别出来，且只识别一次。
        assert_eq!(collect(&[&flat]), vec![OscEvent::PromptStart]);
    }

    #[test]
    fn tracker_measures_prompt_to_prompt_when_only_ab_markers() {
        let mut tracker = CommandTracker::default();
        let t0 = Instant::now();
        // 连接后的第一个提示符没有前置命令，不该报事件。
        assert_eq!(tracker.on_event(&OscEvent::PromptStart, t0), None);
        tracker.on_event(&OscEvent::CommandStart, t0);
        let done = tracker
            .on_event(&OscEvent::PromptStart, t0 + Duration::from_millis(2_500))
            .expect("B 之后的下一个 A 应结算一条命令");
        assert_eq!(done.exit_code, None);
        assert!(
            (2_400..=3_000).contains(&done.duration_ms),
            "实测 {:?}",
            done.duration_ms
        );
        // 结算后必须清零，空闲期间的重复 A 不再产出事件。
        assert_eq!(tracker.on_event(&OscEvent::PromptStart, t0), None);
    }

    #[test]
    fn tracker_prefers_reported_duration_and_reports_exit_code() {
        let mut tracker = CommandTracker::default();
        let t0 = Instant::now();
        tracker.on_event(&OscEvent::CommandStart, t0);
        assert_eq!(
            tracker.on_event(
                &OscEvent::CommandFinish {
                    exit_code: Some(1),
                    duration_ms: Some(4_200),
                },
                t0 + Duration::from_millis(90_000)
            ),
            Some(TrackedCommand {
                exit_code: Some(1),
                duration_ms: 4_200
            })
        );
        // `D` 已结算，紧随其后的 `A` 不能重复上报同一条命令。
        assert_eq!(tracker.on_event(&OscEvent::PromptStart, t0), None);
    }

    /// 没有命令开始标记就出现的结束/提示符，一律忽略：
    /// 否则在只发 `D` 不发 `B` 的环境里会把连接建立时刻当成起点，报出一个荒谬耗时。
    #[test]
    fn tracker_ignores_finish_without_start() {
        let mut tracker = CommandTracker::default();
        let t0 = Instant::now();
        assert_eq!(
            tracker.on_event(
                &OscEvent::CommandFinish {
                    exit_code: Some(0),
                    duration_ms: None,
                },
                t0
            ),
            None
        );
        // 没有耗时参数且起点丢失时同样不报。
        assert_eq!(
            tracker.on_event(
                &OscEvent::CommandFinish {
                    exit_code: None,
                    duration_ms: None,
                },
                t0
            ),
            None
        );
    }

    /// 没有耗时参数时按墙钟兜底，且不会倒退成负数。
    #[test]
    fn tracker_falls_back_to_wall_clock() {
        let mut tracker = CommandTracker::default();
        let t0 = Instant::now();
        tracker.on_event(&OscEvent::CommandStart, t0);
        let done = tracker
            .on_event(
                &OscEvent::CommandFinish {
                    exit_code: Some(0),
                    duration_ms: None,
                },
                t0 + Duration::from_millis(1_200),
            )
            .expect("应结算");
        assert!(
            (1_100..=2_000).contains(&done.duration_ms),
            "{:?}",
            done.duration_ms
        );
        let clock_back = tracker.on_event(&OscEvent::CommandStart, t0);
        assert_eq!(clock_back, None);
        tracker.on_event(&OscEvent::CommandStart, t0 + Duration::from_millis(500));
        let earlier = tracker.on_event(
            &OscEvent::CommandFinish {
                exit_code: None,
                duration_ms: None,
            },
            t0,
        );
        assert_eq!(
            earlier,
            Some(TrackedCommand {
                exit_code: None,
                duration_ms: 0
            }),
            "时钟倒退时耗时按 0 处理而不是 panic"
        );
    }

    #[test]
    fn sequences_repeat_without_cross_talk() {
        let mut scanner = OscScanner::default();
        assert_eq!(scanner.feed(b"\x1b]133;A\x07"), vec![OscEvent::PromptStart]);
        assert_eq!(scanner.feed(b"\x1b]133;A\x07"), vec![OscEvent::PromptStart]);
        // 未闭合的序列会把其后的字节当载荷吞掉（与真实终端一致），
        // 直到遇到终止符才恢复；这条性质必须由测试锁住，否则容易被"顺手修好"成丢字节。
        scanner.feed(b"\x1b]133;A");
        assert!(scanner.feed(b"plain output").is_empty());
        assert!(scanner.feed(b"\x07").is_empty());
        assert_eq!(
            scanner.feed(b"\x1b]133;B\x07"),
            vec![OscEvent::CommandStart]
        );
    }
}
