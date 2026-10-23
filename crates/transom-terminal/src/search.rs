//! 终端缓冲区全文检索：把一次查询编译成 DFA，扫描整个滚动缓冲区得到命中快照。
//!
//! 为什么是"快照 + 纪元失效"而不是持续增量维护命中列表：
//! alacritty_terminal 的 `Line` 是**相对视口底部**的编号（探针见 `engine::tests`），
//! 一旦有新行滚入，同一条内容的所有命中行号整体 -1；对外又没有公开的绝对行计数器，
//! 无法把缓存行号平移回当前坐标系。因此把"缓冲区变过"当作快照失效信号，
//! 由重扫而不是增量修正来恢复正确性，代价是高频输出时结果会短暂缺位。
//!
//! 重扫代价与整个 scrollback 的单元格数成正比（实测 10000 行 × 120 列在 debug 构建下
//! 需要 40~126ms），所以放进渲染帧里按两个条件放行：查询刚变过立即扫一次；快照因新输出
//! 而失效时，要等输出停歇一个窗口并且过了 [`RESCAN_INTERVAL`] 冷却才重扫。
//! 代价是刷屏期间结果会短暂缺位（搜索栏会写明"输出中，稍后自动更新"），
//! 换来的是刷屏不再被检索拖成幻灯片。

use crate::engine::TerminalEngine;
use crate::event::TerminalEventProxy;
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Direction, Line, Point, Side};
use alacritty_terminal::term::Term;
use alacritty_terminal::term::search::{Match, RegexSearch};
use std::time::{Duration, Instant};

/// 允许重扫前要求的输出停歇时长。
pub(crate) const OUTPUT_QUIET_WINDOW: Duration = Duration::from_millis(120);

/// 一次扫描最多登记的命中数。
///
/// 上限存在的理由：`ERROR` 这类高频词能在 10000 行里命中上万次，命中列表既要进内存
/// 也要逐条绘制；到顶即停止扫描并在搜索栏标注“过多”，比让界面卡住更有用。
pub(crate) const MAX_MATCHES: usize = 2_000;

/// 缓冲区失效后允许重扫的最小间隔。
const RESCAN_INTERVAL: Duration = Duration::from_millis(300);
/// 把选中文字预填进搜索栏时的长度上限，避免把整段缓冲区当成查询串。
const PREFILL_MAX_CHARS: usize = 128;

/// 正则元字符集，与 `regex::escape` 的字面量化口径一致（关键词模式使用）。
const REGEX_METACHARS: &[u8] = b"\\.+*?()|[]{}^$#&-~";

