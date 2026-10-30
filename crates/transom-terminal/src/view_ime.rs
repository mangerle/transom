use crate::view::TerminalView;
use gpui_kit::*;
use std::ops::Range;

impl EntityInputHandler for TerminalView {
    fn text_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self
            .ime_marked_range
            .clone()
            .unwrap_or(0..self.ime_text.encode_utf16().count());
        *adjusted_range = Some(range);
        Some(self.ime_text.clone())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        // 组字期间把组合文本末端作为光标位置上报：输入法层据此向 bounds_for_range
        // 询问组字光标所在格来定位候选窗。若恒返回 0..0，候选窗会被固定在组字起点，
        // 且不随组合文本增长而移动。
        Some(UTF16Selection {
            range: self.ime_marked_range.clone().unwrap_or(0..0),
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.ime_marked_range.clone()
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.ime_marked_range = None;
        self.ime_text.clear();
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ime_marked_range = None;
        self.ime_text.clear();
        if !text.is_empty() {
            self.write_input(text.as_bytes());
            self.state.engine.lock().scroll_to_bottom();
            // 可打印输入（含 IME 上屏）累积进本地输入行，随后刷新历史建议。
            self.input_line.push_str(text);
            self.note_input_changed(false, cx);
        }
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ime_text = new_text.to_string();
        self.ime_marked_range = if new_text.is_empty() {
            None
        } else {
            new_selected_range.or_else(|| Some(0..new_text.encode_utf16().count()))
        };
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        _element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let metrics = *self.state.metrics.lock();
        if metrics.cols == 0 || metrics.rows == 0 {
            return None;
        }
        let engine = self.state.engine.lock();
        let cursor = engine.cursor_point();
        let offset = engine.display_offset() as i32;
        let row = cursor.line.0 + offset;
        if row < 0 || row as usize >= metrics.rows || cursor.column.0 >= metrics.cols {
            return None;
        }
        // GPUI 定位候选窗时会把查询区间收敛到「组字所在可视行的行首」再取矩形
        // （gpui-pre platform.rs 的 compute_ime_candidate_bounds），单行组字时
        // 询问的始终是组字起点。因此凡落在组合文本内的查询，统一折算到组合文本末端
        // 即组字光标所在格，候选窗才会随拼音逐字右移；组合文本跨行时该格也落在
        // 光标真实位置，无需依赖框架的行首探测。
        let caret_index = ime_caret_index_for_query(&range_utf16, self.ime_marked_range.as_ref());
        // 组合文本自绘在光标格起始处，末端下标折算出的终端列数即组字光标相对起始格的偏移；
        // 终端网格按 cols 折行，故列偏移需同时推进行号，长拼音跨行时矩形才不会溢出右边界。
        let caret_cells = ime_utf16_index_to_column_cells(&self.ime_text, caret_index);
        let total_columns = cursor.column.0 + caret_cells;
        let caret_row = row + (total_columns / metrics.cols) as i32;
        if caret_row < 0 || caret_row as usize >= metrics.rows {
            return None;
        }
        let caret_column = total_columns % metrics.cols;
        Some(Bounds {
            origin: Point {
                x: metrics.origin.x + metrics.cell_width * caret_column as f32,
                y: metrics.origin.y + metrics.cell_height * caret_row as f32,
            },
            size: Size {
                width: metrics.cell_width.max(px(1.0)),
                height: metrics.cell_height.max(px(1.0)),
            },
        })
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        // 输入法组合文本不属于网格，以光标单元格为起点单独渲染；把点击位置
        // 折算成组合文本内的 UTF-16 下标：命中光标行带后按列偏移换算，
        // 两侧越界收敛到端点，组合为空时恒为 0。
        let utf16_len = self.ime_text.encode_utf16().count();
        if utf16_len == 0 {
            return Some(0);
        }
        let metrics = *self.state.metrics.lock();
        if metrics.cell_width <= px(0.0) || metrics.cell_height <= px(0.0) {
            return Some(0);
        }
        let engine = self.state.engine.lock();
        let cursor = engine.cursor_point();
        let row = cursor.line.0 + engine.display_offset() as i32;
        let row_y = f32::from(metrics.origin.y + metrics.cell_height * row.max(0) as f32);
        let start_x = f32::from(metrics.origin.x + metrics.cell_width * cursor.column.0 as f32);
        drop(engine);

        if f32::from(point.y) < row_y {
            return Some(0);
        }
        if f32::from(point.y) > row_y + f32::from(metrics.cell_height) {
            return Some(utf16_len);
        }
        let col_cells =
            ((f32::from(point.x) - start_x) / f32::from(metrics.cell_width)).floor() as usize;
        Some(ime_column_to_utf16_index(&self.ime_text, col_cells))
    }
}

/// 把组合文本内按格数计的列偏移折算成 UTF-16 下标。
/// 非 ASCII 字符按双格近似（与终端网格的 CJK 宽度一致），折算只服务输入法
/// 候选窗定位，不追求逐像素精确。
pub(crate) fn ime_column_to_utf16_index(text: &str, col_cells: usize) -> usize {
    let mut cells = 0usize;
    for (index, ch) in text.char_indices() {
        if cells >= col_cells {
            return text[..index].encode_utf16().count();
        }
        cells += if ch.is_ascii() { 1 } else { 2 };
    }
    text.encode_utf16().count()
}

/// 选取用于定位候选窗的光标下标。
///
/// GPUI 在定位前会把查询区间收敛到组字所在可视行的行首，单行组字时询问的始终是
/// 组字起点；落在组合文本内的查询统一取组合文本末端（组字光标），候选窗才会跟随
/// 拼音右移。组合文本范围之外的查询保持原样，避免影响其它调用方。
pub(crate) fn ime_caret_index_for_query(
    range_utf16: &Range<usize>,
    marked_range: Option<&Range<usize>>,
) -> usize {
    match marked_range {
        Some(marked) if range_utf16.start >= marked.start && range_utf16.end <= marked.end => {
            marked.end
        }
        _ => range_utf16.end,
    }
}

/// 把组合文本内的 UTF-16 下标折算成其末端占据的终端列数。
///
/// 与 `ime_column_to_utf16_index` 采用同一宽度假设：ASCII 单格、非 ASCII 双格近似。
/// 输入法层以选区末端向 `bounds_for_range` 询问组字光标位置，此处负责把该下标
/// 换算成相对组字起始格的列偏移。
pub(crate) fn ime_utf16_index_to_column_cells(text: &str, utf16_index: usize) -> usize {
    let mut cells = 0usize;
    let mut consumed = 0usize;
    for ch in text.chars() {
        if consumed >= utf16_index {
            break;
        }
        cells += if ch.is_ascii() { 1 } else { 2 };
        consumed += ch.len_utf16();
    }
    cells
}
