//! 多标签并排：N 槽位网格、分组栈布局与标签动作。

mod layout;
mod shell;
mod tabs;
mod types;

#[cfg(test)]
mod tests;

pub(crate) use layout::{auto_tiling_grid, tiling_layout_options};
pub(crate) use tabs::TiledTabs;
pub(crate) use types::{MAX_TILING_SLOTS, StripEntry, TilingId, TilingLayoutChoice, TilingNode};
