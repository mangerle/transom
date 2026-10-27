use super::*;

/// 打包成结构体而不是继续平铺：原先网格、列数、颜色表、光标格已经排到 9 个参数，
/// 再加一份逐列高亮色就会更多，参数顺序也更容易传错。
/// 一行关键词扫描的复用缓冲。
#[derive(Default)]
pub(super) struct RowScratch {
    /// 去掉宽字符占位格后的行文本。
    pub(super) text: String,
    /// 每个字符对应的列号。
    pub(super) columns: Vec<usize>,
    /// 每个字符占用的列宽（宽字符为 2）。
    pub(super) widths: Vec<usize>,
    /// 命中区间展开到每列的前景色。
    pub(super) colors: Vec<Option<Hsla>>,
    pub(super) spans: Vec<HighlightSpan>,
}

impl RowScratch {
    /// 扫描一行，把命中的关键词展开成"每列一个覆盖色"。
    ///
    /// 收配置/规则集/调色板三个引用而不是 `&TerminalRenderer`：后者只能在带窗口的环境里
    /// 构造，那样最容易出错的"字符下标 → 真实列号"换算就没法在单元测试里直接验证。
    pub(super) fn fill(
        &mut self,
        grid: &alacritty_terminal::grid::Grid<Cell>,
        line: Line,
        cols: usize,
        config: &TerminalHighlight,
        rules: &CompiledHighlight,
        palette: &TerminalPalette,
    ) {
        self.colors.clear();
        self.colors.resize(cols, None);
        if !config.enabled {
            return;
        }
        self.text.clear();
        self.columns.clear();
        self.widths.clear();
        for col in 0..cols {
            let cell = &grid[AlacPoint::new(line, Column(col))];
            if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            self.columns.push(col);
            self.widths.push(if cell.flags.contains(Flags::WIDE_CHAR) {
                2
            } else {
                1
            });
            self.text.push(cell.c);
        }
        if self.text.is_empty() {
            return;
        }
        self.spans = rules.scan(&self.text, config);
        for span in &self.spans {
            // 命中区间是按"打印字符"编号的，必须换算回真实列号：宽字符占两列，
            // 直接把字符下标当列号会让中文行整体错位；反过来把列号拿去索引字符数组
            // 会把区间画宽。两个索引空间要分开取，不能混用。
            let Some(last_char) = span.end.checked_sub(1) else {
                continue;
            };
            let (Some(&first), Some(&last_col), Some(&width)) = (
                self.columns.get(span.start),
                self.columns.get(last_char),
                self.widths.get(last_char),
            ) else {
                continue;
            };
            let end_col = (last_col + width).min(cols);
            if end_col <= first {
                continue;
            }
            let color = Hsla::from(rgb(palette.ansi[span.kind.ansi_index().min(15)].to_hex()));
            for slot in &mut self.colors[first..end_col] {
                *slot = Some(color);
            }
        }
    }
}

/// 终端下划线的渲染种类；`None` 表示无下划线。
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum UnderlineKind {
    None,
    Single,
    /// 波浪线（SGR 4:3），gpui 文本系统原生支持。
    Curl,
    /// 双下划线（SGR 4:2），按格自绘两条。
    Double,
    /// 点线（SGR 4:4），按格自绘点。
    Dotted,
    /// 虚线（SGR 4:5），按格自绘两段。
    Dashed,
}

/// 自绘下划线的定位参数（run 的线型、列区间与颜色）。
pub(super) struct CustomUnderline {
    pub(super) kind: UnderlineKind,
    pub(super) start_col: usize,
    pub(super) end_col: usize,
    pub(super) origin: Point<Pixels>,
    pub(super) row: usize,
    pub(super) color: Hsla,
}

/// 单元格的下划线样式：显式 SGR 下划线优先，OSC 8 超链接单元格恒定单下划线
/// （与 kitty/Alacritty 的默认表现一致），让链接可被发现。
pub(super) fn cell_underline_kind(cell: &Cell) -> UnderlineKind {
    let flags = cell.flags;
    if flags.contains(Flags::UNDERCURL) {
        UnderlineKind::Curl
    } else if flags.contains(Flags::DOTTED_UNDERLINE) {
        UnderlineKind::Dotted
    } else if flags.contains(Flags::DASHED_UNDERLINE) {
        UnderlineKind::Dashed
    } else if flags.contains(Flags::DOUBLE_UNDERLINE) {
        UnderlineKind::Double
    } else if flags.contains(Flags::UNDERLINE) || cell.hyperlink().is_some() {
        UnderlineKind::Single
    } else {
        UnderlineKind::None
    }
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) struct RunStyle {
    pub(super) fg: Hsla,
    pub(super) bold: bool,
    pub(super) italic: bool,
    pub(super) underline: UnderlineKind,
    /// 是否属于双格字符：决定取主字体还是中文专用字体，必须参与 run 合并判定，
    /// 否则两种字体的字符会被拼进同一次 `shape_line`。
    pub(super) wide: bool,
}

