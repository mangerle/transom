use super::*;
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::SelectionType;
use std::time::Duration;
use transom_core::BlockViewSettings;

/// shell 集成只给边界标记（绝大多数 bash/zsh 配置）时，命令文本按
/// "提示符结束点到光标"的网格快照补采。
#[test]
fn captures_command_from_grid_between_prompt_and_cursor() {
    let (mut engine, _events) = engine(40, 3);
    engine.process(b"\x1b]133;A\x1b\\user@host:~$ \x1b]133;B\x1b\\");
    engine.process(b"git status");
    engine.process(b"\x1b]133;C\x1b\\");
    assert_eq!(
        captured(&engine),
        vec!["git status".to_string()],
        "应采到用户敲下的整行且不含提示符"
    );
}

/// 集成把命令文本直接上报（WezTerm 的 `E;<cmd>`）时以它为准：
/// 网格快照可能被折行与重绘影响，上报值不会。
#[test]
fn prefers_command_text_reported_by_shell_integration() {
    let (mut engine, _events) = engine(40, 3);
    engine.process(b"\x1b]133;A\x1b\\$ \x1b]133;B\x1b\\");
    engine.process(b"echo hi\x1b]133;E;echo hi > /tmp/x\x1b\\");
    assert_eq!(
        captured(&engine),
        vec!["echo hi > /tmp/x".to_string()],
        "带参上报优先于网格快照"
    );
}

/// 单字符、以空格开头的行（`HIST_IGNORE_SPACE` 约定）都不该进历史。
#[test]
fn skips_short_and_opted_out_commands() {
    let (mut engine, _events) = engine(40, 3);
    engine.process(b"\x1b]133;A\x1b\\$ \x1b]133;B\x1b\\");
    engine.process(b"x\x1b]133;C\x1b\\");
    engine.process(b"\x1b]133;A\x1b\\$ \x1b]133;B\x1b\\");
    engine.process(b" secret-token abc\x1b]133;C\x1b\\");
    assert!(captured(&engine).is_empty(), "短行与刻意不留档的行都不记录");
}

/// 没有 shell 集成（从不发 `B`）时不能凭空产出历史：起点未知就放弃。
#[test]
fn captures_nothing_without_prompt_marker() {
    let (mut engine, _events) = engine(40, 3);
    engine.process(b"plain output without integration\x1b]133;C\x1b\\");
    assert!(captured(&engine).is_empty());
}

/// 一秒钟内跑完的小命令与提示符常挤在同一批字节里：按 OSC 字节偏移
/// 分段推进后，命令文本、输出行与退出码都必须完整落在同一条块上。
#[test]
fn block_captures_fast_command_in_one_chunk() {
    let (mut engine, _events) = engine(40, 10);
    engine.process(
            b"\x1b]133;A\x1b\\$ \x1b]133;B\x1b\\echo hi\r\n\x1b]133;C\x1b\\hi\r\n\x1b]133;D;0;12\x1b\\\x1b]133;A\x1b\\$ ",
        );
    let blocks: Vec<_> = engine.command_blocks().cloned().collect();
    assert_eq!(blocks.len(), 1, "应采集到一条命令块");
    let block = &blocks[0];
    assert_eq!(block.command_text, "echo hi");
    assert_eq!(block.exit_code, Some(0));
    assert_eq!(block.duration_ms, Some(12));
    assert!(block.output_rows() >= 1, "输出至少包含 hi 所在行");
    let output = engine.block_output_text(block.id).expect("块输出应可读取");
    assert!(output.contains("hi"), "输出: {output:?}");
    let markdown = engine.block_markdown(block.id).expect("Markdown 应可生成");
    assert!(markdown.starts_with("```console\n$ echo hi"));
    assert!(markdown.contains("退出码 0"));
}

