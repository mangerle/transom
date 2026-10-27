//! 终端全缓冲区扫描与命中提取算法。

use super::types::{Hit, MAX_MATCHES};
use crate::engine::TerminalEngine;
use crate::event::TerminalEventProxy;
use crate::search::types::{OUTPUT_QUIET_WINDOW, SearchPaint, SearchStatus};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Direction, Line, Point, Side};
use alacritty_terminal::term::Term;
use alacritty_terminal::term::search::RegexSearch;
use std::time::Instant;

/// 用编译好的自动机把整个缓冲区扫一遍。
///
/// 逐次调用 `Term::search_next` 而不是自己拼行文本再匹配：前者已正确处理宽字符、
/// 属性与边界折叠，且内部维护了 DFA 状态缓存。
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

/// 引擎上的检索方法挂载。
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
