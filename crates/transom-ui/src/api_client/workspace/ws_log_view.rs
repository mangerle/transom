//! WebSocket 消息日志实体。
//!
//! 设计原理（Why）：消息可能持续高速到达。若每批都重建完整文本并刷新
//! `TextareaState`，帧成本会随日志长度增长。这里保留有界行缓冲，并用虚拟列表
//! 只渲染可见行；复制按钮按需生成一次性完整文本。
//!
//! 代价与局限：单行过长时列表显示会被截断，复制仍保留完整内容。

use crate::palette::Palette;
use crate::theme::tokens::{rem_px, size, space, typography};
use gpui_kit::component::{VirtualListScrollHandle, v_virtual_list};
use gpui_kit::*;
use std::collections::VecDeque;
use std::rc::Rc;

/// WebSocket 日志有界环形缓冲区上限。
const MAX_WS_LOG_LINES: usize = 1_000;
const MAX_WS_LOG_BYTES: usize = 256 * 1024;

/// 有界 WebSocket 消息日志缓冲区，避免全量回读与无上限内存增长。
struct WsLogBuffer {
    lines: Rc<VecDeque<String>>,
    total_bytes: usize,
}

impl Default for WsLogBuffer {
    fn default() -> Self {
        Self {
            lines: Rc::new(VecDeque::new()),
            total_bytes: 0,
        }
    }
}

impl WsLogBuffer {
    /// 批量追加日志并淘汰最旧内容。
    fn push_many(&mut self, new_lines: impl IntoIterator<Item = String>) {
        let lines = Rc::make_mut(&mut self.lines);
        for line in new_lines {
            let line = truncate_log_tail(line, MAX_WS_LOG_BYTES);
            self.total_bytes = self.total_bytes.saturating_add(line.len());
            lines.push_back(line);
        }
        while (lines.len() > MAX_WS_LOG_LINES || self.total_bytes > MAX_WS_LOG_BYTES)
            && lines.len() > 1
        {
            if let Some(oldest) = lines.pop_front() {
                self.total_bytes = self.total_bytes.saturating_sub(oldest.len());
            }
        }
    }

    /// 共享当前行快照，供虚拟列表在渲染期零拷贝读取。
    fn snapshot(&self) -> Rc<VecDeque<String>> {
        Rc::clone(&self.lines)
    }

    /// 生成复制到剪贴板所需的完整文本快照。
    fn to_text(&self) -> String {
        let mut out = String::with_capacity(self.total_bytes + self.lines.len());
        for (index, item) in self.lines.iter().enumerate() {
            if index > 0 {
                out.push('\n');
            }
            out.push_str(item);
        }
        out
    }

    fn len(&self) -> usize {
        self.lines.len()
    }
}

fn truncate_log_tail(mut line: String, limit: usize) -> String {
    if line.len() <= limit {
        return line;
    }
    let mut start = line.len().saturating_sub(limit);
    while !line.is_char_boundary(start) {
        start = start.saturating_add(1);
    }
    line.drain(..start);
    line
}

/// WebSocket 消息日志实体；生命周期跟随单个 API 工作区。
pub(in crate::app::api_client) struct WsLogView {
    buffer: WsLogBuffer,
    scroll: VirtualListScrollHandle,
}

impl WsLogView {
    pub(in crate::app::api_client) fn new() -> Self {
        Self {
            buffer: WsLogBuffer::default(),
            scroll: VirtualListScrollHandle::new(),
        }
    }

    /// 追加消息；新增内容后自动滚到末尾。
    pub(in crate::app::api_client) fn push_many(
        &mut self,
        lines: impl IntoIterator<Item = String>,
        cx: &mut Context<Self>,
    ) {
        self.buffer.push_many(lines);
        let last = self.buffer.len().saturating_sub(1);
        self.scroll.scroll_to_item(last, ScrollStrategy::Bottom);
        cx.notify();
    }

    /// 复制用完整文本快照。
    pub(in crate::app::api_client) fn to_text(&self) -> String {
        self.buffer.to_text()
    }
}

impl Render for WsLogView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::from(cx);
        let lines = self.buffer.snapshot();
        if lines.is_empty() {
            return div().size_full().into_any_element();
        }
        let row_height = rem_px(size::COMPACT_TREE_ROW_HEIGHT, window.rem_size());
        let item_sizes = Rc::new(vec![size(px(0.0), row_height); lines.len()]);
        let scroll = self.scroll.clone();
        v_virtual_list(
            cx.entity().clone(),
            "api-response-ws-log",
            item_sizes,
            move |_view, range, _window, _cx| {
                range
                    .filter_map(|index| {
                        let line = lines.get(index)?;
                        Some(
                            div()
                                .h(row_height)
                                .w_full()
                                .px(rems(space::SM))
                                .text_size(rems(typography::CAPTION))
                                .text_color(p.foreground)
                                .whitespace_nowrap()
                                .truncate()
                                .child(line.clone()),
                        )
                    })
                    .collect()
            },
        )
        .track_scroll(&scroll)
        .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_WS_LOG_BYTES, MAX_WS_LOG_LINES, WsLogBuffer};

    #[test]
    fn ws_log_buffer_evicts_oldest_when_exceeding_limits() {
        let mut buffer = WsLogBuffer::default();
        for index in 0..(MAX_WS_LOG_LINES + 5) {
            buffer.push_many([format!("line-{index}")]);
        }
        assert_eq!(buffer.lines.len(), MAX_WS_LOG_LINES);
        assert_eq!(buffer.lines.front().map(String::as_str), Some("line-5"));
        let oversized = "x".repeat(MAX_WS_LOG_BYTES.saturating_add(100));
        buffer.push_many([oversized]);
        let text = buffer.to_text();
        assert_eq!(buffer.lines.len(), 1);
        assert!(text.len() <= MAX_WS_LOG_BYTES);
    }
}