/// 折叠切换与视图开关不改变网格内容，但必须让下一帧全量重建行缓存；
/// 否则块视图会沿用旧的行映射，把内容画到错误位置。
#[test]
fn block_layout_change_forces_full_damage() {
    let (mut engine, _events) = engine(40, 10);
    engine.process(
            b"\x1b]133;A\x1b\\$ \x1b]133;B\x1b\\seq 3\r\n\x1b]133;C\x1b\\1\r\n2\r\n3\r\n\x1b]133;D;0\x1b\\\x1b]133;A\x1b\\$ ",
        );
    let id = engine
        .command_blocks()
        .next()
        .map(|block| block.id)
        .expect("命令块");
    let _ = engine.take_damage();

    engine.set_block_auto_collapse(1);
    assert!(
        matches!(engine.take_damage(), DamageInfo::Full),
        "折叠阈值变化必须触发全量重建"
    );

    let _ = engine.take_damage();
    assert!(engine.toggle_block(id));
    assert!(
        matches!(engine.take_damage(), DamageInfo::Full),
        "折叠切换必须触发全量重建"
    );

    let _ = engine.take_damage();
    engine.mark_block_layout_dirty();
    assert!(
        matches!(engine.take_damage(), DamageInfo::Full),
        "视图开关必须触发全量重建"
    );
}

/// 自动折叠阈值在收尾结算时生效，布局把输出压成横幅。
#[test]
fn block_auto_collapses_and_layout_shows_banner() {
    let (mut engine, _events) = engine(40, 10);
    engine.set_block_auto_collapse(1);
    engine.process(
            b"\x1b]133;A\x1b\\$ \x1b]133;B\x1b\\seq 3\r\n\x1b]133;C\x1b\\1\r\n2\r\n3\r\n\x1b]133;D;0\x1b\\\x1b]133;A\x1b\\$ ",
        );
    let block = engine.command_blocks().next().cloned().expect("命令块");
    assert!(block.collapsed, "3 行输出应超过阈值 1 自动折叠");
    let settings = BlockViewSettings {
        enabled: true,
        style: transom_core::BlockViewStyle::Card,
        auto_collapse_rows: 1,
        shell_integration: true,
    };
    let layout = engine
        .build_block_layout(Some(&settings))
        .expect("启用后应有布局");
    assert!(
        layout
            .slots()
            .iter()
            .any(|slot| matches!(slot, crate::block::VisualSlot::Banner { .. })),
        "折叠块必须渲染为横幅"
    );
    // 关闭块视图后回到经典连续流。
    let disabled = BlockViewSettings {
        enabled: false,
        ..settings
    };
    assert!(engine.build_block_layout(Some(&disabled)).is_none());
}

/// Kitty 图像按当前光标位置锚定网格行，删除指令能清掉全部放置。
#[test]
fn kitty_image_is_stored_and_placed_at_cursor() {
    let (mut engine, _events) = engine(20, 4);
    // 光标移到第 2 行第 3 列（0 基）。
    engine.process(b"\x1b[2;3H");
    // 1x1 纯红 RGB，base64 为 /wAA。
    engine.process(b"\x1b_Ga=T,f=24,s=1,v=1,i=5;/wAA\x1b\\");
    let placements = engine.graphics_placements();
    assert_eq!(placements.len(), 1, "图像应有一次放置");
    assert_eq!(placements[0].line, Line(1));
    assert_eq!(placements[0].col, 2);
    assert_eq!((placements[0].width, placements[0].height), (1, 1));

    // 删除全部图像后放置必须消失。
    engine.process(b"\x1b_Ga=d,d=a;\x1b\\");
    assert!(engine.graphics_placements().is_empty());
}

fn captured(engine: &TerminalEngine) -> Vec<String> {
    engine
        .osc_events
        .iter()
        .filter_map(|event| match event {
            OscEvent::CommandRecorded { command } => Some(command.clone()),
            _ => None,
        })
        .collect()
}
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::vte::ansi::{Color, NamedColor};

/// 用指定 profile 建引擎；分隔符行为测试需要非默认配置。
fn engine_with_profile(
    profile: TerminalProfile,
) -> (
    TerminalEngine,
    flume::Receiver<alacritty_terminal::event::Event>,
) {
    let (tx, rx) = flume::bounded(16);
    (
        TerminalEngine::new(TerminalSize::new(20, 4), &profile, tx),
        rx,
    )
}

