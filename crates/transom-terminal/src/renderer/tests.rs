use super::palette::palette_rgb;
use super::scan::{CJK_PROBE_CHAR, RowScratch, fit_cjk_size, is_cjk_char};
use super::{PaletteColors, background_alpha};
use crate::engine::TerminalEngine;
use crate::highlight::CompiledHighlight;
use alacritty_terminal::index::Line;
use gpui_kit::{Hsla, px, rgb};
use transom_core::{TerminalHighlight, TerminalPalette, TerminalProfile, TerminalSize};

/// 背景透明度折算：0% 完全不透明，100% 完全透明；越界值先钳位。
#[test]
fn background_alpha_maps_transparency_to_opacity() {
    assert!((background_alpha(0) - 1.0).abs() < f32::EPSILON);
    assert!((background_alpha(50) - 0.5).abs() < f32::EPSILON);
    assert!(background_alpha(100).abs() < f32::EPSILON);
    // 越界百分比不能折出负不透明度：那会让整块背景凭空消失。
    assert!(background_alpha(200) >= 0.0);
    assert!((background_alpha(255) - background_alpha(100)).abs() < f32::EPSILON);
}

/// 预计算表与即时换算必须逐位一致：查表是渲染热路径的取色来源，
/// 任何偏差都会在升级后被当成"配色变了"的回归。
#[test]
fn palette_colors_match_immediate_conversion() {
    let palette = TerminalPalette::default();
    let cached = PaletteColors::build(&palette);
    let immediate = |color: transom_core::Rgb| Hsla::from(rgb(color.to_hex()));

    assert_eq!(
        palette_rgb(palette.foreground),
        immediate(palette.foreground)
    );
    assert_eq!(
        palette_rgb(palette.background),
        immediate(palette.background)
    );
    assert_eq!(palette_rgb(palette.cursor), immediate(palette.cursor));
    assert_eq!(palette_rgb(palette.selection), immediate(palette.selection));
    assert_eq!(cached.foreground, immediate(palette.foreground));
    assert_eq!(cached.background, immediate(palette.background));
    for slot in 0..16 {
        assert_eq!(cached.ansi[slot], immediate(palette.ansi[slot]));
    }
    // 256 色回退表覆盖立方与灰度两端各抽样。
    for index in [16usize, 100, 231, 232, 255] {
        let expected = if index < 232 {
            let cube = index - 16;
            let r = (cube / 36) as u8;
            let g = ((cube % 36) / 6) as u8;
            let b = (cube % 6) as u8;
            let channel = |value: u8| if value == 0 { 0 } else { value * 40 + 55 };
            immediate(transom_core::Rgb::new(channel(r), channel(g), channel(b)))
        } else {
            let level = 8 + (index - 232) as u8 * 10;
            immediate(transom_core::Rgb::new(level, level, level))
        };
        assert_eq!(cached.indexed_fallback[index - 16], expected);
    }
    // Dim 变体只压 alpha，色相分量保持原值。
    assert!((cached.ansi_dim[0].a - cached.ansi[0].a * 0.7).abs() < 1e-6);
    assert!((cached.dim_foreground.a - cached.foreground.a * 0.7).abs() < 1e-6);
}

/// 双字体路由的判定边界：走错一侧就会让 Latin 字符被中文字体接管，
/// 或让全角字符继续用等宽字体而错位。
#[test]
fn classifies_double_width_characters() {
    assert!(is_cjk_char('中'));
    assert!(is_cjk_char(CJK_PROBE_CHAR));
    assert!(is_cjk_char('あ'));
    assert!(is_cjk_char('カ'));
    assert!(is_cjk_char('한'));
    assert!(is_cjk_char('　')); // 全角空格
    assert!(is_cjk_char('，'));
    assert!(is_cjk_char('！')); // 全角感叹
    assert!(is_cjk_char('Ａ')); // 全角拉丁
    assert!(is_cjk_char('ㄅ')); // 注音符号按东亚宽度占两格
}

