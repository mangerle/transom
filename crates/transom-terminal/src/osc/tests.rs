//! OSC 扫描与命令追踪单元测试。

use super::scanner::OscScanner;
use super::types::{
    CommandTracker, MAX_PAYLOAD_BYTES, OscEvent, TitleAction, TitleGate, TrackedCommand,
};
use std::time::{Duration, Instant};

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
    assert_eq!(
        collect(&[b"\x1b]133;C\x07"]),
        vec![OscEvent::PreExec { command: None }]
    );
    assert_eq!(
        collect(&[b"\x1b]133;E;git status\x07"]),
        vec![OscEvent::PreExec {
            command: Some("git status".to_string())
        }]
    );
    assert_eq!(collect(&[b"\x1b]133;Z\x07"]), vec![]);
    assert_eq!(collect(&[b"\x1b]133;D\x07"]), finish(None, None));
    assert_eq!(collect(&[b"\x1b]133;D;0\x07"]), finish(Some(0), None));
    assert_eq!(
        collect(&[b"\x1b]133;D;1;4200\x07"]),
        finish(Some(1), Some(4200))
    );
}

#[test]
fn parses_string_terminator() {
    assert_eq!(
        collect(&[b"\x1b]133;A", b"\x1b\\"]),
        vec![OscEvent::PromptStart]
    );
}

#[test]
fn survives_arbitrary_chunk_boundaries() {
    let whole = b"\x1b]133;D;0;10\x07";
    for split in 1..whole.len() {
        let events = collect(&[&whole[..split], &whole[split..]]);
        assert_eq!(events, finish(Some(0), Some(10)), "在 {split} 处切碎");
    }
}

#[test]
fn parses_osc7_working_directory() {
    assert_eq!(
        collect(&[b"\x1b]7;file://host/home/user\x07"]),
        vec![OscEvent::Cwd {
            path: "/home/user".to_string()
        }]
    );
    // host 为空是合法形式：`file:///var/log` 的路径从第一个 `/` 开始。
    assert_eq!(
        collect(&[b"\x1b]7;file:///var/log\x1b\\"]),
        vec![OscEvent::Cwd {
            path: "/var/log".to_string()
        }]
    );
    assert_eq!(
        collect(&["\x1b]7;file://h/项目/名 字\x07".as_bytes()]),
        vec![OscEvent::Cwd {
            path: "/项目/名 字".to_string()
        }]
    );
    assert_eq!(
        collect(&[b"\x1b]7;file:///root\x07"]),
        vec![OscEvent::Cwd {
            path: "/root".to_string()
        }]
    );
    assert_eq!(
        collect(&[b"\x1b]7;file:///\x07"]),
        vec![OscEvent::Cwd {
            path: "/".to_string()
        }]
    );
}

#[test]
fn decodes_osc7_percent_encoding_and_tolerates_literal_percent() {
    assert_eq!(
        collect(&[b"\x1b]7;file://h/home/a%20b/%E4%B8%AD\x07"]),
        vec![OscEvent::Cwd {
            path: "/home/a b/中".to_string()
        }]
    );
    // 提示符脚本直接拼 `$PWD` 时目录名里的 `%` 未必编码，不能整条丢弃。
    assert_eq!(
        collect(&[b"\x1b]7;file:///home/100%\x07"]),
        vec![OscEvent::Cwd {
            path: "/home/100%".to_string()
        }]
    );
}

#[test]
fn rejects_invalid_osc7_payloads() {
    assert_eq!(collect(&[b"\x1b]7;http://h/p\x07"]), vec![]);
    assert_eq!(collect(&[b"\x1b]7;file://host\x07"]), vec![]);
    assert_eq!(collect(&[b"\x1b]7;file://\x07"]), vec![]);
    // 解码后含换行等控制字符的路径会污染路径输入框与界面文本，必须拒绝。
    assert_eq!(collect(&[b"\x1b]7;file:///bad%0Apath\x07"]), vec![]);
    assert_eq!(collect(&[b"\x1b]7;file:///bad%00path\x07"]), vec![]);
}

#[test]
fn osc7_survives_chunk_boundary() {
    assert_eq!(
        collect(&[b"\x1b]7;file://host/home/", b"user\x07"]),
        vec![OscEvent::Cwd {
            path: "/home/user".to_string()
        }]
    );
}

