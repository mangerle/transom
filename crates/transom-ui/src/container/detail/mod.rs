//! 容器详情区：基础信息、日志、终端、统计与文件预览。

use crate::container::types::{
    ContainerConfirm, ContainerDetailTab, ContainerSection, LOG_RENDER_LIMIT, LOG_TAIL_CHOICES,
    filtered_log_lines,
};
use crate::container::{ContainerWorkspace, ContainerWorkspaceState};
use crate::divider::{DividerEdge, DividerStyled};
use crate::palette::Palette;
use crate::widgets::action_button;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::{ContextMenuExt, DropdownMenu, PopupMenuItem};
use gpui_kit::*;
use transom_core::{
    ComposeProject, ContainerDetails, ContainerFileEntry, ContainerFileKind, ContainerImageDetails,
    ContainerNetworkDetails, ContainerStats, ContainerSummary, ContainerVolumeDetails, TabId,
};
use transom_ui_icons::AppIcon;
use transom_ui_icons::CustomIcon;
use transom_ui_icons::DomainIcon;
use transom_ui_icons::UiIcon;

mod changes;
mod compose;
pub(crate) mod compose_graph;
mod compose_topology;
pub(crate) mod compose_yaml;
mod file_preview;
mod files;
mod header;
mod header_actions;
mod header_container;
mod image;
mod image_history;
mod logs;
mod network;
mod overview;
mod overview_cards;
mod raw_inspect;
mod render;
mod tab_strip;
mod terminal;
pub(crate) mod timeline;
mod volume;

pub(super) use compose::render_compose_detail;
use files::ContainerFilesView;
pub(super) use files::render_files;
pub(super) use header::detail_icon;
use header::{ContainerHeaderView, render_detail_header};
pub(crate) use image::render_image_detail;
#[cfg(test)]
pub(crate) use image_history::clean_dockerfile_instruction;
use logs::ContainerLogsView;
pub(super) use logs::render_logs;
pub(super) use network::{ContainerNetworkView, render_network_detail};
pub(super) use overview::{
    centered_hint, format_unix, kv, linked_containers, render_container_overview, scroll_body,
    short,
};
pub(super) use raw_inspect::{InspectSearch, render_raw_inspect};
pub(super) use render::render_container_detail;
use tab_strip::render_detail_tab_buttons;
pub(super) use terminal::render_terminal;
pub(super) use volume::{ContainerVolumeView, render_volume_detail};

/// tar 头里的权限位转成常见的 `rwxr-xr-x` 形式，只保留读写执行九位。
fn mode_label(mode: u32) -> String {
    const BITS: [(char, u32); 9] = [
        ('r', 0o400),
        ('w', 0o200),
        ('x', 0o100),
        ('r', 0o040),
        ('w', 0o020),
        ('x', 0o010),
        ('r', 0o004),
        ('w', 0o002),
        ('x', 0o001),
    ];
    let mut text = String::with_capacity(9);
    for (glyph, bit) in BITS {
        text.push(if mode & bit != 0 { glyph } else { '-' });
    }
    text
}

/// 返回容器文件路径的父目录；根目录与空路径统一回落到 `/`。
pub(crate) fn parent_path(path: &str) -> String {
    if path == "/" || path.is_empty() {
        return "/".to_string();
    }
    match path.rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(index) => path[..index].to_string(),
    }
}

/// 删除按钮：把待确认动作写回工作区实体，确认弹窗仍由外壳统一渲染。
pub(super) fn delete_button(
    view: &Entity<ContainerWorkspace>,
    id: &'static str,
    confirm: ContainerConfirm,
) -> AnyElement {
    let view = view.clone();
    action_button(
        id,
        UiIcon::DELETE,
        rust_i18n::t!("common.delete").to_string(),
    )
    .danger()
    .on_click(move |_, _, cx| {
        let confirm = confirm.clone();
        view.update(cx, |workspace, cx| {
            workspace.state.modals.confirm = Some(confirm);
            cx.notify();
        });
    })
    .into_any_element()
}

/// 容器详情区的渲染参数。
///
/// 设计原理（Why）：`InputState`/`TextareaState` 的当前值必须经 `App` 读取，
/// 纯渲染函数拿不到 `App`；外壳在进入详情渲染前把关键词与编辑器文本快照
/// 进来，实体句柄与状态快照仍按借用传递，避免克隆整份详情状态。
pub(super) struct ContainerDetailParams<'a> {
    /// 所属标签页 id：变更页的按钮标识需要按标签区分。
    pub(super) tab_id: TabId,
    pub(super) view: &'a Entity<ContainerWorkspace>,
    pub(super) workspace: &'a ContainerWorkspaceState,
    /// Compose YAML 编辑器当前文本快照。
    pub(super) compose_yaml_text: &'a str,
    /// 日志搜索关键词快照。
    pub(super) logs_filter: &'a str,
    /// 变更页搜索关键词快照。
    pub(super) changes_search_value: &'a str,
    /// 原始 inspect 搜索关键词快照。
    pub(super) inspect_keyword: &'a str,
}
