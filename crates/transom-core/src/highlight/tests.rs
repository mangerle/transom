//! 关键词高亮模块单元测试。

use super::engine::CompiledHighlight;
use super::types::{HighlightKind, HighlightSpan, TerminalHighlight};

fn config(keywords: &[&str]) -> TerminalHighlight {
    TerminalHighlight {
        enabled: true,
        keywords: keywords.iter().map(|k| (*k).to_string()).collect(),
    }
}

fn scan(text: &str, keywords: &[&str]) -> Vec<HighlightSpan> {
    let config = config(keywords);
    CompiledHighlight::compile(&config).scan(text, &config)
}

fn kinds(text: &str, keywords: &[&str]) -> Vec<HighlightKind> {
    scan(text, keywords)
        .into_iter()
        .map(|span| span.kind)
        .collect()
}

fn spans_of(text: &str, keywords: &[&str]) -> Vec<(usize, usize, HighlightKind)> {
    scan(text, keywords)
        .into_iter()
        .map(|span| (span.start, span.end, span.kind))
        .collect()
}

#[test]
fn highlights_log_levels() {
    assert_eq!(kinds("ERROR boom", &[]), vec![HighlightKind::Error]);
    assert_eq!(kinds("[WARN] slow", &[]), vec![HighlightKind::Warning]);
    assert_eq!(kinds("WARNING: low", &[]), vec![HighlightKind::Warning]);
    assert_eq!(
        kinds("info started", &[]),
        Vec::<HighlightKind>::new(),
        "小写 info 属于正文"
    );
    assert_eq!(
        kinds("DEBUG=1 INFO=2 FATAL", &[]),
        vec![
            HighlightKind::Debug,
            HighlightKind::Info,
            HighlightKind::Error
        ]
    );
}

#[test]
fn respects_word_boundaries() {
    assert_eq!(kinds("INFORMATION", &[]), Vec::<HighlightKind>::new());
    assert_eq!(kinds("ERRORS happen", &[]), Vec::<HighlightKind>::new());
    assert_eq!(kinds("MY_ERROR=1", &[]), Vec::<HighlightKind>::new());
    assert_eq!(kinds("(ERROR)", &[]), vec![HighlightKind::Error]);
    assert_eq!(kinds("level:ERROR", &[]), vec![HighlightKind::Error]);
}

#[test]
fn highlights_ipv4_only_when_valid() {
    assert_eq!(kinds("from 192.168.1.1 ok", &[]), vec![HighlightKind::Ipv4]);
    assert_eq!(kinds("0.0.0.0", &[]), vec![HighlightKind::Ipv4]);
    assert_eq!(kinds("255.255.255.255", &[]), vec![HighlightKind::Ipv4]);
    assert_eq!(
        kinds("256.1.1.1", &[]),
        Vec::<HighlightKind>::new(),
        "段值越界不算地址"
    );
    assert_eq!(
        kinds("1.2.3", &[]),
        Vec::<HighlightKind>::new(),
        "三段不算地址"
    );
    assert_eq!(
        kinds("1.2.3.4.5", &[]),
        Vec::<HighlightKind>::new(),
        "五段版本串不能只染前四段"
    );
    assert_eq!(
        kinds("v1.2.3.4", &[]),
        Vec::<HighlightKind>::new(),
        "前面紧贴版本号前缀时不算地址"
    );
}

#[test]
fn highlights_uuid() {
    assert_eq!(
        kinds("id=6f1c2b3a-4d5e-4f60-7a8b-9c0d1e2f3a4b", &[]),
        vec![HighlightKind::Uuid]
    );
    assert_eq!(
        kinds("6F1C2B3A-4D5E-4F60-7A8B-9C0D1E2F3A4B", &[]),
        vec![HighlightKind::Uuid],
        "大写十六进制同样命中"
    );
    assert_eq!(
        kinds("6f1c2b3a-4d5e-4f60-7a8b-9c0d1e2f3a4", &[]),
        Vec::<HighlightKind>::new(),
        "最后一段少一位不算 UUID"
    );
}

#[test]
fn spans_are_ordered_and_non_overlapping() {
    let found = spans_of(
        "ERROR at 10.0.0.1 id 6f1c2b3a-4d5e-4f60-7a8b-9c0d1e2f3a4b",
        &[],
    );
    assert_eq!(found.len(), 3);
    assert_eq!(found[0].1, 5, "ERROR 占 0..5");
    assert!(found[0].1 <= found[1].0, "区间必须升序且不重叠");
    assert!(found[1].1 <= found[2].0);
}

#[test]
fn counts_characters_not_bytes() {
    let found = spans_of("服务 ERROR 已发", &[]);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].0, 3, "三个汉字后是第 3 个字符");
    assert_eq!(found[0].1, 8);
}

#[test]
fn custom_keywords_are_case_insensitive() {
    assert_eq!(
        kinds("connection refused", &["refused"]),
        vec![HighlightKind::Custom]
    );
    assert_eq!(kinds("REFUSED", &["refused"]), vec![HighlightKind::Custom]);
    assert_eq!(
        kinds("refusedly", &["refused"]),
        Vec::<HighlightKind>::new()
    );
    assert_eq!(
        kinds("磁盘空间不足", &["空间不足"]),
        vec![HighlightKind::Custom]
    );
}

#[test]
fn preset_wins_over_custom_at_same_position() {
    assert_eq!(kinds("ERROR", &["error"]), vec![HighlightKind::Error]);
}

#[test]
fn disabled_config_scans_nothing() {
    let config = TerminalHighlight {
        enabled: false,
        keywords: vec!["ERROR".into()],
    };
    let compiled = CompiledHighlight::compile(&config);
    assert!(compiled.scan("ERROR 1.2.3.4", &config).is_empty());
    assert!(
        compiled.keywords.is_empty(),
        "关闭状态下不该编译出任何关键词"
    );
    assert!(scan("", &[]).is_empty());
}

#[test]
fn parses_keywords_across_chinese_separators() {
    assert_eq!(
        TerminalHighlight::parse_keywords("timeout, refused、连接失败;oom  killed"),
        vec!["timeout", "refused", "连接失败", "oom", "killed"]
    );
    assert!(TerminalHighlight::parse_keywords("  , ， 、 ;;  ").is_empty());
    let highlight = TerminalHighlight {
        enabled: true,
        keywords: vec!["oom".into(), "killed".into()],
    };
    assert_eq!(highlight.keywords_joined(), "oom, killed");
}

#[test]
fn sanitize_drops_blank_dedupes_and_caps() {
    let raw = TerminalHighlight {
        enabled: true,
        keywords: vec![
            "  warn ".into(),
            "WARN".into(),
            "".into(),
            "   ".into(),
            "x".repeat(41),
            "disk".into(),
        ],
    };
    let cleaned = raw.sanitized();
    assert_eq!(
        cleaned.keywords,
        vec!["warn", "disk"],
        "去空白、不分大小写去重、丢超长项"
    );
    let many = TerminalHighlight {
        enabled: true,
        keywords: (0..60).map(|index| format!("k{index}")).collect(),
    };
    assert_eq!(many.sanitized().keywords.len(), 32, "条数上限必须生效");
}

#[test]
fn ansi_indices_stay_inside_palette() {
    for kind in [
        HighlightKind::Error,
        HighlightKind::Warning,
        HighlightKind::Info,
        HighlightKind::Debug,
        HighlightKind::Ipv4,
        HighlightKind::Uuid,
        HighlightKind::Custom,
    ] {
        assert!((0..16).contains(&kind.ansi_index()), "{kind:?}");
    }
}