#[test]
fn parses_osc9_notifications_and_ignores_reserved_subcodes() {
    assert_eq!(
        collect(&[b"\x1b]9;build finished\x07"]),
        vec![OscEvent::Notify {
            title: String::new(),
            body: "build finished".to_string()
        }]
    );
    // 正文可以自带分号，不能在第一个分号处截断。
    assert_eq!(
        collect(&[b"\x1b]9;hello;world\x07"]),
        vec![OscEvent::Notify {
            title: String::new(),
            body: "hello;world".to_string()
        }]
    );
    // 9;4 是进度、9;9 是 ConEmu cwd：都不属于通知，不能弹窗。
    assert_eq!(collect(&[b"\x1b]9;4;1;50\x07"]), vec![]);
    assert_eq!(collect(&[b"\x1b]9;9;/home/user\x07"]), vec![]);
    assert_eq!(collect(&[b"\x1b]9;\x07"]), vec![]);
    assert_eq!(collect(&[b"\x1b]9;   \x07"]), vec![]);
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

#[test]
fn caps_unterminated_payload_and_recovers() {
    let mut flat: Vec<u8> = Vec::new();
    flat.extend_from_slice(b"\x1b]133;A");
    flat.extend(std::iter::repeat_n(b'x', MAX_PAYLOAD_BYTES + 100));
    flat.push(0x07);
    flat.extend_from_slice(b"\x1b]133;A\x07");
    assert_eq!(collect(&[&flat]), vec![OscEvent::PromptStart]);
}

#[test]
fn title_gate_only_publishes_titles_set_by_running_commands() {
    let mut gate = TitleGate::default();
    // 初始提示符自己设置的标题不能接管标签。
    assert_eq!(
        gate.on_title("wangle@ubuntu: ~/project"),
        TitleAction::Ignore
    );
    assert_eq!(
        gate.on_boundary(&OscEvent::PromptStart),
        TitleAction::Ignore
    );
    assert_eq!(
        gate.on_title("wangle@ubuntu: ~/project"),
        TitleAction::Ignore
    );

    // 命令开始后，TUI/程序设置的标题可以发布。
    assert_eq!(
        gate.on_boundary(&OscEvent::PreExec { command: None }),
        TitleAction::Ignore
    );
    assert_eq!(gate.on_title("app.rs - nvim"), TitleAction::Publish);
    assert_eq!(
        gate.on_title("user@host: /path"),
        TitleAction::Ignore,
        "命令期间到达的提示符标题也不能覆盖标签"
    );

    // 命令结束或新提示符出现时清除，标签回落会话名。
    assert_eq!(
        gate.on_boundary(&OscEvent::CommandFinish {
            exit_code: Some(0),
            duration_ms: Some(10),
        }),
        TitleAction::Clear
    );
    assert_eq!(
        gate.on_boundary(&OscEvent::PromptStart),
        TitleAction::Ignore
    );
    assert_eq!(
        gate.on_title("wangle@ubuntu: ~/project"),
        TitleAction::Ignore
    );
}

#[test]
fn title_gate_handles_reset_and_command_start_marker() {
    let mut gate = TitleGate::default();
    assert_eq!(
        gate.on_boundary(&OscEvent::CommandStart),
        TitleAction::Ignore
    );
    assert_eq!(gate.on_title("tmux: work"), TitleAction::Publish);
    assert_eq!(gate.on_reset(), TitleAction::Clear);
    assert_eq!(gate.on_reset(), TitleAction::Ignore, "重复重置不应重复通知");
    assert_eq!(
        gate.on_title("tmux: work"),
        TitleAction::Publish,
        "同一命令运行期间仍可再次设置标题"
    );
    assert_eq!(gate.on_boundary(&OscEvent::PromptStart), TitleAction::Clear);
}

#[test]
fn title_gate_rejects_common_prompt_title_shapes_even_while_command_runs() {
    let mut gate = TitleGate::default();
    assert_eq!(
        gate.on_boundary(&OscEvent::CommandStart),
        TitleAction::Ignore
    );
    for title in [
        "wangle@ubuntu: ~/project",
        "user@host",
        "[user@localhost ~]$",
        "user@host /srv/app",
    ] {
        assert_eq!(gate.on_title(title), TitleAction::Ignore, "{title}");
    }
    // 真正包含 @ 的 TUI 标题不能被误杀。
    assert_eq!(gate.on_title("user@host - nvim"), TitleAction::Publish);
}

#[test]
fn tracker_starts_on_preexec_because_shell_hooks_do_not_send_b() {
    let mut tracker = CommandTracker::default();
    let t0 = Instant::now();
    // 我们的 bash/zsh/fish 钩子只发 133;C（PreExec），不发 133;B。
    assert_eq!(
        tracker.on_event(&OscEvent::PreExec { command: None }, t0),
        None
    );
    let done = tracker
        .on_event(
            &OscEvent::CommandFinish {
                exit_code: Some(0),
                duration_ms: None,
            },
            t0 + Duration::from_secs(15),
        )
        .expect("C 之后应能结算命令");
    assert_eq!(done.exit_code, Some(0));
    assert!(
        (14_900..=15_100).contains(&done.duration_ms),
        "实测 {:?}",
        done.duration_ms
    );
}

#[test]
fn tracker_prefers_preexec_over_earlier_command_start() {
    let mut tracker = CommandTracker::default();
    let t0 = Instant::now();
    assert_eq!(tracker.on_event(&OscEvent::CommandStart, t0), None);
    assert_eq!(
        tracker.on_event(
            &OscEvent::PreExec { command: None },
            t0 + Duration::from_secs(2)
        ),
        None
    );
    let done = tracker
        .on_event(
            &OscEvent::CommandFinish {
                exit_code: Some(1),
                duration_ms: None,
            },
            t0 + Duration::from_secs(5),
        )
        .expect("应以 PreExec 为计时起点");
    assert!(
        (2_900..=3_100).contains(&done.duration_ms),
        "应只计算命令执行阶段的 3 秒，实测 {:?}",
        done.duration_ms
    );
}

#[test]
fn tracker_measures_prompt_to_prompt_when_only_ab_markers() {
    let mut tracker = CommandTracker::default();
    let t0 = Instant::now();
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
    assert_eq!(tracker.on_event(&OscEvent::PromptStart, t0), None);
}

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
    scanner.feed(b"\x1b]133;A");
    assert!(scanner.feed(b"plain output").is_empty());
    assert!(scanner.feed(b"\x07").is_empty());
    assert_eq!(
        scanner.feed(b"\x1b]133;B\x07"),
        vec![OscEvent::CommandStart]
    );
}

