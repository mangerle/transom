//! 终端缓冲区全文检索模块。
//!
//! 负责把查询编译成 DFA，扫描整个滚动缓冲区得到命中快照并映射为绘制数据。

mod scan;
mod types;

#[cfg(test)]
mod tests;

pub(crate) use types::{RowPaint, SearchPaint, SearchState, SearchStatus, prefill_from_selection};
