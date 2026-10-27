//! 终端检索单元测试。

use super::types::{Hit, SearchState, clamp_span, escape_literal, prefill_from_selection};
use alacritty_terminal::index::{Column, Line, Point};
use std::time::{Duration, Instant};

#[test]
fn escapes_regex_metachars_for_keyword_mode() {
    assert_eq!(escape_literal("a.b"), "a\\.b");
    assert_eq!(escape_literal("[1+1]"), "\\[1\\+1\\]");
    assert_eq!(escape_literal("中文 (x)"), "中文 \\(x\\)");
    // 非 ASCII 标点不参与转义，避免把全角符号写成非法转义序列；`%` 也不是正则元字符。
    assert_eq!(escape_literal("价格：100%"), "价格：100%");
}

#[test]
fn clamps_spans_outside_viewport() {
    assert_eq!(clamp_span(0, 5, 10), Some(5));
    assert_eq!(clamp_span(8, 20, 10), Some(10));
    assert_eq!(clamp_span(10, 12, 10), None);
    assert_eq!(clamp_span(4, 4, 10), None);
}

#[test]
fn ticks_bucket_by_pixel_row() {
    let hits = vec![
        Hit {
            start: Point::new(Line(-9), Column(0)),
            end: Point::new(Line(-9), Column(3)),
        },
        Hit {
            start: Point::new(Line(-8), Column(0)),
            end: Point::new(Line(-8), Column(3)),
        },
    ];
    // 10 行缓冲、2 个命中落在同一像素桶时只保留一条刻度。
    let ticks = crate::search_ticks::build_ticks(&hits, Line(-9), Line(0), 4);
    assert_eq!(ticks.len(), 1);
    let many = crate::search_ticks::build_ticks(&hits, Line(-9), Line(0), 100);
    assert_eq!(many.len(), 2);
}

#[test]
fn empty_tick_rows_yields_no_ticks() {
    let hits = vec![Hit {
        start: Point::new(Line(0), Column(0)),
        end: Point::new(Line(0), Column(1)),
    }];
    assert!(crate::search_ticks::build_ticks(&hits, Line(-1), Line(0), 0).is_empty());
    assert!(crate::search_ticks::build_ticks(&hits, Line(0), Line(0), 10).is_empty());
}

#[test]
fn invalid_regex_is_reported_not_panicked() {
    let mut state = SearchState::default();
    state.set_query("(", true);
    assert!(!state.is_active());
    assert!(state.status(0).error.is_some());
    state.set_query("a(", false);
    assert!(state.is_active());
    assert!(state.status(0).error.is_none());
}

#[test]
fn query_change_forces_rescan_and_invalidation_waits_for_quiet() {
    let mut state = SearchState::default();
    let t0 = Instant::now();
    state.set_query("abc", false);
    // 新查询立刻放行，不必等冷却或输出停歇。
    assert!(state.scan_allowed(0, t0, false));
    state.record_scan(
        0,
        vec![Hit {
            start: Point::new(Line(0), Column(0)),
            end: Point::new(Line(0), Column(2)),
        }],
        false,
        t0,
    );
    // 同一纪元内不重复扫描。
    assert!(!state.scan_allowed(0, t0, true));
    // 缓冲区变化后：输出未停歇不扫。
    assert!(state.is_stale(1));
    assert!(!state.scan_allowed(1, t0, false));
    // 停歇了但仍在冷却期内也不扫。
    assert!(!state.scan_allowed(1, t0, true));
    // 过了冷却且停歇才放行。
    assert!(state.scan_allowed(1, t0 + Duration::from_millis(400), true));
    // 改查询会清空命中并重新立即放行。
    state.set_query("abcd", false);
    assert_eq!(state.status(1).total, 0);
    assert!(state.scan_allowed(1, t0, false));
}