/// Shell 集成握手（`OSC 777;transom;…`）与桌面通知共用 777 段，靠首段关键字分流。
#[test]
fn parses_shell_integration_handshake() {
    assert_eq!(
        collect(&[b"\x1b]777;transom;shell=/usr/bin/zsh\x07"]),
        vec![OscEvent::ShellReport {
            shell: "/usr/bin/zsh".to_string()
        }]
    );
    assert_eq!(
        collect(&[b"\x1b]777;transom;ready=bash\x1b\\"]),
        vec![OscEvent::IntegrationReady {
            shell: "bash".to_string()
        }]
    );
    // 空值仍是一条有效回报：它证明探针确实被执行了，与"没有回报"是两种结论。
    assert_eq!(
        collect(&[b"\x1b]777;transom;shell=\x07"]),
        vec![OscEvent::ShellReport {
            shell: String::new()
        }]
    );
}

/// 回报可能跨读缓冲到达，半条序列不能被误判成通知或丢弃后丢事件。
#[test]
fn handshake_survives_chunk_boundary() {
    assert_eq!(
        collect(&[b"\x1b]777;transom;sh", b"ell=/bin/bash\x07"]),
        vec![OscEvent::ShellReport {
            shell: "/bin/bash".to_string()
        }]
    );
}

/// 未知握手子命令与真实通知都不能串台。
#[test]
fn handshake_does_not_shadow_notify() {
    assert!(collect(&[b"\x1b]777;transom;bogus=1\x07"]).is_empty());
    let notify = format!("\x1b]777;notify;{};{}\x07", "构建完成", "目标已生成");
    assert_eq!(
        collect(&[notify.as_bytes()]),
        vec![OscEvent::Notify {
            title: "构建完成".to_string(),
            body: "目标已生成".to_string()
        }]
    );
}
