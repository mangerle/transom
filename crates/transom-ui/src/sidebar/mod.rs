//! 会话树侧边栏：搜索、文件夹树渲染与根层级拖放区域。

mod folders;
mod state;
mod tree;

pub(crate) use folders::FolderForm;
pub(crate) use state::{SidebarRowsCache, SidebarState};

/// 侧栏工具图标按钮配置属性。
pub(crate) struct SidebarButtonProps<F> {
    pub(crate) id: &'static str,
    pub(crate) icon: IconName,
    pub(crate) tooltip: String,
    pub(crate) active: bool,
    pub(crate) on_click: F,
}

use crate::app::{AppShell, AppState};
use crate::divider::{DividerEdge, DividerStyled};
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::{Sizable, v_virtual_list};
use gpui_kit::*;
use std::rc::Rc;
use transom_core::SessionId;
pub(crate) use tree::SidebarRow;
pub(crate) use tree::{
    BAND_HEIGHT, ROW_CHEVRON_SIZE, ROW_HEIGHT, ROW_ICON_SIZE, SidebarDragPreview, indent_padding,
};
use tree::{SidebarDrag, SidebarTarget, build_rows};

impl AppState {
    pub(crate) fn sidebar_rows(&mut self, query: &str) -> Rc<Vec<SidebarRow>> {
        if let Some(cache) = &self.sidebar.rows_cache
            && cache.revision == self.catalog_revision
            && cache.query == query
        {
            return cache.rows.clone();
        }
        let rows = Rc::new(build_rows(&self.catalog, query));
        self.sidebar.rows_cache = Some(SidebarRowsCache {
            revision: self.catalog_revision,
            query: query.to_string(),
            rows: rows.clone(),
        });
        rows
    }

    pub(crate) fn active_session_id(&self) -> Option<SessionId> {
        self.workspace
            .active_tab
            .and_then(|tab_id| self.workspace.tab(tab_id).map(|tab| tab.session_id))
    }
}

impl AppShell {
    /// 侧栏行快照：目录版本与搜索词都没变时复用上次构建结果。
    ///
    /// 设计原理（Why）：render 在流量/统计事件触发时整壳重建，行构建是
    /// O(会话数) 的分配与遍历；缓存把"数据没变"的重绘成本降到一次 Rc
    /// clone。失效键用 persist_catalog 推进的目录版本——目录的每次修改
    /// 都以持久化为终点（见 AppShell 字段注释）。
    pub(crate) fn render_sidebar(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let query = self.state.sidebar.search.read(cx).value().to_string();
        // 目录与搜索词都没变时复用上次构建的行表（见 `sidebar_rows`）。
        let rows = self.state.sidebar_rows(&query);
        let content = if rows.is_empty() {
            self.render_sidebar_empty(&query, p)
        } else {
            let item_sizes = Rc::new(
                rows.iter()
                    .map(|row| size(px(0.0), row.height()))
                    .collect::<Vec<_>>(),
            );
            let p = p.clone();
            v_virtual_list(
                cx.entity().clone(),
                "sidebar-sessions-virtual-list",
                item_sizes,
                move |this, range, _window, cx| {
                    let active_session = this.state.active_session_id();
                    range
                        .filter_map(|index| {
                            let row = rows.get(index)?;
                            match row {
                                SidebarRow::Folder(row) => {
                                    Some(this.render_folder_row(row, &p, cx))
                                }
                                SidebarRow::Session(row) => {
                                    Some(this.render_session_row(row, active_session, &p, cx))
                                }
                            }
                        })
                        .collect()
                },
            )
            .into_any_element()
        };

        let app_entity = cx.entity().clone();
        let header = self.render_sidebar_header(p, cx);

        v_flex()
            .size_full()
            .overflow_hidden()
            .bg(p.sidebar)
            .text_color(p.sidebar_foreground)
            .child(header)
            .child(
                v_flex()
                    .id("session-list")
                    .flex_1()
                    .w_full()
                    .overflow_hidden()
                    .p_2()
                    .on_drop(move |drag: &SidebarDrag, window, cx| {
                        let drag = *drag;
                        app_entity.update(cx, |this, cx| {
                            this.handle_sidebar_drop(drag, SidebarTarget::Root, cx);
                            window.refresh();
                        });
                    })
                    .child(content),
            )
            .into_any_element()
    }