fn engine(
    cols: u16,
    rows: u16,
) -> (
    TerminalEngine,
    flume::Receiver<alacritty_terminal::event::Event>,
) {
    let (tx, rx) = flume::bounded(16);
    let profile = TerminalProfile::default();
    (
        TerminalEngine::new(TerminalSize::new(cols, rows), &profile, tx),
        rx,
    )
}

/// 损伤跟踪的取走-重置契约：脏行渲染依赖"取走即重置、新输入必损伤"。
#[test]
fn take_damage_reports_changes_and_resets() {
    let (mut engine, _rx) = engine(20, 4);
    // 初始状态必然全量损伤：首帧必须完整绘制。
    assert!(matches!(engine.take_damage(), DamageInfo::Full));
    // 无新输入时只剩光标行损伤（内核恒定标记当前光标）。
    match engine.take_damage() {
        DamageInfo::Rows(rows) => {
            assert!(!rows.is_empty(), "光标行应保持损伤");
            assert!(rows.len() <= 4, "视口外行号不应出现");
        }
        other => panic!("静止网格不应报全量损伤: {other:?}"),
    }
    // 新输入必须重新损伤其所在行。
    engine.process(b"hi");
    match engine.take_damage() {
        DamageInfo::Rows(rows) => assert!(rows.contains(&0), "新输入行应损伤: {rows:?}"),
        DamageInfo::Full => {}
    }
    // resize 折行重排后必须回到全量。
    engine.resize(TerminalSize::new(10, 4));
    assert!(matches!(engine.take_damage(), DamageInfo::Full));
}

/// OSC 8：链接元数据进入单元格，渲染与点击打开都以此为准。
#[test]
fn hyperlink_at_returns_osc8_uri() {
    let (mut engine, _rx) = engine(20, 4);
    engine.process(b"\x1b]8;;https://example.com\x1b\\home\x1b]8;;\x1b\\plain");
    let uri = engine
        .hyperlink_at(Point::new(Line(0), Column(0)))
        .expect("链接文本的首格应携带 URI");
    assert_eq!(uri, "https://example.com");
    assert!(
        engine
            .hyperlink_at(Point::new(Line(0), Column(6)))
            .is_none(),
        "链接关闭后的文本不应携带 URI"
    );
}

/// 分隔符设置必须真的改变双击选词边界。
#[test]
fn word_separators_drive_semantic_selection() {
    let (mut engine, _events) = engine_with_profile(TerminalProfile::default());
    engine.process(b"aa-bb cc");
    engine.start_selection(
        SelectionType::Semantic,
        Point::new(Line(0), Column(4)),
        Side::Left,
    );
    assert_eq!(
        engine.selection_text().as_deref(),
        Some("aa-bb"),
        "默认分隔符不含连字符，应整段选中"
    );

    let dashed = TerminalProfile {
        word_separators: "- \t".into(),
        ..TerminalProfile::default()
    };
    let (mut engine, _events) = engine_with_profile(dashed);
    engine.process(b"aa-bb cc");
    engine.start_selection(
        SelectionType::Semantic,
        Point::new(Line(0), Column(4)),
        Side::Left,
    );
    assert_eq!(
        engine.selection_text().as_deref(),
        Some("bb"),
        "把连字符与空格列为分隔符后只应选中后半段"
    );
}

/// 默认分隔符必须与仿真内核常量逐字一致，否则不改设置时的双击行为就会悄悄漂移。
#[test]
fn default_word_separators_match_kernel_constant() {
    assert_eq!(
        TerminalProfile::default().word_separators,
        alacritty_terminal::term::SEMANTIC_ESCAPE_CHARS
    );
}

#[test]
fn processes_plain_text() {
    let (mut engine, _events) = engine(20, 4);
    engine.process(b"hello\r\nworld");
    let grid = engine.grid();
    assert_eq!(grid[Point::new(Line(0), Column(0))].c, 'h');
    assert_eq!(grid[Point::new(Line(0), Column(4))].c, 'o');
    assert_eq!(grid[Point::new(Line(1), Column(0))].c, 'w');
    assert_eq!(engine.cursor_point(), Point::new(Line(1), Column(5)));
}

