//! 终端输出关键词规则集高亮子模块。

pub mod engine;
pub mod types;

#[cfg(test)]
mod tests;

pub use engine::CompiledHighlight;
pub use types::{HighlightKind, HighlightSpan};