#[test]
fn keeps_narrow_characters_on_the_primary_font() {
    for ch in ['a', 'Z', '0', ' ', '~', 'é', 'ß', 'ф', 'é'] {
        assert!(!is_cjk_char(ch), "{ch} 不应被判定为双格");
    }
    // 半角假名在网格里只占一格，用中文专用字体会撑破邻格。
    assert!(!is_cjk_char('ｱ'));
    assert!(!is_cjk_char('¦'));
    // 组合用零宽字符也不该抢双格字体。
    assert!(!is_cjk_char('\u{0301}'));
}

#[test]
fn fits_cjk_size_to_exactly_two_cells() {
    // 探针 20px 的目标是 16px（两格 = 2 × 8px）→ 字号按 0.8 缩放。
    let fitted = fit_cjk_size(px(20.0), px(16.0), px(14.0));
    assert!((f32::from(fitted) - 11.2).abs() < 0.01);
    // 已经正好两格时不变。
    assert_eq!(fit_cjk_size(px(16.0), px(16.0), px(14.0)), px(14.0));
}

/// 字体缺失时 gpui 的回退宽度可能离谱，缩放必须夹在可用范围内。
#[test]
fn clamps_degenerate_probe_widths() {
    assert_eq!(fit_cjk_size(px(0.0), px(16.0), px(14.0)), px(14.0));
    assert_eq!(fit_cjk_size(px(1.0), px(16.0), px(14.0)), px(21.0));
    assert_eq!(fit_cjk_size(px(1000.0), px(16.0), px(14.0)), px(7.0));
    assert_eq!(fit_cjk_size(px(-3.0), px(16.0), px(14.0)), px(14.0));
    assert_eq!(fit_cjk_size(px(20.0), px(0.0), px(14.0)), px(14.0));
}

/// 用一行真实网格内容填充高亮，返回每列覆盖色的十六进制（`None` 表示该列不上色）。
///
/// 断言比色值而不是比调色板下标：默认调色板里 ansi[1] 与 ansi[9] 同值，
/// 反查下标会先命中前者，测试会变成在测查找顺序而不是测上色。
fn highlighted_columns(text: &str, keywords: &[&str], cols: u16) -> Vec<Option<u32>> {
    let (tx, _rx) = flume::bounded(16);
    let profile = TerminalProfile {
        highlight: TerminalHighlight {
            enabled: true,
            keywords: keywords.iter().map(|k| (*k).to_string()).collect(),
        },
        ..TerminalProfile::default()
    };
    let mut engine = TerminalEngine::new(TerminalSize::new(cols, 3), &profile, tx);
    engine.process(text.as_bytes());
    let rules = CompiledHighlight::compile(&profile.highlight);
    let mut scratch = RowScratch::default();
    // 扫描时刻意不看渲染结果，只比对"这列该染成调色板里的哪个 ANSI 槽位"。
    scratch.fill(
        engine.grid(),
        Line(0),
        cols as usize,
        &profile.highlight,
        &rules,
        &profile.palette,
    );
    let ansi = &profile.palette.ansi;
    scratch
        .colors
        .iter()
        .map(|color| {
            color.map(|color| {
                ansi.iter()
                    .find(|candidate| Hsla::from(rgb(candidate.to_hex())) == color)
                    .expect("覆盖色必须来自调色板")
                    .to_hex()
            })
        })
        .collect()
}

/// 关键词必须落在它真正所在的列上：ANSI 9 是错误级别（ERROR/FATAL）的槽位。
#[test]
fn highlights_expected_columns() {
    let red = TerminalPalette::default().ansi[9].to_hex();
    let colors = highlighted_columns("ERROR boom here", &[], 20);
    assert_eq!(colors[0], Some(red));
    assert_eq!(colors[4], Some(red));
    assert_eq!(colors[5], None, "关键词后一列不该继续染色");
}