#[test]
fn parses_sgr_and_wide_char_cells() {
    let (mut engine, _events) = engine(20, 4);
    engine.process(b"\x1b[31mX");
    engine.process("中".as_bytes());
    engine.process(b"a");

    let grid = engine.grid();
    let x = &grid[Point::new(Line(0), Column(0))];
    assert_eq!(x.c, 'X');
    assert_eq!(x.fg, Color::Named(NamedColor::Red));

    let wide = &grid[Point::new(Line(0), Column(1))];
    assert_eq!(wide.c, '中');
    assert!(wide.flags.contains(Flags::WIDE_CHAR));
    let spacer = &grid[Point::new(Line(0), Column(2))];
    assert!(spacer.flags.contains(Flags::WIDE_CHAR_SPACER));
    assert_eq!(grid[Point::new(Line(0), Column(3))].c, 'a');
}

#[test]
fn resize_and_scrollback() {
    let (mut engine, _events) = engine(10, 2);
    engine.process(b"one\r\ntwo\r\nthree\r\nfour\r\nfive");
    engine.resize(TerminalSize::new(20, 3));
    assert_eq!(engine.size(), TerminalSize::new(20, 3));
    assert_eq!(engine.grid().columns(), 20);
    assert_eq!(engine.grid().screen_lines(), 3);

    engine.scroll_display(Scroll::Top);
    assert!(engine.display_offset() > 0);
    engine.scroll_to_bottom();
    assert_eq!(engine.display_offset(), 0);
}

#[test]
fn default_cursor_style_follows_profile() {
    let (tx, _rx) = flume::bounded(16);
    let profile = TerminalProfile {
        cursor_shape: TerminalCursorShape::Bar,
        cursor_blink: false,
        ..TerminalProfile::default()
    };
    let engine = TerminalEngine::new(TerminalSize::new(20, 4), &profile, tx);
    let style = engine.term().cursor_style();
    assert_eq!(style.shape, AlacCursorShape::Beam);
    assert!(!style.blinking);

    let (tx, _rx) = flume::bounded(16);
    let profile = TerminalProfile {
        cursor_shape: TerminalCursorShape::Underline,
        cursor_blink: true,
        ..TerminalProfile::default()
    };
    let engine = TerminalEngine::new(TerminalSize::new(20, 4), &profile, tx);
    let style = engine.term().cursor_style();
    assert_eq!(style.shape, AlacCursorShape::Underline);
    assert!(style.blinking);
}

#[test]
fn selection_returns_text() {
    let (mut engine, _events) = engine(20, 4);
    engine.process(b"hello world");
    engine.start_selection(
        SelectionType::Simple,
        Point::new(Line(0), Column(0)),
        Side::Left,
    );
    engine.update_selection(Point::new(Line(0), Column(4)), Side::Right);
    assert_eq!(engine.selection_text().as_deref(), Some("hello"));
}

/// 矩形列选的价值就在于"只抠每行的同一几列"，因此选区文本必须逐行截取到列区间。
#[test]
fn block_selection_extracts_only_the_column_range() {
    let (mut engine, _events) = engine(20, 4);
    engine.process(b"aaaaBBBBaaaa\r\nccccBBBBdddd\r\neeeeBBBBffff");
    engine.start_selection(
        SelectionType::Block,
        Point::new(Line(0), Column(4)),
        Side::Left,
    );
    engine.update_selection(Point::new(Line(2), Column(7)), Side::Right);
    let range = engine.selection_range().expect("块选应有选区");
    assert!(range.is_block, "选区必须标记为块，渲染层据此逐行截取");
    assert_eq!(engine.selection_text().as_deref(), Some("BBBB\nBBBB\nBBBB"));
    // 非矩形选择完全不受补齐影响。
    engine.start_selection(
        SelectionType::Simple,
        Point::new(Line(0), Column(4)),
        Side::Left,
    );
    engine.update_selection(Point::new(Line(0), Column(7)), Side::Right);
    assert_eq!(engine.selection_text().as_deref(), Some("BBBB"));
}

