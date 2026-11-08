//! 容器引擎侧栏：按会话树的形态渲染引擎分组、连接行与拖拽排序。
//!
//! 设计原理（Why）：引擎连接与会话一样是「经常打开、偶尔整理」的资源，行形态、
//! 选中态、双击打开、右键菜单与拖拽排序都沿用会话树，用户只需学一次。分组数据
//! 落在 `SessionCatalog.engine_folders`，与会话文件夹同构。
//!
//! 代价与局限：引擎数量通常很少，没有采用会话树的虚拟列表，直接一次性渲染；
//! 行度量与操作按钮复用会话树的实现，避免两套视觉漂移。

use crate::app::AppShell;
use crate::container::types::{ContainerDiscovery, ContainerWorkspaceStatus, EngineFolderForm};
use crate::divider::{DividerEdge, DividerStyled};
use crate::palette::Palette;
use crate::sidebar::{
    BAND_HEIGHT, ROW_CHEVRON_SIZE, ROW_HEIGHT, ROW_ICON_SIZE, SidebarDragPreview, indent_padding,
};
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use std::collections::{HashMap, HashSet};
use transom_core::{
    CatalogError, ContainerEngineProfile, DropSide, EngineFolder, EngineProfileId, FolderId,
    SessionCatalog,
};

mod actions;
mod drag;
mod empty;
mod engine_row;
mod folder_form;
mod folder_row;
mod rows;

pub(crate) use drag::*;
pub(crate) use rows::*;

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

impl AppShell {
    /// 左侧槽位中的引擎连接树。
    pub(crate) fn render_container_sidebar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let search = self.ensure_engine_search(window, cx);
        let query = search.read(cx).value().trim().to_string();
        let rows = build_engine_rows(&self.state.catalog, &query);
        let states = self.state.container_profile_states(cx);
        let active = self.active_engine_profile_id();
        let selected = self.state.containers.sidebar_selected;
        let discovery = self.state.containers.discovery.clone();

        let mut list = v_flex().gap_0p5().p_2();
        if let Some(discovery) = discovery {
            list = list.child(self.render_container_discovery(discovery, p, cx));
        }
        if rows.is_empty() {
            list = list.child(self.render_engine_empty(&query, p, cx));
        }
        for row in &rows {
            list = list.child(match row {
                ContainerSidebarRow::Folder(row) => self.render_engine_folder_row(row, p, cx),
                ContainerSidebarRow::Engine(row) => {
                    self.render_engine_row(row, active, selected, &states, p, cx)
                }
            });
        }

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
                            .child(div().flex_1().min_w_0().child(Input::new(&search).w_full()))
                            .child(engine_sidebar_button(
                                "container-engine-folder-new",
                                IconName::FolderPlus,
                                rust_i18n::t!("container.sidebar.new_folder").to_string(),
                                cx.listener(|this, _, window, cx| {
                                    this.open_engine_folder_form(None, None, window, cx);
                                }),
                            ))
                            .child(engine_sidebar_button(
                                "container-engine-new",
                                IconName::Plus,
                                rust_i18n::t!("container.sidebar.new").to_string(),
                                cx.listener(|this, _, window, cx| {
                                    this.open_new_container_in_folder(None, window, cx);
                                }),
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

    /// 侧栏搜索框：首次渲染时创建，之后复用同一实体。
    ///
    /// 与会话树同一口径：不额外订阅输入变化，输入框自身的刷新会带动整壳重绘；
    /// 在渲染期订阅并 notify 容易引入重入渲染。
    fn ensure_engine_search(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some(search) = &self.state.containers.sidebar_search {
            return search.clone();
        }
        let search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("container.sidebar.search").to_string())
        });
        self.state.containers.sidebar_search = Some(search.clone());
        search
    }
}
