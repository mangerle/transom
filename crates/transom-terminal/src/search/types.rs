//! 终端检索数据契约与命中模型。

use alacritty_terminal::index::{Line, Point};
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
pub(crate) const RESCAN_INTERVAL: Duration = Duration::from_millis(300);

/// 把选中文字预填进搜索栏时的长度上限，避免把整段缓冲区当成查询串。
pub(crate) const PREFILL_MAX_CHARS: usize = 128;

/// 正则元字符集，与 `regex::escape` 的字面量化口径一致（关键词模式使用）。
const REGEX_METACHARS: &[u8] = b"\\.+*?()|[]{}^$#&-~";

/// 将用户输入按字面量编译，避免关键词模式里的 `.`、`[` 被当成正则元字符。
pub(crate) fn escape_literal(query: &str) -> String {
    let mut escaped = String::with_capacity(query.len() + query.len() / 2);
    for ch in query.chars() {
        if REGEX_METACHARS.contains(&(ch as u8)) && ch.is_ascii() {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

/// 把列区间夹进 `[0, cols)`，返回 `None` 表示该段不可见。
pub(crate) fn clamp_span(start: usize, end: usize, cols: usize) -> Option<usize> {
    let end = end.min(cols);
    (start < cols && start < end).then_some(end)
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

/// 单个命中在网格坐标系里的区间（跨行命中保留起止两端）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Hit {
    pub(crate) start: Point,
    pub(crate) end: Point,
}

impl Hit {
    pub(crate) fn from_match(found: &Match) -> Self {
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

        let ticks = crate::search_ticks::build_ticks(
            self.hits.iter().map(|hit| hit.start.line.0),
            top,
            bottom,
            tick_rows,
        );
        SearchPaint {
            rows: row_paints,
            ticks,
        }
    }
}