/// 宽字符占两列：只按字符下标定列会让中文行的染色整体左移。
#[test]
fn wide_characters_shift_columns() {
    // 「中文」占 0..4 四列，ERROR 起于第 4 列。
    let red = TerminalPalette::default().ansi[9].to_hex();
    let colors = highlighted_columns("中文ERROR tail", &[], 20);
    assert_eq!(colors[3], None, "汉字本身不染色");
    assert_eq!(colors[4], Some(red));
    assert_eq!(colors[8], Some(red));
    assert_eq!(colors[9], None);
}

#[test]
fn custom_keyword_columns_use_green_slot() {
    let green = TerminalPalette::default().ansi[10].to_hex();
    let colors = highlighted_columns("a refused b", &["refused"], 20);
    assert_eq!(colors[2], Some(green));
    assert_eq!(colors[8], Some(green), "refused 覆盖到第 8 列");
    assert_eq!(colors[9], None, "词后空格不该染色");
}

#[test]
fn ipv4_and_uuid_slots() {
    let palette = TerminalPalette::default();
    let colors = highlighted_columns("ip 10.0.0.1 ok", &[], 20);
    assert_eq!(colors[3], Some(palette.ansi[14].to_hex()));
    assert_eq!(colors[10], Some(palette.ansi[14].to_hex()));
    assert_eq!(colors[11], None);
    let colors = highlighted_columns("id 6f1c2b3a-4d5e-4f60-7a8b-9c0d1e2f3a4b", &[], 40);
    assert_eq!(colors[3], Some(palette.ansi[13].to_hex()));
}

/// 行尾空白列与越界列都不能panic：网格列数会随窗口缩放。
#[test]
fn empty_and_partial_rows_are_safe() {
    let colors = highlighted_columns("ERROR", &[], 40);
    assert_eq!(colors.len(), 40);
    assert_eq!(colors[6], None, "行尾空白列不上色");
    let colors = highlighted_columns("", &[], 10);
    assert!(colors.iter().all(|color| color.is_none()));
}

/// 关掉总开关时零染色，且不做任何扫描工作。
#[test]
fn disabled_highlight_colors_nothing() {
    let (tx, _rx) = flume::bounded(16);
    let profile = TerminalProfile {
        highlight: TerminalHighlight {
            enabled: false,
            keywords: vec!["boom".into()],
        },
        ..TerminalProfile::default()
    };
    let mut engine = TerminalEngine::new(TerminalSize::new(20, 3), &profile, tx);
    engine.process(b"ERROR boom");
    let rules = CompiledHighlight::compile(&profile.highlight);
    let mut scratch = RowScratch::default();
    scratch.fill(
        engine.grid(),
        Line(0),
        20,
        &profile.highlight,
        &rules,
        &profile.palette,
    );
    assert!(scratch.colors.iter().all(|color| color.is_none()));
    assert!(scratch.text.is_empty());
}

/// 相邻两行复用同一缓冲：上一行的字符与染色不能残留到下一行。
#[test]
fn scratch_reuse_does_not_leak_between_rows() {
    let (tx, _rx) = flume::bounded(16);
    let profile = TerminalProfile::default();
    let mut engine = TerminalEngine::new(TerminalSize::new(20, 3), &profile, tx);
    // 必须带 `\r`：单独的 `\n` 在终端里只是下移一行而不复位列，两行文字会错位到同一列起点。
    let stream = "ERROR first\r\nsecond line\r\n";
    engine.process(stream.as_bytes());
    let rules = CompiledHighlight::compile(&profile.highlight);
    let mut scratch = RowScratch::default();
    scratch.fill(
        engine.grid(),
        Line(0),
        20,
        &profile.highlight,
        &rules,
        &profile.palette,
    );
    assert!(scratch.colors[0].is_some(), "第一行有 ERROR，必须染色");
    scratch.fill(
        engine.grid(),
        Line(1),
        20,
        &profile.highlight,
        &rules,
        &profile.palette,
    );
    assert!(
        scratch.colors.iter().all(|color| color.is_none()),
        "第二行没有关键词，必须全空"
    );
    assert_eq!(
        scratch.text.trim_end(),
        "second line",
        "行尾补白不该留下上一行痕迹"
    );
}