/// 短行落在列区间之外时必须补空格，否则抠出来的各列会左移错位。
#[test]
fn block_selection_pads_short_rows() {
    let (mut engine, _events) = engine(20, 4);
    engine.process(b"aaaaBBBBaaaa\r\ncccc\r\neeeeBBBBffff");
    engine.start_selection(
        SelectionType::Block,
        Point::new(Line(0), Column(4)),
        Side::Left,
    );
    engine.update_selection(Point::new(Line(2), Column(7)), Side::Right);
    assert_eq!(engine.selection_text().as_deref(), Some("BBBB\n    \nBBBB"));
}

/// 锁定检索赖以成立的行号语义：`Line` 相对视口底部编号，新行滚入会让同一条内容
/// 的行号整体 -1，因此命中快照必须随缓冲区写入失效，而不是平移复用。
#[test]
fn line_indices_are_relative_to_viewport_bottom() {
    let (mut engine, _events) = engine(20, 2);
    engine.process(b"A\r\nB\r\nC");
    assert_eq!(engine.grid()[Line(-1)][Column(0)].c, 'A');
    assert_eq!(engine.grid()[Line(0)][Column(0)].c, 'B');
    assert_eq!(engine.grid()[Line(1)][Column(0)].c, 'C');

    // 再滚入一行：同样的内容整体下移一个行号，A 退到更深的位置。
    engine.process(b"\r\nD");
    assert_eq!(engine.grid()[Line(-2)][Column(0)].c, 'A');
    assert_eq!(engine.grid()[Line(-1)][Column(0)].c, 'B');
    assert_eq!(engine.grid()[Line(0)][Column(0)].c, 'C');
    assert_eq!(engine.grid()[Line(1)][Column(0)].c, 'D');
}

/// 关键词模式按字面量匹配，正则模式按表达式匹配；中文与宽字符同样可检索。
#[test]
fn searches_literal_and_regex_queries() {
    let (mut engine, _events) = engine(20, 4);
    engine.process("alpha.bravo\r\n中文 错误码 404\r\ngamma".as_bytes());
    let now = Instant::now();

    engine.set_search_query("br.vo", false);
    engine.refresh_search(now);
    assert_eq!(
        engine.search_status().total,
        0,
        "字面量 br.vo 不应命中 bravo"
    );

    engine.set_search_query("br.vo", true);
    engine.refresh_search(now);
    assert_eq!(engine.search_status().total, 1, "正则 br.vo 应命中 bravo");

    engine.set_search_query("错误", false);
    engine.refresh_search(now);
    let hit = engine.search_select(true).expect("应命中中文关键词");
    assert_eq!(hit.start.line, Line(1));

    engine.set_search_query("\\d+", true);
    engine.refresh_search(now);
    assert_eq!(engine.search_status().total, 1);
}

/// 游标在首尾都要回绕，且当前命中变化后视线要跟到可见区。
#[test]
fn cursor_wraps_and_scrolls_history_into_view() {
    let (mut engine, _events) = engine(20, 2);
    engine.process(b"mark one\r\nfiller a\r\nfiller b\r\nmark two");
    engine.set_search_query("mark", false);
    engine.refresh_search(Instant::now());
    assert_eq!(engine.search_status().total, 2);

    // 命中按行号升序：第一条是已滚入历史的 `mark one`。
    let oldest = engine.search_select(true).unwrap();
    assert_eq!(oldest.start.line, Line(-2));
    assert!(
        engine.display_offset() > 0,
        "历史命中必须把显示偏移推上去才看得见"
    );

    let newest = engine.search_select(true).unwrap();
    assert_eq!(newest.start.line, Line(1));
    assert_eq!(engine.display_offset(), 0, "屏内命中不应再滚动");

    let wrapped = engine.search_select(true).unwrap();
    assert_eq!(wrapped, oldest, "到末尾应回绕到第一个");
    let back = engine.search_select(false).unwrap();
    assert_eq!(back, newest, "反向越过头部应回绕到最后一个");
}