/// 将用户输入按字面量编译，避免关键词模式里的 `.`、`[` 被当成正则元字符。
fn escape_literal(query: &str) -> String {
    let mut escaped = String::with_capacity(query.len() + query.len() / 2);
    for ch in query.chars() {
        if REGEX_METACHARS.contains(&(ch as u8)) && ch.is_ascii() {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

/// 单个命中在网格坐标系里的区间（跨行命中保留起止两端）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Hit {
    pub(crate) start: Point,
    pub(crate) end: Point,
}

impl Hit {
    fn from_match(found: &Match) -> Self {
        Self {
            start: *found.start(),
            end: *found.end(),
        }
    }
}

/// 一个渲染帧可用的绘制数据，已按当前显示偏移换算到视口行号。
#[derive(Clone, Default)]
pub(crate) struct SearchPaint {
    /// 每个视口行的命中区间，索引即屏幕行号（0 为可见区首行）。
    pub(crate) rows: Vec<RowPaint>,
    /// 右侧刻度带上的命中位置，取值 `0.0`（缓冲区顶部）到 `1.0`（缓冲区底部），已按像素去重。
    pub(crate) ticks: Vec<f32>,
}

/// 检索栏渲染所需的状态摘要。
///
/// 渲染层不能长期持有引擎锁（`TerminalView` 的锁还要给数据流与鼠标事件用），
/// 因此一次性拷出这几个标量，让搜索栏在锁之外完成文本拼装。
pub(crate) struct SearchStatus {
    pub(crate) has_query: bool,
    pub(crate) total: usize,
    pub(crate) ordinal: usize,
    pub(crate) truncated: bool,
    pub(crate) error: Option<String>,
    /// 快照是否已随缓冲区变化失效（此时计数与刻度都只是过期参考，界面上要说明）。
    pub(crate) stale: bool,
}

/// 单个屏幕行的检索绘制区间。
#[derive(Clone, Default)]
pub(crate) struct RowPaint {
    /// 非当前命中的区间集合，元素为 `(起始列, 结束列 + 1)`。
    pub(crate) hits: Vec<(usize, usize)>,
    /// 当前命中的区间，绘制时覆盖在普通命中之上。
    pub(crate) current: Option<(usize, usize)>,
}

/// 检索状态：当前查询、编译结果与整缓冲命中快照。
#[derive(Default)]
pub(crate) struct SearchState {
    /// 用户输入的原始查询（显示用）。
    query: String,
    /// 是否把查询当正则表达式解析。
    use_regex: bool,
    /// 编译好的自动机；非法正则时为 `None` 并保留错误文本。
    regex: Option<RegexSearch>,
    /// 上一次编译失败的原因，供搜索栏就地提示。
    error: Option<String>,
    /// 命中列表，按行号升序。
    hits: Vec<Hit>,
    /// 当前命中在 [`SearchState::hits`] 中的下标。
    current: Option<usize>,
    truncated: bool,
    /// 快照对应的缓冲区纪元；与引擎纪元不一致即失效。
    scanned_epoch: u64,
    /// 下次允许重扫的时刻，用于节流高频输出下的全缓冲扫描。
    next_scan_at: Option<Instant>,
}

impl SearchState {
    /// 更新查询与模式；内容变化时立即作废快照并请求一次不迟到的重扫。
    pub(crate) fn set_query(&mut self, query: &str, use_regex: bool) {
        if self.query == query && self.use_regex == use_regex {
            return;
        }
        self.query = query.to_string();
        self.use_regex = use_regex;
        self.hits.clear();
        self.current = None;
        self.truncated = false;
        self.scanned_epoch = u64::MAX;
        self.next_scan_at = None;
        self.regex = None;
        self.error = None;
        if query.is_empty() {
            return;
        }
        let pattern = if use_regex {
            query.to_string()
        } else {
            escape_literal(query)
        };
        match RegexSearch::new(&pattern) {
            Ok(regex) => self.regex = Some(regex),
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    pub(crate) fn is_active(&self) -> bool {
        self.regex.is_some()
    }

    /// 快照是否已随缓冲区变化而失效（失效期间不绘制任何高亮，避免指错位置）。
    pub(crate) fn is_stale(&self, epoch: u64) -> bool {
        self.scanned_epoch != epoch
    }

    /// 当前是否允许扫描：查询可用且快照已失效。
    ///
    /// 两种放行条件：`next_scan_at` 为空表示查询刚变过，立刻扫一次让用户不必等待；
    /// 否则必须同时满足"输出已停歇"（`quiet`）与冷却到期，避免刷屏时每一帧都做
    /// 一次全缓冲扫描（实测 10000 行 × 120 列在 debug 构建下要 40~126ms）。
    pub(crate) fn scan_allowed(&self, epoch: u64, now: Instant, quiet: bool) -> bool {
        if self.regex.is_none() || self.scanned_epoch == epoch {
            return false;
        }
        match self.next_scan_at {
            None => true,
            Some(due) => quiet && now >= due,
        }
    }

    /// 记下一次扫描结果。`epoch` 为扫描时缓冲区纪元。
    pub(crate) fn record_scan(
        &mut self,
        epoch: u64,
        hits: Vec<Hit>,
        truncated: bool,
        now: Instant,
    ) {
        let previous_line = self.current_hit().map(|hit| hit.start.line);
        self.hits = hits;
        self.truncated = truncated;
        self.scanned_epoch = epoch;
        self.next_scan_at = Some(now + RESCAN_INTERVAL);
        self.current = previous_line.and_then(|line| self.nearest_index(line));
    }

    /// 交出自动机的所有权供扫描使用。
    ///
    /// `Term::search_next` 需要 `&mut RegexSearch`（DFA 缓存就地复用），而扫描结果又要
    /// 写回本结构，直接借 `self` 会同时读写同一字段；取出再放回把这个冲突显式化。
    pub(crate) fn take_regex(&mut self) -> Option<RegexSearch> {
        self.regex.take()
    }

    pub(crate) fn put_regex(&mut self, regex: RegexSearch) {
        self.regex = Some(regex);
    }

    pub(crate) fn clear(&mut self) {
        self.query.clear();
        self.hits.clear();
        self.current = None;
        self.truncated = false;
        self.regex = None;
        self.error = None;
        self.scanned_epoch = u64::MAX;
        self.next_scan_at = None;
    }

    /// 当前命中序号，从 1 开始；无当前命中时为 0。
    pub(crate) fn ordinal(&self) -> usize {
        self.current.map_or(0, |index| index + 1)
    }

    pub(crate) fn current_hit(&self) -> Option<Hit> {
        self.current.and_then(|index| self.hits.get(index).copied())
    }

    pub(crate) fn status(&self, epoch: u64) -> SearchStatus {
        SearchStatus {
            has_query: !self.query.is_empty(),
            total: self.hits.len(),
            ordinal: self.ordinal(),
            truncated: self.truncated,
            error: self.error.clone(),
            stale: self.is_stale(epoch),
        }
    }

    /// 把游标移到下一个命中，到末尾后回绕到第一个。
    pub(crate) fn select_next(&mut self) -> Option<Hit> {
        if self.hits.is_empty() {
            return None;
        }
        let next = self
            .current
            .map_or(0, |index| (index + 1) % self.hits.len());
        self.current = Some(next);
        self.hits.get(next).copied()
    }

    /// 把游标移到上一个命中，到头后回绕到最后一个。
    pub(crate) fn select_prev(&mut self) -> Option<Hit> {
        if self.hits.is_empty() {
            return None;
        }
        let last = self.hits.len() - 1;
        let prev = self
            .current
            .map_or(last, |index| (index + last) % self.hits.len());
        self.current = Some(prev);
        self.hits.get(prev).copied()
    }

    /// 按缓冲区内的纵向比例选中最近的命中，供刻度带点击跳转使用。
    pub(crate) fn select_by_fraction(
        &mut self,
        fraction: f32,
        top: Line,
        bottom: Line,
    ) -> Option<Hit> {
        if self.hits.is_empty() {
            return None;
        }
        let span = (bottom.0 - top.0).max(1) as f32;
        let target = top.0 + (fraction.clamp(0.0, 1.0) * span).round() as i32;
        let index = self.nearest_index(Line(target))?;
        self.current = Some(index);
        self.hits.get(index).copied()
    }

    /// 第一个 `start.line >= line` 的命中下标，不存在时取最后一个。
    fn nearest_index(&self, line: Line) -> Option<usize> {
        if self.hits.is_empty() {
            return None;
        }
        let index = self
            .hits
            .partition_point(|hit| hit.start.line < line)
            .min(self.hits.len() - 1);
        Some(index)
    }

    /// 构造当前帧的绘制数据。
    ///
    /// `display_offset` 为向上滚动的行数，可见行 `row` 对应网格行号 `row - display_offset`，
    /// 与 [`crate::renderer`] 里的行映射保持同一口径。
    pub(crate) fn paint(
        &self,
        display_offset: i32,
        rows: usize,
        cols: usize,
        top: Line,
        bottom: Line,
        tick_rows: usize,
    ) -> SearchPaint {
        if self.hits.is_empty() {
            return SearchPaint {
                rows: vec![RowPaint::default(); rows],
                ticks: Vec::new(),
            };
        }
        let viewport_top = -display_offset;
        let viewport_bottom = rows as i32 - 1 - display_offset;
        let mut row_paints = vec![RowPaint::default(); rows];
        let current = self.current_hit();
        for hit in &self.hits {
            let hit_top = hit.start.line.0.max(viewport_top);
            let hit_bottom = hit.end.line.0.min(viewport_bottom);
            if hit_top > hit_bottom {
                continue;
            }
            for line in hit_top..=hit_bottom {
                let row = (line + display_offset) as usize;
                if row >= rows {
                    continue;
                }
                // 跨行命中的首行从命中起始列开始、末行到命中结束列截止，中间行整行着色。
                let start_col = if line == hit.start.line.0 {
                    hit.start.column.0
                } else {
                    0
                };
                let end_col = if line == hit.end.line.0 {
                    hit.end.column.0.saturating_add(1)
                } else {
                    cols
                };
                let is_current = current == Some(*hit);
                let Some(end_col) = clamp_span(start_col, end_col, cols) else {
                    continue;
                };
                let row_state = &mut row_paints[row];
                if is_current {
                    row_state.current = Some((start_col, end_col));
                } else {
                    row_state.hits.push((start_col, end_col));
                }
            }
        }

        let ticks = build_ticks(&self.hits, top, bottom, tick_rows);
        SearchPaint {
            rows: row_paints,
            ticks,
        }
    }
}

/// 把列区间夹进 `[0, cols)`，返回 `None` 表示该段不可见。
fn clamp_span(start: usize, end: usize, cols: usize) -> Option<usize> {
    let end = end.min(cols);
    (start < cols && start < end).then_some(end)
}

/// 把命中映射到刻度带：按像素桶去重，避免上千个命中画出上千个矩形。
fn build_ticks(hits: &[Hit], top: Line, bottom: Line, tick_rows: usize) -> Vec<f32> {
    // 少于 2 个桶时无法给出可用的比例分母（窗格被压到 2px 以下的极端情况），直接不画。
    if tick_rows < 2 || bottom.0 <= top.0 {
        return Vec::new();
    }
    let span = (bottom.0 - top.0) as f32;
    let mut buckets = vec![false; tick_rows];
    for hit in hits {
        let fraction = (hit.start.line.0 - top.0) as f32 / span;
        let bucket = (fraction.clamp(0.0, 1.0) * (tick_rows - 1) as f32).round() as usize;
        buckets[bucket.min(tick_rows - 1)] = true;
    }
    buckets
        .iter()
        .enumerate()
        .filter(|(_, occupied)| **occupied)
        .map(|(bucket, _)| bucket as f32 / (tick_rows - 1) as f32)
        .collect()
}

/// 用编译好的自动机把整个缓冲区扫一遍。
///
/// 逐次调用 `Term::search_next` 而不是自己拼行文本再匹配：前者已正确处理宽字符、
/// 自动换行（`WRAPLINE`）与行尾空白语义，自己重写容易在中文与折行上出错。
/// `search_next` 在找不到"更靠后的命中"时会回绕返回区间内第一个命中，
/// 因此用"命中起点不再前进"作为终止条件，而不是依赖返回值是否为空。
pub(crate) fn scan(
    term: &Term<TerminalEventProxy>,
    regex: &mut RegexSearch,
    limit: usize,
) -> (Vec<Hit>, bool) {
    let bottom = Line(term.screen_lines() as i32 - 1);
    let last_column = term.last_column();
    let mut hits: Vec<Hit> = Vec::with_capacity(limit.min(64));
    let mut origin = Point::new(term.topmost_line(), Column(0));

    while hits.len() < limit {
        let Some(found) = term.search_next(regex, origin, Direction::Right, Side::Left, None)
        else {
            break;
        };
        let hit = Hit::from_match(&found);
        // Point 没有实现 Ord，按 (行, 列) 字典序比较才能判断起点是否继续前进。
        if hits.last().is_some_and(|last| {
            (hit.start.line, hit.start.column) <= (last.start.line, last.start.column)
        }) {
            // 回绕：起点不再严格递增，说明整块缓冲已扫完。
            break;
        }
        let reached_bottom = hit.start.line > bottom;
        hits.push(hit);
        if reached_bottom {
            break;
        }
        let end = found.end();
        if end.column == last_column {
            if end.line >= bottom {
                break;
            }
            origin = Point::new(end.line + 1, Column(0));
        } else {
            origin = Point::new(end.line, end.column + 1);
        }
    }

    let truncated = hits.len() == limit;
    (hits, truncated)
}

/// 预填搜索栏的文本：取当前选区首行的前 [`PREFILL_MAX_CHARS`] 个字符。
pub(crate) fn prefill_from_selection(selection: Option<&str>) -> Option<String> {
    let text = selection?;
    let first_line = text.lines().next()?;
    let trimmed = first_line.trim();
    if trimmed.is_empty() || trimmed.chars().count() > PREFILL_MAX_CHARS {
        return None;
    }
    Some(trimmed.to_string())
}

/// 引擎上的检索入口。
///
/// 拆在 `search` 模块而不是留在 `engine`：检索的状态机、扫描算法、滚动定位与绘制数据
/// 是一个内聚整体，留在引擎里会让引擎文件同时承担"网格宿主"和"检索实现"两件事。
impl TerminalEngine {
    /// 更新查询词与正则开关；内容未变时是空操作，避免无谓作废快照。
    pub(crate) fn set_search_query(&mut self, query: &str, use_regex: bool) {
        self.search.set_query(query, use_regex);
    }

    /// 关闭检索并清空全部命中。
    pub(crate) fn clear_search(&mut self) {
        self.search.clear();
    }

    /// 按需重扫整个缓冲区。
    ///
    /// 由渲染帧调用而不是在 `process` 里同步扫描：重扫代价随 scrollback 规模增长，
    /// 放进数据流路径会把高频输出直接转成渲染卡顿。
    pub(crate) fn refresh_search(&mut self, now: Instant) {
        self.do_scan(now, false);
    }

    /// 显式导航前的强制重扫：用户按下回车就是明确意图，不再等输出停歇或冷却到期。
    ///
    /// 不这么做的话，回车会拿着一份已过期的命中列表跳位——那比"还没更新"更糟。
    pub(crate) fn refresh_search_now(&mut self, now: Instant) {
        self.do_scan(now, true);
    }

    fn do_scan(&mut self, now: Instant, force: bool) {
        if !self
            .search
            .scan_allowed(self.epoch, now, force || self.output_quiet(now))
        {
            return;
        }
        let mut regex = match self.search.take_regex() {
            Some(regex) => regex,
            None => return,
        };
        let (hits, truncated) = scan(&self.term, &mut regex, MAX_MATCHES);
        self.search.put_regex(regex);
        self.search.record_scan(self.epoch, hits, truncated, now);
    }

    /// 前进/后退一个命中并把视线移到该处。
    pub(crate) fn search_select(&mut self, forward: bool) -> Option<Hit> {
        self.refresh_search_now(Instant::now());
        let hit = if forward {
            self.search.select_next()
        } else {
            self.search.select_prev()
        };
        if let Some(hit) = hit {
            self.scroll_hit_into_view(hit);
        }
        hit
    }

    /// 按刻度带上的纵向比例跳到最近的命中。
    pub(crate) fn search_select_by_fraction(&mut self, fraction: f32) -> Option<Hit> {
        self.refresh_search_now(Instant::now());
        let top = self.term.grid().topmost_line();
        let bottom = Line(self.term.screen_lines() as i32 - 1);
        let hit = self.search.select_by_fraction(fraction, top, bottom)?;
        self.scroll_hit_into_view(hit);
        Some(hit)
    }

    /// 检索栏状态摘要（含快照是否过期）。
    pub(crate) fn search_status(&self) -> SearchStatus {
        self.search.status(self.epoch)
    }

    /// 生成当前帧的检索绘制数据；快照失效或无查询时返回 `None`（不画任何高亮）。
    pub(crate) fn search_paint(&self, tick_rows: usize) -> Option<SearchPaint> {
        if !self.search.is_active() || self.search.is_stale(self.epoch) {
            return None;
        }
        let grid = self.term.grid();
        let rows = grid.screen_lines();
        let cols = grid.columns();
        let display_offset = grid.display_offset() as i32;
        let top = grid.topmost_line();
        let bottom = Line(rows as i32 - 1);
        Some(
            self.search
                .paint(display_offset, rows, cols, top, bottom, tick_rows),
        )
    }

    /// 输出是否已停歇到可以重扫的程度。
    ///
    /// 全缓冲扫描实测要几十到上百毫秒，落在刷屏的每一帧上会直接把界面拖成幻灯片；
    /// 等待一个停歇窗口后再扫，既保住帧率，也让结果在用户抬头看屏时正好是最新的。
    fn output_quiet(&self, now: Instant) -> bool {
        self.last_write
            .is_none_or(|last| now.saturating_duration_since(last) >= OUTPUT_QUIET_WINDOW)
    }

    /// 命中已在视口内则不动，否则把它滚到接近垂直居中处。
    ///
    /// 不做居中的分支是为了避免"每按一次下一个就整屏跳动"；只有确实看不见才滚。
    fn scroll_hit_into_view(&mut self, hit: Hit) {
        let rows = self.term.screen_lines() as i32;
        let offset = self.term.grid().display_offset() as i32;
        let row = hit.start.line.0 + offset;
        if (0..rows).contains(&row) && hit.end.line.0 + offset < rows {
            return;
        }
        let history = self.term.grid().history_size() as i32;
        let target = (rows / 2 - hit.start.line.0).clamp(0, history);
        self.term.scroll_display(Scroll::Delta(target - offset));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let ticks = build_ticks(&hits, Line(-9), Line(0), 4);
        assert_eq!(ticks.len(), 1);
        let many = build_ticks(&hits, Line(-9), Line(0), 100);
        assert_eq!(many.len(), 2);
    }

    #[test]
    fn empty_tick_rows_yields_no_ticks() {
        let hits = vec![Hit {
            start: Point::new(Line(0), Column(0)),
            end: Point::new(Line(0), Column(1)),
        }];
        assert!(build_ticks(&hits, Line(-1), Line(0), 0).is_empty());
        assert!(build_ticks(&hits, Line(0), Line(0), 10).is_empty());
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
        let long = "x".repeat(PREFILL_MAX_CHARS + 1);
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
}
