//! 多标签并排：N 槽位网格、分组栈布局与标签动作。

mod layout;
mod shell;
pub(crate) use shell::{activate_group, add_groups_to_tiling, apply_tiling_layout_for, clear_tab_selection, close_tiling_group, close_tiling_groups, fill_tiling_slot, focus_neighbor_tile, tile_tab, toggle_tab_selection, untile_tiling};
mod strip;
mod tabs;
mod types;

#[cfg(test)]
mod tests;

pub(crate) use layout::{auto_tiling_grid, tiling_layout_options};
pub(crate) use tabs::TiledTabs;
pub(crate) use types::{MAX_TILING_SLOTS, StripEntry, TilingId, TilingLayoutChoice, TilingNode};
