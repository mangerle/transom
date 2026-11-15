//! 容器引擎侧栏：按会话树的形态渲染引擎分组、连接行与拖拽排序。
//!
//! 设计原理（Why）：引擎连接与会话一样是「经常打开、偶尔整理」的资源，行形态、
//! 选中态、双击打开、右键菜单与拖拽排序都沿用会话树，用户只需学一次。分组数据
//! 落在 `SessionCatalog.engine_folders`，与会话文件夹同构；目录快照由外壳在渲染
//! 期注入，侧栏实体只持有搜索、选中、表单与发现面板等界面状态。
//!
//! 代价与局限：引擎数量通常很少，没有采用会话树的虚拟列表，直接一次性渲染；
//! 行度量与操作按钮复用会话树的实现，避免两套视觉漂移。

mod drag;
mod empty;
mod engine_row;
mod entity;
mod folder_form;
mod folder_row;
mod rows;

pub(crate) use drag::*;
pub(crate) use entity::{ContainerSidebarEvent, ContainerSidebarView, ContainerSidebarViewState};
pub(crate) use folder_form::render_engine_folder_form_body;
pub(crate) use rows::*;

use crate::container::types::{ContainerDiscovery, ContainerWorkspaceStatus};
use crate::divider::{DividerEdge, DividerStyled};
use crate::palette::Palette;
use crate::sidebar::{
    ROW_CHEVRON_SIZE, ROW_HEIGHT_REM, ROW_ICON_SIZE, SidebarDragPreview, indent_padding,
};
use crate::widgets::{IconButton, IconButtonSize, sidebar_list_frame};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::*;
use std::collections::{HashMap, HashSet};
use transom_core::{
    ContainerEngineProfile, DropSide, EngineFolder, EngineProfileId, FolderId, SessionCatalog,
};
use transom_ui_icons::AppIcon;
use transom_ui_icons::UiIcon;

/// 侧栏拖动载荷：引擎连接与分组共用一条拖放通道。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContainerSidebarDrag {
    Engine(EngineProfileId),
    Folder(FolderId),
}

/// 侧栏拖放落点：根层级、某个分组内部，或锚点行前/后。
#[derive(Debug, Clone, Copy)]
pub(crate) enum ContainerSidebarTarget {
    Root,
    Folder(FolderId),
    Relative {
        anchor: ContainerSidebarDrag,
        side: DropSide,
    },
}

#[derive(Clone)]
pub(crate) struct EngineFolderRow {
    id: FolderId,
    name: String,
    depth: usize,
    collapsed: bool,
    engine_count: usize,
    folder_count: usize,
}

#[derive(Clone)]
pub(crate) struct EngineRow {
    profile: Box<ContainerEngineProfile>,
    depth: usize,
}

#[derive(Clone)]
pub(crate) enum ContainerSidebarRow {
    Folder(EngineFolderRow),
    Engine(EngineRow),
}

/// 侧栏渲染参数：目录与连接状态由外壳注入，实体不反查外壳。
pub(crate) struct ContainerSidebarParams<'a> {
    pub(crate) view: &'a Entity<ContainerSidebarView>,
    pub(crate) state: &'a ContainerSidebarViewState,
    pub(crate) cx: &'a App,
    pub(crate) catalog: &'a SessionCatalog,
    /// 当前活动标签对应的引擎连接。
    pub(crate) active: Option<EngineProfileId>,
    /// 每个引擎连接当前的工作区状态，用于行首状态点。
    pub(crate) profile_states: &'a HashMap<EngineProfileId, ContainerWorkspaceStatus>,
}

/// 左侧槽位中的引擎连接树。
pub(crate) fn render_container_sidebar(
    params: ContainerSidebarParams<'_>,
    p: &Palette,
) -> AnyElement {
    let ContainerSidebarParams {
        view,
        state,
        cx,
        catalog,
        active,
        profile_states,
    } = params;
    let query = state.search.read(cx).value().trim().to_string();
    let rows = build_engine_rows(catalog, &query);
    let selected = state.selected;
    let discovery = state.discovery.clone();

    let mut list = sidebar_list_frame("container-engine-list");
    if let Some(discovery) = discovery {
        list = list.child(empty::render_container_discovery(
            view, state, &discovery, catalog, p,
        ));
    }
    if rows.is_empty() {
        list = list.child(empty::render_engine_empty(view, &query, p));
    }
    for row in &rows {
        list = list.child(match row {
            ContainerSidebarRow::Folder(row) => folder_row::render_engine_folder_row(view, row, p),
            ContainerSidebarRow::Engine(row) => {
                engine_row::render_engine_row(view, row, active, selected, profile_states, p)
            }
        });
    }

    let search = state.search.clone();
    let view_for_folder = view.clone();
    let view_for_new = view.clone();
    let search_for_clear = search.clone();
    let view_for_clear = view.clone();
    let clear_search = IconButton::new(
        "container-engine-search-clear",
        AppIcon::Ui(UiIcon::CLOSE),
        rust_i18n::t!("container.sidebar.clear_search").to_string(),
    )
    .size(IconButtonSize::RowAction)
    .render()
    .on_click(move |_, window, cx| {
        search_for_clear.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.focus(window, cx);
        });
        view_for_clear.update(cx, |_, cx| cx.notify());
    });
    let search_input = Input::new(&search).w_full().cleanable(false);
    let search_input = if query.is_empty() {
        search_input
    } else {
        search_input.suffix(clear_search)
    };
    v_flex()
        .size_full()
        .child(
            v_flex()
                .w_full()
                .p_3()
                .gap_2()
                .border_color(p.divider(p.sidebar_border))
                .divider_edge(DividerEdge::Bottom, p.divider_width)
                .child(
                    h_flex()
                        .w_full()
                        .gap_1()
                        .items_center()
                        .child(div().flex_1().min_w_0().child(search_input))
                        .child(engine_sidebar_button(
                            "container-engine-folder-new",
                            AppIcon::Ui(UiIcon::NEW_FOLDER),
                            rust_i18n::t!("container.sidebar.new_folder").to_string(),
                            move |_, window, cx| {
                                view_for_folder.update(cx, |sidebar, cx| {
                                    sidebar.open_folder_form(None, None, String::new(), window, cx);
                                });
                            },
                        ))
                        .child(engine_sidebar_button(
                            "container-engine-new",
                            AppIcon::Ui(UiIcon::ADD),
                            rust_i18n::t!("container.sidebar.new").to_string(),
                            move |_, _, cx| {
                                view_for_new.update(cx, |sidebar, cx| {
                                    sidebar.request_new_engine(None, cx);
                                });
                            },
                        )),
                ),
        )
        .child(
            div()
                .id("container-engine-tree")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .child(list),
        )
        .into_any_element()
}
