//! OSC 扫描与命令追踪单元测试。

use super::scanner::OscScanner;
use super::types::{CommandTracker, MAX_PAYLOAD_BYTES, OscEvent, TrackedCommand};
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