    fn sidebar_icon_button(
        props: SidebarButtonProps<impl Fn(&ClickEvent, &mut Window, &mut App) + 'static>,
        _p: &Palette,
    ) -> AnyElement {
        let SidebarButtonProps {
            id,
            icon,
            tooltip,
            active,
            on_click,
        } = props;
        crate::widgets::IconButton::new(id, icon, tooltip)
            .selected(active)
            .render()
            .on_click(on_click)
            .into_any_element()
    }

    /// 侧栏头部：搜索框、选择/新建按钮、筛选与新建会话/批量入口。
    fn render_sidebar_header(&self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let mut header = v_flex()
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
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&self.state.sidebar.search).w_full()),
                    )
                    .child(Self::sidebar_icon_button(
                        SidebarButtonProps {
                            id: "sidebar-select",
                            icon: IconName::ListChecks,
                            tooltip: rust_i18n::t!("batch.select_mode").to_string(),
                            active: self.state.sidebar.selection_mode,
                            on_click: cx.listener(|this, _, _, cx| {
                                this.toggle_selection_mode(cx);
                            }),
                        },
                        p,
                    ))
                    .child(Self::sidebar_icon_button(
                        SidebarButtonProps {
                            id: "sidebar-new-folder",
                            icon: IconName::FolderPlus,
                            tooltip: rust_i18n::t!("folder.new_title").to_string(),
                            active: false,
                            on_click: cx.listener(|this, _, window, cx| {
                                this.open_new_folder(None, window, cx);
                            }),
                        },
                        p,
                    ))
                    .child(Self::sidebar_icon_button(
                        SidebarButtonProps {
                            id: "sidebar-new",
                            icon: IconName::Plus,
                            tooltip: rust_i18n::t!("app.new_session").to_string(),
                            active: false,
                            on_click: cx.listener(|this, _, window, cx| {
                                this.open_new_session(window, cx);
                            }),
                        },
                        p,
                    )),
            );
        if self.state.sidebar.selection_mode {
            header = header.child(self.render_batch_selection_bar(p, cx));
        }
        header.into_any_element()
    }

    /// 批量选择状态条：已选数量、全选与批量编辑入口。
    fn render_batch_selection_bar(&self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let selected_count = self.state.sidebar.selected_sessions.len();
        h_flex()
            .w_full()
            .justify_between()
            .items_center()
            .child(
                div().text_xs().text_color(p.muted_foreground).child(
                    rust_i18n::t!("batch.selected_count", count = selected_count).to_string(),
                ),
            )
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("batch-select-all")
                            .ghost()
                            .xsmall()
                            .label(rust_i18n::t!("batch.select_all").to_string())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.select_all_sessions(cx);
                            })),
                    )
                    .child(
                        Button::new("batch-open")
                            .primary()
                            .xsmall()
                            .label(rust_i18n::t!("batch.edit").to_string())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_batch_edit(window, cx);
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_sidebar_empty(&self, query: &str, p: &Palette) -> AnyElement {
        let (icon, title, hint) = if !query.trim().is_empty() {
            (
                IconName::Search,
                rust_i18n::t!("app.no_matches").to_string(),
                rust_i18n::t!("app.no_matches_hint").to_string(),
            )
        } else {
            (
                IconName::Server,
                rust_i18n::t!("app.no_sessions").to_string(),
                rust_i18n::t!("app.no_sessions_hint").to_string(),
            )
        };
        crate::widgets::empty_state(icon, title, hint, p)
    }
}