/// 新写入使快照失效：失效期间不绘制高亮，冷却期内也不重复全缓冲扫描。
#[test]
fn output_invalidates_snapshot_until_rescan_cooldown_allows() {
    let (mut engine, _events) = engine(20, 4);
    engine.process(b"needle in haystack");
    let t0 = Instant::now();
    engine.set_search_query("needle", false);
    engine.refresh_search(t0);
    assert!(engine.search_paint(40).is_some());

    engine.process(b"\r\nmore");
    // 输出仍在继续：既未停歇也在冷却期内，绝不重扫，因此不画任何可能指错位置的高亮。
    assert!(
        engine.search_paint(40).is_none(),
        "缓冲区变化后不得沿用旧行号"
    );
    engine.refresh_search(t0);
    assert!(engine.search_paint(40).is_none(), "冷却期内不重扫");
    engine.refresh_search(t0 + std::time::Duration::from_millis(400));
    assert!(engine.search_paint(40).is_some());
}

/// 非法正则只登记错误文本，不能影响既有渲染或让检索崩溃。
#[test]
fn invalid_query_keeps_engine_usable() {
    let (mut engine, _events) = engine(20, 4);
    engine.process(b"still works");
    engine.set_search_query("(unclosed", true);
    engine.refresh_search(Instant::now());
    let status = engine.search_status();
    assert!(status.error.is_some());
    assert!(engine.search_paint(40).is_none());
    assert!(engine.search_select(true).is_none());
    engine.clear_search();
    assert!(!engine.search_status().has_query);
}

/// 构造三条命令，退出码依次为 0/2/1，每条 4 行输出，制造足够历史供滚动与刻度断言。
fn three_commands(engine: &mut TerminalEngine) -> Vec<i64> {
    let mut prompts = Vec::with_capacity(3);
    for index in 0..3u32 {
        engine.process(
            format!(
                "\x1b]133;A\x1b\\$ cmd{index}\r\nout-a\r\n\x1b]133;C\x1b\\out-b\r\n\x1b]133;D;{index}\x1b\\"
            )
            .as_bytes(),
        );
        let block = engine
            .command_blocks()
            .last()
            .cloned()
            .expect("每条命令都应收尾成块");
        prompts.push(block.prompt_line);
    }
    engine.process(b"\x1b]133;A\x1b\\$ ");
    prompts
}

/// 只有非 0 退出码的命令进刻度带：成功命令不该被标成错误。
#[test]
fn failure_ticks_marks_only_nonzero_exits() {
    let (mut engine, _events) = engine(40, 3);
    three_commands(&mut engine);
    let ticks = engine.failure_ticks(64);
    assert_eq!(ticks.len(), 2, "退出码 2 与 1 各出一条刻度");
    assert!(
        ticks.iter().all(|f| *f >= 0.0 && *f <= 1.0),
        "刻度必须落在缓冲区比例区间内: {ticks:?}"
    );
}

/// 刻度位置要能反查回对应命令：点击刻度跳的就是那条命令的提示符行。
#[test]
fn failure_click_jumps_to_nearest_failed_prompt() {
    let (mut engine, _events) = engine(40, 3);
    let prompts = three_commands(&mut engine);
    // 取第二条失败命令（退出码 2）的刻度比例点击，应精确落到它的提示符行。
    let ticks = engine.failure_ticks(512);
    let fraction = *ticks.first().expect("应有失败刻度");
    assert!(
        engine.scroll_to_failure_near(fraction),
        "点击刻度必须真的移动视口"
    );
    assert_eq!(
        engine.current_display_abs_line(),
        prompts[1],
        "应跳到第一条失败命令的提示符"
    );
    // 贴回底部后再点一次最靠下的刻度，命中第二条失败命令。
    engine.scroll_to_bottom();
    let fraction = *ticks.last().expect("应有失败刻度");
    assert!(engine.scroll_to_failure_near(fraction));
    assert_eq!(engine.current_display_abs_line(), prompts[2]);
}