pub(super) struct TextRunBuilder {
    pub(super) text: String,
    pub(super) start_col: usize,
    pub(super) end_col: usize,
    pub(super) style: RunStyle,
}

/// 双格字号探针用的字符：`中` 在 CJK 字体里是最稳定的全宽字形。
pub(super) const CJK_PROBE_CHAR: char = '中';

/// IME 预编辑串里哪些字符按中文专用字体渲染。
///
/// 正文走的是网格的 `WIDE_CHAR` 标记，只有预编辑阶段的字符还没进网格、没有标记可用，
/// 才需要这份范围表兜底：范围取 Unicode 东亚宽度表里判为 Wide 的那批（汉字及扩展、假名、
/// 谚文、注音、中日韩标点与全角形式），半角假名从 U+FF61 起只占一格、刻意排除。
/// 判定偏窄只会让候选字暂时用回退字体渲染，偏宽才会撑破邻格，因此宁可偏窄。
pub(super) fn is_cjk_char(ch: char) -> bool {
    matches!(ch as u32,
        0x1100..=0x115F      // 谚文字母（初声，占两格）
        | 0x2E80..=0x303E    // 中日韩部首补充、康熙部首、假名、注音以外的中日韩符号标点
        | 0x3041..=0x33FF    // 平假名、片假名、注音符号、中日韩兼容符号
        | 0x3400..=0x4DBF    // 汉字扩展 A
        | 0x4E00..=0x9FFF    // 中日韩统一表意文字
        | 0xA000..=0xA4CF    // 彝文
        | 0xAC00..=0xD7A3    // 谚文音节
        | 0xF900..=0xFAFF    // 中日韩兼容表意文字
        | 0xFE30..=0xFE6F    // 小写变体、中日韩兼容形式、全角小符号
        | 0xFF00..=0xFF60    // 全角形式（半角假名从 FF61 起，不在此列）
        | 0xFFE0..=0xFFE6    // 全角货币与空格
        | 0x20000..=0x2FFFD  // 汉字扩展 B 及之后
        | 0x30000..=0x3FFFD
    )
}

/// 把双格字体字号缩放到"一个汉字恰好等于两格宽"。
///
/// 夹在基准字号的 ±50% 内：字体不存在时 gpui 会回退到别的字形、宽度探针可能离谱，
/// 与其把中文拽成蚂蚁字，不如保持接近原字号的结果。
pub(super) fn fit_cjk_size(probe_width: Pixels, target_width: Pixels, base: Pixels) -> Pixels {
    let natural = f32::from(probe_width);
    let target = f32::from(target_width);
    if natural <= 0.0 || target <= 0.0 {
        return base;
    }
    let scaled = f32::from(base) * (target / natural);
    px(scaled
        .clamp(f32::from(base) * 0.5, f32::from(base) * 1.5)
        .max(1.0))
}

pub(super) fn cell_text(cell: &Cell) -> String {
    let mut text = String::new();
    text.push(cell.c);
    if let Some(zerowidth) = cell.zerowidth() {
        text.extend(zerowidth);
    }
    text
}

pub(super) fn selection_segment(
    selection: &SelectionRange,
    line: Line,
    cols: usize,
) -> Option<(usize, usize)> {
    if line < selection.start.line || line > selection.end.line {
        return None;
    }
    let last_col = cols.saturating_sub(1);
    let start = if selection.is_block || line == selection.start.line {
        selection.start.column.0.min(last_col)
    } else {
        0
    };
    let end_inclusive = if selection.is_block || line == selection.end.line {
        selection.end.column.0.min(last_col)
    } else {
        last_col
    };
    (start <= end_inclusive).then_some((start, end_inclusive + 1))
}

pub(super) fn utf16_range_to_char_range(text: &str, range: &Range<usize>) -> Option<Range<usize>> {
    let mut utf16_offset = 0;
    let mut start = None;
    let mut end = None;
    for (char_index, ch) in text.chars().enumerate() {
        if utf16_offset == range.start {
            start = Some(char_index);
        }
        if utf16_offset == range.end {
            end = Some(char_index);
            break;
        }
        utf16_offset += ch.len_utf16();
    }
    let char_count = text.chars().count();
    if utf16_offset == range.start {
        start = Some(char_count);
    }
    if utf16_offset == range.end {
        end = Some(char_count);
    }
    match (start, end) {
        (Some(start), Some(end)) if start <= end => Some(start..end),
        _ => None,
    }
}
