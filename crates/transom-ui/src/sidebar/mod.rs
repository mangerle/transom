//! 会话树侧边栏：搜索、文件夹树渲染与根层级拖放区域。

mod bridge;
mod entity;
mod slot;
mod state;
mod tree;

pub(crate) use bridge::{on_session_sidebar_event, render_folder_form_body, save_folder_form};
pub(crate) use entity::{SessionSidebarEvent, SessionSidebarView};
pub(crate) use slot::{SidebarSlotLayout, SidebarSlotSync, sync_sidebar_slot};
pub(crate) use state::{SidebarRowsCache, SidebarSection, SidebarState};

/// 新建/编辑文件夹的模态表单；`base` 为编辑目标，`parent` 为新建位置。
pub(crate) struct FolderForm {
    pub(crate) base: Option<FolderId>,
    pub(crate) parent: Option<FolderId>,
    pub(crate) name: Entity<InputState>,
    pub(crate) default_port: Entity<InputState>,
    pub(crate) default_username: Entity<InputState>,
    pub(crate) default_key_path: Entity<InputState>,
    pub(crate) error: Option<String>,
}

impl FolderForm {
    /// 表单全部文本项的当前值，用作脏判定基线。
    pub(crate) fn text_values(&self, cx: &App) -> Vec<String> {
        [
            &self.name,
            &self.default_port,
            &self.default_username,
            &self.default_key_path,
        ]
        .into_iter()
        .map(|input| input.read(cx).value().to_string())
        .collect()
    }
}

/// 侧栏工具图标按钮配置属性。
pub(crate) struct SidebarButtonProps<F> {
    pub(crate) id: &'static str,
    pub(crate) icon: AppIcon,
    pub(crate) tooltip: String,
    pub(crate) active: bool,
    pub(crate) on_click: F,
}

use crate::app::AppState;
use crate::divider::{DividerEdge, DividerStyled};
use crate::palette::Palette;
use crate::probe::ProbeBadge;
use crate::widgets::sidebar_list_frame;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{Sizable, v_virtual_list};
use gpui_kit::*;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use transom_core::{FolderId, SessionId};
use transom_ui_icons::{AppIcon, UiIcon};
pub(crate) use tree::{
    ROW_CHEVRON_SIZE, ROW_HEIGHT_REM, ROW_ICON_SIZE, SidebarDragPreview, SidebarRow, indent_padding,
};
use tree::{
    SessionRowRenderParams, SidebarDrag, SidebarTarget, build_rows, render_folder_row,
    render_session_row,
};

/// 渲染会话侧栏所需的只读参数。
pub(crate) struct SessionSidebarParams<'a> {
    pub(crate) view: &'a Entity<SessionSidebarView>,
    pub(crate) search: &'a Entity<InputState>,
    pub(crate) rows: Rc<Vec<SidebarRow>>,
    pub(crate) query: &'a str,
    pub(crate) selection_mode: bool,
    pub(crate) selected_sessions: &'a HashSet<SessionId>,
    pub(crate) tree_selection: Option<SessionId>,
    pub(crate) active_session: Option<SessionId>,
    pub(crate) probe_badges: &'a HashMap<SessionId, ProbeBadge>,
}