/// 没有任何失败命令时刻度为空，且点击不产生虚假的"跳转成功"。
#[test]
fn no_failures_yields_empty_ticks_and_no_jump() {
    let (mut engine, _events) = engine(40, 3);
    engine.process(
        b"\x1b]133;A\x1b\\$ ok\r\n\x1b]133;C\x1b\\done\r\n\x1b]133;D;0\x1b\\\x1b]133;A\x1b\\$ ",
    );
    assert!(engine.failure_ticks(64).is_empty());
    assert!(!engine.scroll_to_failure_near(0.5));
}

/// 滚动缓冲容量为 0 时块记录整体不可靠，失败标记必须跟着失效而不是画错行。
#[test]
fn saturated_buffer_drops_failure_marks() {
    let (tx, _rx) = flume::bounded(16);
    let profile = TerminalProfile {
        scrollback: 0,
        ..TerminalProfile::default()
    };
    let mut engine = TerminalEngine::new(TerminalSize::new(40, 3), &profile, tx);
    engine.process(b"\x1b]133;A\x1b\\$ bad\r\n\x1b]133;C\x1b\\boom\r\n\x1b]133;D;1\x1b\\");
    assert!(engine.failure_ticks(64).is_empty());
    assert!(!engine.scroll_to_failure_near(0.5));
    assert!(!engine.scroll_to_previous_prompt());
}

/// 贴在底部时"上一条"的基准是光标行而不是视口顶行：否则短缓冲里第一次按键就静默无效。
#[test]
fn prompt_navigation_walks_back_and_forth() {
    let (mut engine, _events) = engine(40, 3);
    three_commands(&mut engine);
    assert_eq!(engine.display_offset(), 0, "新命令输出后应贴底");
    assert!(
        engine.scroll_to_previous_prompt(),
        "应跳回最近一条已完成命令的提示符"
    );
    assert!(engine.display_offset() > 0, "跳转后视口必须真的移动");
    assert!(
        engine.scroll_to_next_prompt(),
        "已在最后一条提示符上再按下一条应回到实时提示符"
    );
    assert_eq!(engine.display_offset(), 0);
    assert!(
        !engine.scroll_to_next_prompt(),
        "已经贴底时不得谎报跳转成功"
    );
}

/// 反复按上一条要逐条回退，最后停在最早一条命令上而不是抖动。
#[test]
fn prompt_navigation_walks_through_every_block() {
    let (mut engine, _events) = engine(40, 3);
    let prompts = three_commands(&mut engine);
    let mut visited = Vec::new();
    for _ in 0..4 {
        if !engine.scroll_to_previous_prompt() {
            break;
        }
        visited.push(engine.current_display_abs_line());
    }
    assert_eq!(
        visited,
        vec![prompts[2], prompts[1], prompts[0]],
        "应逐条回退到最早一条命令后停止"
    );
}

/// 集成注入的时机判据：收到过输出、输出停歇、收到过 133 标记，都必须如实反映。
#[test]
fn integration_gates_report_real_session_state() {
    let (mut engine, _events) = engine(40, 3);
    assert!(!engine.output_started(), "没收到字节前不算已开始");
    assert!(!engine.output_idle(Instant::now(), Duration::from_millis(0)));
    assert!(!engine.shell_markers_seen());
    engine.process(b"\x1b]133;A\x1b\\$ ");
    assert!(engine.output_started());
    assert!(engine.shell_markers_seen(), "收到 A 即证明远端在发 133");
    assert!(engine.output_idle(Instant::now(), Duration::from_millis(0)));
    assert!(!engine.output_idle(Instant::now(), Duration::from_secs(60 * 60)));
}

/// 集成握手回报不参与命令边界采集：它没有网格位置，混进去会造出假块。
#[test]
fn handshake_markers_do_not_create_blocks() {
    let (mut engine, _events) = engine(40, 3);
    engine.process(b"hello\r\n\x1b]777;transom;shell=/bin/bash\x07");
    assert_eq!(engine.command_blocks().count(), 0);
    let events = engine.take_osc_events();
    assert!(
        events
            .iter()
            .any(|event| matches!(event, OscEvent::ShellReport { shell } if shell == "/bin/bash")),
        "握手事件必须原样交给视图层: {events:?}"
    );
    assert!(!engine.shell_markers_seen(), "握手不是 133 标记");
}
