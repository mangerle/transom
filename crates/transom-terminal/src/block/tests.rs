use super::*;

fn store_with_threshold(rows: usize) -> BlockStore {
    let mut store = BlockStore::default();
    store.set_auto_collapse_rows(rows);
    store
}

/// 完整 A/B/C/D/A 序列产出一条带退出码、耗时与输出行数的块。
#[test]
fn full_sequence_builds_one_block() {
    let mut store = store_with_threshold(0);
    store.on_prompt(0);
    store.on_command_start(0);
    store.on_preexec(1, "cargo build".into());
    store.on_finish(Some(0), Some(4_200), 10, true);
    store.on_prompt(11);
    let blocks: Vec<_> = store.blocks().cloned().collect();
    assert_eq!(blocks.len(), 1);
    let block = &blocks[0];
    assert_eq!(block.prompt_line, 0);
    assert_eq!(block.command_line, 0);
    assert_eq!(block.output_start_line, 1);
    assert_eq!(block.output_end_line, 10);
    assert_eq!(block.output_rows(), 10);
    assert_eq!(block.exit_code, Some(0));
    assert_eq!(block.duration_ms, Some(4_200));
    assert!(!block.collapsed);
}

/// 超过阈值的输出自动折叠；阈值 0 关闭自动折叠。
#[test]
fn auto_collapse_respects_threshold() {
    let mut store = store_with_threshold(5);
    store.on_preexec(0, "make".into());
    store.on_finish(Some(1), None, 20, true);
    store.on_prompt(21);
    let block = store.blocks().next().unwrap();
    assert!(block.collapsed, "20 行输出超过阈值 5 应自动折叠");

    let mut store = store_with_threshold(0);
    store.on_preexec(0, "make".into());
    store.on_finish(Some(1), None, 20, true);
    store.on_prompt(21);
    assert!(!store.blocks().next().unwrap().collapsed);
}

/// 空输出块也能收尾，行数为 0 且不参与折叠。
#[test]
fn empty_output_block_finishes_with_zero_rows() {
    let mut store = store_with_threshold(1);
    store.on_prompt(0);
    store.on_command_start(0);
    store.on_preexec(1, "true".into());
    store.on_finish(Some(0), Some(1), 1, true);
    store.on_prompt(1);
    let block = store.blocks().next().unwrap();
    assert_eq!(block.output_rows(), 0);
    assert!(!block.collapsed);
}

/// 折叠布局只保留横幅与块外行，光标行（块后提示符）始终可见。
#[test]
fn layout_collapses_output_into_banner() {
    let mut store = store_with_threshold(2);
    // 3 行输出，阈值 2 → 自动折叠。
    store.on_prompt(0);
    store.on_command_start(0);
    store.on_preexec(1, "make".into());
    store.on_finish(Some(0), Some(10), 4, true);
    store.on_prompt(4);
    store.on_command_start(4);

    // 网格：history=0，可见 6 行（绝对 0..=5），底部贴底。
    let layout = build_layout(0, 6, 0, &store, BlockViewStyle::Card);
    let slots = layout.slots();
    assert_eq!(slots.len(), 6);
    // 无历史可补，折叠省出的两行落在顶部；块本体与光标行保持贴底。
    assert_eq!(slots[0], VisualSlot::Blank);
    assert_eq!(slots[1], VisualSlot::Blank);
    assert_eq!(slots[2], VisualSlot::Line(Line(0)));
    match &slots[3] {
        VisualSlot::Banner {
            block_id, label, ..
        } => {
            assert_eq!(*block_id, 0);
            // 断言按当前语言的横幅模板拼出期望值，避免测试依赖某一门语言。
            let expected = rust_i18n::t!(
                "terminal.block.banner",
                rows = 3,
                status = rust_i18n::t!("terminal.block.status_success"),
                duration = " · 10ms"
            )
            .to_string();
            assert!(label.contains(&expected), "横幅: {label}");
        }
        other => panic!("应为横幅: {other:?}"),
    }
    assert_eq!(slots[4], VisualSlot::Line(Line(4)));
    assert_eq!(slots[5], VisualSlot::Line(Line(5)));
}

/// 展开后布局回到原始行序。
#[test]
fn expanded_block_keeps_all_rows() {
    let mut store = store_with_threshold(2);
    store.on_preexec(0, "make".into());
    store.on_finish(Some(0), None, 2, true);
    store.on_prompt(3);
    assert!(store.toggle(0), "折叠块应可展开");
    let layout = build_layout(0, 6, 0, &store, BlockViewStyle::Minimal);
    assert_eq!(layout.slots()[0], VisualSlot::Line(Line(0)));
    assert_eq!(layout.slots()[2], VisualSlot::Line(Line(2)));
}

/// 折叠节省的空间由更早的历史行补足，顶部不会跳到错误内容。
#[test]
fn collapsed_space_is_filled_from_older_history() {
    let mut store = store_with_threshold(1);
    store.on_preexec(5, "log".into());
    store.on_finish(Some(0), None, 7, true);
    store.on_prompt(8);
    // history=10，可见 4 行（绝对 9..=12），折叠 5..=7 全在视口之上，
    // 不产生横幅，布局应与经典视图一致。
    let layout = build_layout(10, 4, 0, &store, BlockViewStyle::Card);
    assert_eq!(layout.slots()[0], VisualSlot::Line(Line(0)));
    assert!(layout.banner_at(0).is_none());
}

/// 滚动缓冲写满后块记录必须清空，避免绝对行号整体平移后错位。
#[test]
fn clear_removes_all_blocks_and_open_state() {
    let mut store = store_with_threshold(0);
    store.on_preexec(0, "x".into());
    store.on_finish(Some(0), None, 1, false);
    assert_eq!(store.blocks().count(), 1);
    store.clear();
    assert_eq!(store.blocks().count(), 0);
    assert!(store.collapsed_at(0).is_none());
}

#[test]
fn banner_label_formats_status_and_duration() {
    let block = CommandBlock {
        id: 1,
        prompt_line: 0,
        command_line: 0,
        output_start_line: 1,
        output_end_line: 120,
        command_text: "make".into(),
        exit_code: Some(2),
        duration_ms: Some(65_000),
        collapsed: true,
    };
    let label = banner_label(&block);
    let expected = rust_i18n::t!(
        "terminal.block.banner",
        rows = 120,
        status = rust_i18n::t!("terminal.block.status_exit_code", code = 2),
        duration = " · 1m5s"
    )
    .to_string();
    assert_eq!(label, expected);
}