impl AppState {
    pub(crate) fn sidebar_rows(&mut self, query: &str) -> Rc<Vec<SidebarRow>> {
        if let Some(cache) = &self.session.sidebar.rows_cache
            && cache.revision == self.session.catalog_revision
            && cache.query == query
        {
            return cache.rows.clone();
        }
        let rows = Rc::new(build_rows(&self.session.catalog, query));
        self.session.sidebar.rows_cache = Some(SidebarRowsCache {
            revision: self.session.catalog_revision,
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

/// 渲染会话侧栏主体（头部工具条 + 虚拟列表 + 根层级拖放区）。
pub(crate) fn render_sidebar(
    params: SessionSidebarParams<'_>,
    window: &Window,
    p: &Palette,
) -> AnyElement {
    let view = params.view.clone();
    let content = if params.rows.is_empty() {
        render_sidebar_empty(params.query, p)
    } else {
        let item_sizes = Rc::new(
            params
                .rows
                .iter()
                .map(|row| size(px(0.0), row.height(window.rem_size())))
                .collect::<Vec<_>>(),
        );
        let rows = params.rows.clone();
        let active_session = params.active_session;
        let selection_mode = params.selection_mode;
        let selected_sessions = params.selected_sessions.clone();
        let tree_selection = params.tree_selection;
        let probe_badges = params.probe_badges.clone();
        let p = p.clone();
        v_virtual_list(
            view.clone(),
            "sidebar-sessions-virtual-list",
            item_sizes,
            move |view_ref, range, _window, cx| {
                let view_handle = cx.entity();
                let _ = view_ref;
                range
                    .filter_map(|index| {
                        let row = rows.get(index)?;
                        match row {
                            SidebarRow::Folder(row) => {
                                Some(render_folder_row(&view_handle, row, &p))
                            }
                            SidebarRow::Session(row) => {
                                let session_id = row.session.id;
                                Some(render_session_row(
                                    SessionRowRenderParams {
                                        view: &view_handle,
                                        row,
                                        active_session,
                                        selection_mode,
                                        is_selected: selected_sessions.contains(&session_id),
                                        is_marked: tree_selection == Some(session_id),
                                        probe_badge: probe_badges.get(&session_id),
                                    },
                                    &p,
                                ))
                            }
                        }
                    })
                    .collect()
            },
        )
        .into_any_element()
    };

    let header = render_sidebar_header(&params, p);
    let drop_view = view;

    v_flex()
        .size_full()
        .overflow_hidden()
        .bg(p.background)
        .text_color(p.foreground)
        .child(header)
        .child(
            sidebar_list_frame("session-list")
                .flex_1()
                .overflow_hidden()
                .on_drop(move |drag: &SidebarDrag, window, cx| {
                    let drag = *drag;
                    drop_view.update(cx, |this, cx| {
                        this.emit_action(SessionSidebarEvent::Drop(drag, SidebarTarget::Root), cx);
                    });
                    window.refresh();
                })
                .child(content),
        )
        .into_any_element()
}

fn sidebar_icon_button(
    props: SidebarButtonProps<impl Fn(&ClickEvent, &mut Window, &mut App) + 'static>,
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
fn render_sidebar_header(params: &SessionSidebarParams<'_>, p: &Palette) -> AnyElement {
    let emit = |event: SessionSidebarEvent| {
        let view = params.view.clone();
        move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
            view.update(cx, |this, cx| this.emit_action(event.clone(), cx));
        }
    };

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
                .child(div().flex_1().child(Input::new(params.search).w_full()))
                .child(sidebar_icon_button(SidebarButtonProps {
                    id: "sidebar-select",
                    icon: AppIcon::Ui(UiIcon::CHECK),
                    tooltip: rust_i18n::t!("batch.select_mode").to_string(),
                    active: params.selection_mode,
                    on_click: emit(SessionSidebarEvent::ToggleSelectionMode),
                }))
                .child(sidebar_icon_button(SidebarButtonProps {
                    id: "sidebar-new-folder",
                    icon: AppIcon::Ui(UiIcon::NEW_FOLDER),
                    tooltip: rust_i18n::t!("sidebar.new_group").to_string(),
                    active: false,
                    on_click: emit(SessionSidebarEvent::OpenNewFolder(None)),
                }))
                .child(sidebar_icon_button(SidebarButtonProps {
                    id: "sidebar-new",
                    icon: AppIcon::Ui(UiIcon::ADD),
                    tooltip: rust_i18n::t!("app.new_session").to_string(),
                    active: false,
                    on_click: emit(SessionSidebarEvent::OpenNewSession(None)),
                })),
        );
    if params.selection_mode {
        header = header.child(render_batch_selection_bar(
            params.view,
            params.selected_sessions.len(),
            p,
        ));
    }
    header.into_any_element()
}

/// 批量选择状态条：已选数量、全选与批量编辑入口。
fn render_batch_selection_bar(
    view: &Entity<SessionSidebarView>,
    selected_count: usize,
    p: &Palette,
) -> AnyElement {
    let select_all_view = view.clone();
    let edit_view = view.clone();
    h_flex()
        .w_full()
        .justify_between()
        .items_center()
        .child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("batch.selected_count", count = selected_count).to_string()),
        )
        .child(
            h_flex()
                .gap_1()
                .child(
                    Button::new("batch-select-all")
                        .ghost()
                        .xsmall()
                        .label(rust_i18n::t!("batch.select_all").to_string())
                        .on_click(move |_, _, cx| {
                            select_all_view.update(cx, |this, cx| {
                                this.emit_action(SessionSidebarEvent::SelectAllSessions, cx);
                            });
                        }),
                )
                .child(
                    Button::new("batch-open")
                        .primary()
                        .xsmall()
                        .label(rust_i18n::t!("batch.edit").to_string())
                        .on_click(move |_, _, cx| {
                            edit_view.update(cx, |this, cx| {
                                this.emit_action(SessionSidebarEvent::OpenBatchEdit, cx);
                            });
                        }),
                ),
        )
        .into_any_element()
}

fn render_sidebar_empty(query: &str, p: &Palette) -> AnyElement {
    let (icon, title, hint) = if !query.trim().is_empty() {
        (
            AppIcon::Ui(UiIcon::SEARCH),
            rust_i18n::t!("app.no_matches").to_string(),
            rust_i18n::t!("app.no_matches_hint").to_string(),
        )
    } else {
        (
            AppIcon::Ui(UiIcon::PROJECT),
            rust_i18n::t!("app.no_sessions").to_string(),
            rust_i18n::t!("app.no_sessions_hint").to_string(),
        )
    };
    crate::widgets::empty_state(icon, title, hint, p)
}