#[test]
fn cursor_wraps_in_both_directions() {
    let mut state = SearchState::default();
    state.set_query("x", false);
    let hits: Vec<Hit> = (0..3)
        .map(|index| Hit {
            start: Point::new(Line(index), Column(0)),
            end: Point::new(Line(index), Column(1)),
        })
        .collect();
    state.record_scan(0, hits, false, Instant::now());
    assert_eq!(state.select_next().unwrap().start.line, Line(0));
    assert_eq!(state.select_next().unwrap().start.line, Line(1));
    assert_eq!(state.select_next().unwrap().start.line, Line(2));
    assert_eq!(state.select_next().unwrap().start.line, Line(0));
    assert_eq!(state.select_prev().unwrap().start.line, Line(2));
    assert_eq!(state.ordinal(), 3);
    assert_eq!(state.status(0).total, 3);
}

#[test]
fn empty_hit_list_never_panics() {
    let mut state = SearchState::default();
    state.set_query("nothing", false);
    state.record_scan(0, Vec::new(), false, Instant::now());
    assert!(state.select_next().is_none());
    assert!(state.select_prev().is_none());
    assert!(state.select_by_fraction(0.5, Line(-5), Line(0)).is_none());
    assert_eq!(state.ordinal(), 0);
    let paint = state.paint(0, 4, 10, Line(-5), Line(3), 20);
    assert_eq!(paint.rows.len(), 4);
    assert!(paint.ticks.is_empty());
}

#[test]
fn fraction_selection_picks_nearest_hit() {
    let mut state = SearchState::default();
    state.set_query("x", false);
    let hits = vec![
        Hit {
            start: Point::new(Line(-9), Column(0)),
            end: Point::new(Line(-9), Column(1)),
        },
        Hit {
            start: Point::new(Line(-1), Column(0)),
            end: Point::new(Line(-1), Column(1)),
        },
    ];
    state.record_scan(0, hits, false, Instant::now());
    assert_eq!(
        state
            .select_by_fraction(0.0, Line(-9), Line(0))
            .unwrap()
            .start
            .line,
        Line(-9)
    );
    assert_eq!(
        state
            .select_by_fraction(1.5, Line(-9), Line(0))
            .unwrap()
            .start
            .line,
        Line(-1)
    );
}

#[test]
fn prefill_takes_trimmed_first_line_only() {
    assert_eq!(
        prefill_from_selection(Some("  hello  ")).as_deref(),
        Some("hello")
    );
    // 多行选区只取首行：跨行整块匹配不是检索栏的目标，静默取首行比清空更好用。
    assert_eq!(
        prefill_from_selection(Some("first\nsecond")).as_deref(),
        Some("first")
    );
    assert_eq!(prefill_from_selection(Some("   ")), None);
    assert_eq!(prefill_from_selection(None), None);
    // 超长选区直接不预填，避免把一大段缓冲区当成查询词。
    let long = "x".repeat(super::types::PREFILL_MAX_CHARS + 1);
    assert_eq!(prefill_from_selection(Some(&long)), None);
}

#[test]
fn paint_maps_hits_to_viewport_rows() {
    let mut state = SearchState::default();
    state.set_query("ab", false);
    let hits = vec![
        // 向上滚动 8 行后可见区是网格行 -8..-6。
        Hit {
            start: Point::new(Line(-9), Column(0)),
            end: Point::new(Line(-9), Column(1)),
        },
        Hit {
            start: Point::new(Line(-8), Column(2)),
            end: Point::new(Line(-8), Column(3)),
        },
        // 跨行命中：首行从列 5 到行尾，次行从列 0 到列 1。
        Hit {
            start: Point::new(Line(-7), Column(5)),
            end: Point::new(Line(-6), Column(1)),
        },
    ];
    state.record_scan(0, hits, true, Instant::now());
    // 走两次把游标停在第二个命中上，验证当前命中与普通命中分层绘制。
    state.select_next();
    state.select_next();
    let paint = state.paint(8, 3, 10, Line(-9), Line(-6), 12);
    let status = state.status(0);
    assert_eq!(status.total, 3);
    assert_eq!(status.ordinal, 2);
    assert!(status.truncated);
    // 视口外的历史命中不参与绘制。
    assert!(paint.rows[0].hits.is_empty());
    assert_eq!(paint.rows[0].current, Some((2, 4)));
    assert_eq!(paint.rows[1].hits, vec![(5, 10)]);
    assert_eq!(paint.rows[2].hits, vec![(0, 2)]);
    assert!(!paint.ticks.is_empty());
}
