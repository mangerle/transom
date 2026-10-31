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
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use std::collections::{HashMap, HashSet};
use transom_core::{
    CatalogError, ContainerEngineProfile, DropSide, EngineFolder, EngineProfileId, FolderId,
    SessionCatalog,
};

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
struct EngineFolderRow {
    id: FolderId,
    name: String,
    depth: usize,
    collapsed: bool,
    engine_count: usize,
    folder_count: usize,
}

#[derive(Clone)]
struct EngineRow {
    profile: Box<ContainerEngineProfile>,
    depth: usize,
}

#[derive(Clone)]
enum ContainerSidebarRow {
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
        let rows = build_engine_rows(&self.catalog, &query);
        let states = self.container_profile_states();
        let active = self.active_engine_profile_id();
        let selected = self.containers.sidebar_selected;
        let discovery = self.containers.discovery.clone();

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

    /// 侧栏搜索框：首次渲染时创建并订阅变化，之后复用同一实体。
    fn ensure_engine_search(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some(search) = &self.containers.sidebar_search {
            return search.clone();
        }
        let search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("container.sidebar.search").to_string())
        });
        let subscription = cx.subscribe_in(&search, window, |_, _, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        self.containers.sidebar_search = Some(search.clone());
        self.containers.sidebar_search_subscription = Some(subscription);
        search
    }

    fn active_engine_profile_id(&self) -> Option<EngineProfileId> {
        let tab = self
            .workspace
            .active_tab
            .and_then(|tab_id| self.workspace.tab(tab_id))?;
        match &tab.session.connection {
            transom_core::Connection::ContainerEngine(profile) => Some(profile.id),
            _ => None,
        }
    }

    fn render_engine_row(
        &mut self,
        row: &EngineRow,
        active: Option<EngineProfileId>,
        selected: Option<EngineProfileId>,
        states: &HashMap<EngineProfileId, ContainerWorkspaceStatus>,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let profile = &row.profile;
        let id = profile.id;
        let name = profile.display_name();
        let description = profile.engine.describe();
        let folder_id = profile.folder_id;
        let is_active = active == Some(id);
        let is_selected = selected == Some(id);
        let status_color = match states.get(&id) {
            Some(ContainerWorkspaceStatus::Connected) => p.success,
            Some(ContainerWorkspaceStatus::Connecting) => p.warning,
            Some(ContainerWorkspaceStatus::Failed(_))
            | Some(ContainerWorkspaceStatus::Disconnected) => p.danger,
            None => p.border,
        };
        let app = cx.entity().clone();
        let menu_name = name.clone();
        let menu_endpoint = description.clone();

        let element = div()
            .id(ElementId::Name(format!("container-engine-{id}").into()))
            .w_full()
            .h(ROW_HEIGHT)
            .pl(indent_padding(row.depth))
            .pr(px(4.0))
            .flex()
            .items_center()
            .cursor_pointer()
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                if event.click_count() >= 2 {
                    if let Some(profile) = this
                        .catalog
                        .engines
                        .iter()
                        .find(|profile| profile.id == id)
                        .cloned()
                    {
                        this.open_engine_workspace(profile, window, cx);
                    }
                } else {
                    this.containers.sidebar_selected = Some(id);
                    cx.notify();
                }
            }))
            .on_drag(ContainerSidebarDrag::Engine(id), {
                let drag_label = name.clone();
                move |_, _, _, cx| {
                    cx.new(|_| SidebarDragPreview::new(drag_label.clone(), IconName::Container))
                }
            })
            .on_drop({
                let app = app.clone();
                move |drag: &ContainerSidebarDrag, window, cx| {
                    let drag = *drag;
                    let target = match folder_id {
                        Some(folder_id) => ContainerSidebarTarget::Folder(folder_id),
                        None => ContainerSidebarTarget::Root,
                    };
                    app.update(cx, |this, cx| {
                        this.handle_container_sidebar_drop(drag, target, cx);
                        window.refresh();
                    });
                }
            })
            .context_menu({
                let app = app.clone();
                move |menu, _window, _cx| {
                    let app_entity = app.clone();
                    let mut menu = menu.item(
                        PopupMenuItem::new(rust_i18n::t!("container.sidebar.open").to_string())
                            .icon(IconName::Play)
                            .on_click({
                                let app = app_entity.clone();
                                move |_, window, cx| {
                                    app.update(cx, |this, cx| {
                                        if let Some(profile) = this
                                            .catalog
                                            .engines
                                            .iter()
                                            .find(|profile| profile.id == id)
                                            .cloned()
                                        {
                                            this.open_engine_workspace(profile, window, cx);
                                        }
                                    });
                                }
                            }),
                    );
                    if folder_id.is_some() {
                        menu = menu.item(
                            PopupMenuItem::new(
                                rust_i18n::t!("container.sidebar.move_to_root").to_string(),
                            )
                            .icon(IconName::FolderTree)
                            .on_click({
                                let app = app_entity.clone();
                                move |_, _, cx| {
                                    app.update(cx, |this, cx| {
                                        this.move_engine_to_folder(id, None, cx);
                                    });
                                }
                            }),
                        );
                    }
                    menu = menu.item(
                        PopupMenuItem::new(rust_i18n::t!("common.edit").to_string())
                            .icon(IconName::Pen)
                            .on_click({
                                let app = app_entity.clone();
                                move |_, window, cx| {
                                    app.update(cx, |this, cx| {
                                        let profile = this
                                            .catalog
                                            .engines
                                            .iter()
                                            .find(|profile| profile.id == id)
                                            .cloned();
                                        this.open_container_form(profile, window, cx);
                                    });
                                }
                            }),
                    );
                    // 复制项在渲染期就把文本带走，菜单回调运行时不再回查目录。
                    let endpoint_app = app_entity.clone();
                    let endpoint = menu_endpoint.clone();
                    menu = menu.item(
                        PopupMenuItem::new(
                            rust_i18n::t!("container.sidebar.copy_endpoint").to_string(),
                        )
                        .icon(IconName::Copy)
                        .on_click(move |_, window, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(endpoint.clone()));
                            let message = rust_i18n::t!(
                                "container.sidebar.endpoint_copied",
                                endpoint = endpoint.as_str()
                            )
                            .to_string();
                            endpoint_app.update(cx, |this, cx| {
                                this.show_toast(message, true, window, cx);
                            });
                        }),
                    );
                    let name_app = app_entity.clone();
                    let copied_name = menu_name.clone();
                    menu = menu.item(
                        PopupMenuItem::new(
                            rust_i18n::t!("container.sidebar.copy_name").to_string(),
                        )
                        .icon(IconName::Copy)
                        .on_click(move |_, window, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(copied_name.clone()));
                            let message = rust_i18n::t!(
                                "container.sidebar.name_copied",
                                name = copied_name.as_str()
                            )
                            .to_string();
                            name_app.update(cx, |this, cx| {
                                this.show_toast(message, true, window, cx);
                            });
                        }),
                    );
                    let delete_app = app_entity.clone();
                    menu.item(PopupMenuItem::separator()).item(
                        PopupMenuItem::new(rust_i18n::t!("common.delete").to_string())
                            .icon(IconName::Trash)
                            .on_click(move |_, window, cx| {
                                delete_app.update(cx, |this, cx| {
                                    this.delete_container_profile(id, window, cx);
                                });
                            }),
                    )
                }
            });

        let mut band = h_flex()
            .id(ElementId::Name(
                format!("container-engine-band-{id}").into(),
            ))
            .relative()
            .w_full()
            .h(BAND_HEIGHT)
            .px_2()
            .gap_2()
            .items_center()
            .rounded(p.radius.md)
            .bg(if is_active || is_selected {
                p.list_active
            } else {
                p.list_background
            })
            .text_color(if is_active {
                p.sidebar_accent_foreground
            } else {
                p.sidebar_foreground
            });
        if is_active {
            band = band.child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .bottom_0()
                    .w(px(2.0))
                    .rounded_full()
                    .bg(p.list_active_border),
            );
        } else if !is_selected {
            band = band.hover(|style| style.bg(p.list_hover));
        }
        let action_color = if is_active {
            p.sidebar_accent_foreground
        } else {
            p.muted_foreground
        };
        band = band
            .child(
                Icon::new(IconName::Container)
                    .with_size(ROW_ICON_SIZE)
                    .text_color(if is_active { action_color } else { p.primary }),
            )
            .child(
                v_flex()
                    .flex_1()
                    .overflow_hidden()
                    .child(
                        div()
                            .text_sm()
                            .line_height(px(17.0))
                            .font_weight(FontWeight::MEDIUM)
                            .truncate()
                            .child(name),
                    )
                    .child(
                        div()
                            .text_xs()
                            .line_height(px(14.0))
                            .truncate()
                            .text_color(p.muted_foreground)
                            .child(description),
                    ),
            )
            .child(
                div()
                    .size(px(6.0))
                    .flex_none()
                    .rounded_full()
                    .bg(status_color),
            )
            .child(engine_reorder_overlay(
                &app,
                ContainerSidebarDrag::Engine(id),
                false,
                p,
            ));
        element.child(band).into_any_element()
    }

    fn render_engine_folder_row(
        &mut self,
        row: &EngineFolderRow,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let folder_id = row.id;
        let app = cx.entity().clone();
        let label = row.name.clone();
        let chevron = if row.collapsed {
            IconName::ChevronRight
        } else {
            IconName::ChevronDown
        };
        let element = div()
            .id(ElementId::Name(
                format!("container-engine-folder-{folder_id}").into(),
            ))
            .w_full()
            .h(ROW_HEIGHT)
            .pl(indent_padding(row.depth))
            .pr(px(4.0))
            .flex()
            .items_center()
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.toggle_engine_folder(folder_id, cx);
            }))
            .on_drag(
                ContainerSidebarDrag::Folder(folder_id),
                move |_, _, _, cx| {
                    cx.new(|_| SidebarDragPreview::new(label.clone(), IconName::Folder))
                },
            )
            .on_drop({
                let app = app.clone();
                move |drag: &ContainerSidebarDrag, window, cx| {
                    let drag = *drag;
                    app.update(cx, |this, cx| {
                        this.handle_container_sidebar_drop(
                            drag,
                            ContainerSidebarTarget::Folder(folder_id),
                            cx,
                        );
                        window.refresh();
                    });
                }
            })
            .context_menu({
                let app = app.clone();
                move |menu, _window, _cx| {
                    let app_entity = app.clone();
                    menu.item(
                        PopupMenuItem::new(
                            rust_i18n::t!("container.sidebar.new_engine_here").to_string(),
                        )
                        .icon(IconName::Plus)
                        .on_click({
                            let app = app_entity.clone();
                            move |_, window, cx| {
                                app.update(cx, |this, cx| {
                                    this.open_new_container_in_folder(Some(folder_id), window, cx);
                                });
                            }
                        }),
                    )
                    .item(
                        PopupMenuItem::new(
                            rust_i18n::t!("container.sidebar.new_subfolder").to_string(),
                        )
                        .icon(IconName::FolderPlus)
                        .on_click({
                            let app = app_entity.clone();
                            move |_, window, cx| {
                                app.update(cx, |this, cx| {
                                    this.open_engine_folder_form(Some(folder_id), None, window, cx);
                                });
                            }
                        }),
                    )
                    .item(
                        PopupMenuItem::new(rust_i18n::t!("container.sidebar.rename").to_string())
                            .icon(IconName::FolderPen)
                            .on_click({
                                let app = app_entity.clone();
                                move |_, window, cx| {
                                    app.update(cx, |this, cx| {
                                        this.open_engine_folder_form(
                                            None,
                                            Some(folder_id),
                                            window,
                                            cx,
                                        );
                                    });
                                }
                            }),
                    )
                    .item(PopupMenuItem::separator())
                    .item(
                        PopupMenuItem::new(
                            rust_i18n::t!("container.sidebar.delete_folder").to_string(),
                        )
                        .icon(IconName::Trash)
                        .on_click({
                            let app = app_entity.clone();
                            move |_, window, cx| {
                                app.update(cx, |this, cx| {
                                    this.delete_engine_folder(folder_id, window, cx);
                                });
                            }
                        }),
                    )
                }
            });

        let into_bg = p.drop_target;
        let hover_bg = p.list_head.blend(p.sidebar_border.opacity(0.6));
        let mut band = h_flex()
            .id(ElementId::Name(
                format!("container-engine-folder-band-{folder_id}").into(),
            ))
            .relative()
            .w_full()
            .h(BAND_HEIGHT)
            .px_2()
            .gap_1()
            .items_center()
            .rounded(p.radius.md)
            .bg(p.list_head)
            .border_b_1()
            .border_color(p.sidebar_border)
            .text_color(p.sidebar_foreground)
            .hover(move |style| style.bg(hover_bg))
            .drag_over::<ContainerSidebarDrag>(move |style, _, _, _| style.bg(into_bg))
            .child(Icon::new(chevron).with_size(ROW_CHEVRON_SIZE))
            .child(
                Icon::new(if row.collapsed {
                    IconName::Folder
                } else {
                    IconName::FolderOpen
                })
                .with_size(ROW_ICON_SIZE),
            )
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .truncate()
                    .child(row.name.clone()),
            )
            .child(div().text_xs().child(row.engine_count.to_string()));
        if row.folder_count > 0 {
            band = band.child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground.opacity(0.7))
                    .child(format!("({})", row.folder_count)),
            );
        }
        let anchor = ContainerSidebarDrag::Folder(folder_id);
        band = band.child(engine_reorder_overlay(&app, anchor, true, p));
        element.child(band).into_any_element()
    }

    fn render_engine_empty(
        &mut self,
        query: &str,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if !query.is_empty() {
            return div()
                .p_2()
                .text_sm()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("container.sidebar.no_match").to_string())
                .into_any_element();
        }
        v_flex()
            .gap_2()
            .p_2()
            .text_sm()
            .text_color(p.muted_foreground)
            .child(rust_i18n::t!("container.sidebar.empty").to_string())
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("container-profile-new-empty")
                            .primary()
                            .small()
                            .icon(IconName::Plus)
                            .label(rust_i18n::t!("container.sidebar.new").to_string())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_new_container_in_folder(None, window, cx);
                            })),
                    )
                    .child(
                        Button::new("container-discover-empty")
                            .small()
                            .icon(IconName::Radar)
                            .label(rust_i18n::t!("container.sidebar.discover").to_string())
                            .on_click(cx.listener(|this, _, _window, cx| {
                                this.discover_engines(cx);
                            })),
                    ),
            )
            .into_any_element()
    }

    /// 发现结果面板：列出可用引擎并给「添加并打开」，同时把不可用原因摊开。
    fn render_container_discovery(
        &mut self,
        discovery: ContainerDiscovery,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let ContainerDiscovery {
            candidates,
            failures,
        } = discovery;
        let mut body = v_flex()
            .gap_1()
            .p_2()
            .rounded(p.radius.md)
            .bg(p.muted.opacity(0.35));
        body = body.child(
            h_flex()
                .gap_1()
                .items_center()
                .justify_between()
                .child(
                    h_flex()
                        .gap_1()
                        .items_center()
                        .child(Icon::new(IconName::Radar).xsmall().text_color(p.primary))
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::BOLD)
                                .child(rust_i18n::t!("container.discovery.title").to_string()),
                        ),
                )
                .child(
                    Button::new("container-discovery-dismiss")
                        .ghost()
                        .xsmall()
                        .icon(IconName::Close)
                        .on_click(cx.listener(|this, _, _window, cx| {
                            this.dismiss_container_discovery(cx);
                        })),
                ),
        );
        if candidates.is_empty() {
            body = body.child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("container.discovery.none").to_string()),
            );
        }
        for candidate in candidates {
            let name = candidate.display_name();
            let endpoint = candidate.engine.describe();
            body = body.child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_xs().truncate().child(name))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(p.muted_foreground)
                                    .truncate()
                                    .child(endpoint),
                            ),
                    )
                    .child(
                        Button::new(ElementId::Name(
                            format!("container-discovery-add-{}", candidate.id).into(),
                        ))
                        .primary()
                        .xsmall()
                        .icon(IconName::Plus)
                        .label(rust_i18n::t!("container.discovery.add").to_string())
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.add_discovered_engine(candidate.clone(), window, cx);
                            },
                        )),
                    ),
            );
        }
        for failure in failures {
            body = body.child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(failure),
            );
        }
        body.into_any_element()
    }

    /// 折叠/展开引擎分组并持久化。
    pub(crate) fn toggle_engine_folder(&mut self, folder_id: FolderId, cx: &mut Context<Self>) {
        let collapsed = self
            .catalog
            .engine_folder(folder_id)
            .map(|folder| !folder.collapsed);
        if let Some(collapsed) = collapsed {
            let _ = self
                .catalog
                .set_engine_folder_collapsed(folder_id, collapsed);
            self.persist_catalog();
        }
        cx.notify();
    }

    /// 打开引擎分组的新建/重命名弹窗。
    pub(crate) fn open_engine_folder_form(
        &mut self,
        parent: Option<FolderId>,
        editing: Option<FolderId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = editing
            .and_then(|id| self.catalog.engine_folder(id))
            .map(|folder| folder.name.clone())
            .unwrap_or_default();
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(name)
                .placeholder(rust_i18n::t!("container.sidebar.folder_name").to_string())
        });
        input.update(cx, |state, cx| state.focus(window, cx));
        self.containers.engine_folder_form = Some(EngineFolderForm {
            parent,
            editing,
            input,
            error: None,
        });
        cx.notify();
    }

    pub(crate) fn dismiss_engine_folder_form(&mut self, cx: &mut Context<Self>) {
        self.containers.engine_folder_form = None;
        cx.notify();
    }

    /// 提交分组名称：新建或重命名，错误就地显示。
    pub(crate) fn submit_engine_folder_form(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(form) = self.containers.engine_folder_form.as_ref() else {
            return;
        };
        let name = form.input.read(cx).value().trim().to_string();
        let parent = form.parent;
        let editing = form.editing;
        let result = match editing {
            Some(id) => self.catalog.rename_engine_folder(id, &name),
            None => self.catalog.add_engine_folder(&name, parent).map(|_| ()),
        };
        match result {
            Ok(()) => {
                self.containers.engine_folder_form = None;
                self.persist_catalog();
            }
            Err(error) => {
                if let Some(form) = self.containers.engine_folder_form.as_mut() {
                    form.error = Some(error.to_string());
                }
            }
        }
        cx.notify();
    }

    /// 删除分组；其中的连接与子分组提升到父层级。
    pub(crate) fn delete_engine_folder(
        &mut self,
        folder_id: FolderId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.catalog.delete_engine_folder(folder_id) {
            Ok(name) => {
                self.persist_catalog();
                self.show_toast(
                    rust_i18n::t!("container.sidebar.folder_deleted", name = name).to_string(),
                    true,
                    window,
                    cx,
                );
            }
            Err(error) => self.set_error(error.to_string()),
        }
        cx.notify();
    }

    /// 把引擎连接移入某个分组（`None` 为根层级）。
    pub(crate) fn move_engine_to_folder(
        &mut self,
        profile_id: EngineProfileId,
        folder: Option<FolderId>,
        cx: &mut Context<Self>,
    ) {
        match self.catalog.move_engine(profile_id, folder) {
            Ok(()) => {
                self.persist_catalog();
            }
            Err(error) => self.set_error(error.to_string()),
        }
        cx.notify();
    }

    /// 处理引擎树拖放：换分组或同级排序。
    pub(crate) fn handle_container_sidebar_drop(
        &mut self,
        drag: ContainerSidebarDrag,
        target: ContainerSidebarTarget,
        cx: &mut Context<Self>,
    ) {
        match self.apply_container_sidebar_drop(drag, target) {
            Ok(landed) => {
                // 目标分组被折叠时自动展开，保证拖进去的条目可见。
                if let Some(folder_id) = landed
                    && let Some(folder) = self
                        .catalog
                        .engine_folders
                        .iter_mut()
                        .find(|folder| folder.id == folder_id)
                {
                    folder.collapsed = false;
                }
                self.persist_catalog();
            }
            Err(error) => self.set_error(error.to_string()),
        }
        cx.notify();
    }

    fn apply_container_sidebar_drop(
        &mut self,
        drag: ContainerSidebarDrag,
        target: ContainerSidebarTarget,
    ) -> Result<Option<FolderId>, CatalogError> {
        match (drag, target) {
            (ContainerSidebarDrag::Engine(profile_id), ContainerSidebarTarget::Root) => {
                self.catalog.move_engine(profile_id, None)?;
                Ok(None)
            }
            (ContainerSidebarDrag::Folder(folder_id), ContainerSidebarTarget::Root) => {
                self.catalog.move_engine_folder(folder_id, None)?;
                Ok(None)
            }
            (
                ContainerSidebarDrag::Engine(profile_id),
                ContainerSidebarTarget::Folder(folder_id),
            ) => {
                self.catalog.move_engine(profile_id, Some(folder_id))?;
                Ok(Some(folder_id))
            }
            (ContainerSidebarDrag::Folder(moved), ContainerSidebarTarget::Folder(parent)) => {
                if moved == parent {
                    return Ok(Some(parent));
                }
                self.catalog.move_engine_folder(moved, Some(parent))?;
                Ok(Some(parent))
            }
            (drag, ContainerSidebarTarget::Relative { anchor, side }) => match (drag, anchor) {
                (
                    ContainerSidebarDrag::Engine(profile_id),
                    ContainerSidebarDrag::Engine(anchor_id),
                ) => {
                    self.catalog.reorder_engine(profile_id, anchor_id, side)?;
                    Ok(self
                        .catalog
                        .engines
                        .iter()
                        .find(|profile| profile.id == anchor_id)
                        .and_then(|profile| profile.folder_id))
                }
                (
                    ContainerSidebarDrag::Folder(folder_id),
                    ContainerSidebarDrag::Folder(anchor_id),
                ) => {
                    self.catalog
                        .reorder_engine_folder(folder_id, anchor_id, side)?;
                    Ok(self
                        .catalog
                        .engine_folder(folder_id)
                        .and_then(|folder| folder.parent))
                }
                (
                    ContainerSidebarDrag::Engine(profile_id),
                    ContainerSidebarDrag::Folder(anchor_id),
                ) => {
                    self.catalog.move_engine(profile_id, Some(anchor_id))?;
                    Ok(Some(anchor_id))
                }
                (
                    ContainerSidebarDrag::Folder(folder_id),
                    ContainerSidebarDrag::Engine(anchor_id),
                ) => {
                    let parent = self
                        .catalog
                        .engines
                        .iter()
                        .find(|profile| profile.id == anchor_id)
                        .and_then(|profile| profile.folder_id);
                    self.catalog.move_engine_folder(folder_id, parent)?;
                    Ok(parent)
                }
            },
        }
    }

    /// 引擎分组的新建/重命名弹窗。
    pub(crate) fn render_engine_folder_form(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(form) = self.containers.engine_folder_form.as_ref() else {
            return div().into_any_element();
        };
        let editing = form.editing.is_some();
        let parent = form.parent;
        let input = form.input.clone();
        let error = form.error.clone();
        let title = if editing {
            rust_i18n::t!("container.sidebar.folder_edit_title").to_string()
        } else {
            rust_i18n::t!("container.sidebar.folder_new_title").to_string()
        };
        let description = parent
            .and_then(|id| self.catalog.engine_folder_path(id))
            .map(|path| rust_i18n::t!("container.sidebar.folder_parent", path = path).to_string());
        let mut content = v_flex().gap_3().child(Self::form_field(
            rust_i18n::t!("container.sidebar.folder_name").to_string(),
            input,
            p,
        ));
        if let Some(error) = error {
            content = content.child(div().text_xs().text_color(p.danger).child(error));
        }
        let footer = h_flex()
            .gap_2()
            .justify_end()
            .child(
                Button::new("container-engine-folder-cancel")
                    .ghost()
                    .label(rust_i18n::t!("common.cancel").to_string())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.dismiss_engine_folder_form(cx);
                    })),
            )
            .child(
                Button::new("container-engine-folder-save")
                    .primary()
                    .icon(IconName::Check)
                    .label(rust_i18n::t!("common.save").to_string())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.submit_engine_folder_form(window, cx);
                    })),
            );
        let frame = Self::dialog_frame(
            "container-engine-folder-modal",
            title,
            description,
            content,
            Some(footer),
            px(420.0),
            p,
        );
        let backdrop = div()
            .id("container-engine-folder-backdrop")
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .on_click(cx.listener(|this, _, _, cx| {
                this.dismiss_engine_folder_form(cx);
            }))
            .key_context(crate::keymap::OVERLAY_CONTEXT)
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                this.on_container_overlay_key(event, None, cx);
            }))
            .child(div().occlude().child(frame));
        Self::overlay(backdrop, p)
    }
}

/// 侧栏图标按钮：与会话树的图标按钮同尺寸、同悬停口径。
fn engine_sidebar_button(
    id: &'static str,
    icon: IconName,
    tooltip: String,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    Button::new(id)
        .ghost()
        .xsmall()
        .icon(icon)
        .tooltip(tooltip)
        .on_click(on_click)
        .into_any_element()
}

/// 引擎行/分组行的排序落点遮罩：会话行二分，分组行保留中段作为“放入该分组”。
fn engine_reorder_overlay(
    app: &Entity<AppShell>,
    anchor: ContainerSidebarDrag,
    keep_middle: bool,
    p: &Palette,
) -> AnyElement {
    let overlay = div()
        .absolute()
        .inset_0()
        .flex()
        .flex_col()
        .child(engine_reorder_half(app, anchor, DropSide::Before, true, p));
    let overlay = if keep_middle {
        overlay.child(div().flex_1())
    } else {
        overlay
    };
    overlay
        .child(engine_reorder_half(app, anchor, DropSide::After, false, p))
        .into_any_element()
}

fn engine_reorder_half(
    app: &Entity<AppShell>,
    anchor: ContainerSidebarDrag,
    side: DropSide,
    top: bool,
    p: &Palette,
) -> AnyElement {
    let target = ContainerSidebarTarget::Relative { anchor, side };
    let line_color = p.sidebar_accent_foreground;
    let zone = div().flex_1().border_color(line_color);
    let zone = if top {
        zone.drag_over::<ContainerSidebarDrag>(move |style, _, _, _| {
            style.border_t_2().border_color(line_color)
        })
    } else {
        zone.drag_over::<ContainerSidebarDrag>(move |style, _, _, _| {
            style.border_b_2().border_color(line_color)
        })
    };
    let app = app.clone();
    zone.on_drop(move |drag: &ContainerSidebarDrag, window, cx| {
        let drag = *drag;
        app.update(cx, |this, cx| {
            this.handle_container_sidebar_drop(drag, target, cx);
            window.refresh();
        });
    })
    .into_any_element()
}

/// 构建引擎分组树行：同级按目录数组顺序输出，搜索时保留命中条目的祖先。
fn build_engine_rows(catalog: &SessionCatalog, query: &str) -> Vec<ContainerSidebarRow> {
    let query = query.trim().to_ascii_lowercase();
    let mut child_folders: HashMap<Option<FolderId>, Vec<&EngineFolder>> = HashMap::new();
    for folder in &catalog.engine_folders {
        child_folders.entry(folder.parent).or_default().push(folder);
    }
    let mut child_engines: HashMap<Option<FolderId>, Vec<&ContainerEngineProfile>> = HashMap::new();
    for profile in &catalog.engines {
        child_engines
            .entry(profile.folder_id)
            .or_default()
            .push(profile);
    }

    let (engine_counts, folder_counts) = engine_subtree_counts(catalog);
    let folder_paths: HashMap<FolderId, String> = catalog
        .engine_folders
        .iter()
        .filter_map(|folder| {
            catalog
                .engine_folder_path(folder.id)
                .map(|path| (folder.id, path))
        })
        .collect();
    let matched_engines: HashSet<EngineProfileId> = catalog
        .engines
        .iter()
        .filter(|profile| {
            if query.is_empty() {
                return true;
            }
            let path = profile
                .folder_id
                .and_then(|id| folder_paths.get(&id))
                .map(String::as_str)
                .unwrap_or_default();
            let haystack = format!(
                "{} {} {}",
                profile.display_name().to_ascii_lowercase(),
                profile.engine.describe().to_ascii_lowercase(),
                path.to_ascii_lowercase()
            );
            haystack.contains(&query)
        })
        .map(|profile| profile.id)
        .collect();

    let mut visible_folders = HashSet::new();
    for folder in &catalog.engine_folders {
        let name_matches = !query.is_empty() && folder.name.to_ascii_lowercase().contains(&query);
        let has_engine = child_engines.get(&Some(folder.id)).is_some_and(|engines| {
            engines
                .iter()
                .any(|profile| matched_engines.contains(&profile.id))
        });
        if name_matches || has_engine {
            let mut current = Some(folder.id);
            while let Some(id) = current {
                if !visible_folders.insert(id) {
                    break;
                }
                current = catalog.engine_folder(id).and_then(|folder| folder.parent);
            }
        }
    }

    enum Pending {
        Folder(FolderId, usize, bool),
        EngineList(Option<FolderId>, usize, bool),
    }
    let mut rows = Vec::with_capacity(catalog.engines.len() + catalog.engine_folders.len());
    let mut stack = vec![Pending::EngineList(None, 0, false)];
    if let Some(roots) = child_folders.get(&None) {
        for folder in roots.iter().rev() {
            stack.push(Pending::Folder(folder.id, 0, false));
        }
    }
    while let Some(item) = stack.pop() {
        match item {
            Pending::EngineList(parent, depth, show_all) => {
                let Some(engines) = child_engines.get(&parent) else {
                    continue;
                };
                for profile in engines {
                    if query.is_empty() || show_all || matched_engines.contains(&profile.id) {
                        rows.push(ContainerSidebarRow::Engine(EngineRow {
                            profile: Box::new((*profile).clone()),
                            depth,
                        }));
                    }
                }
            }
            Pending::Folder(id, depth, ancestor_matched) => {
                let Some(folder) = catalog.engine_folder(id) else {
                    continue;
                };
                let name_matched =
                    !query.is_empty() && folder.name.to_ascii_lowercase().contains(&query);
                let show_all = ancestor_matched || name_matched;
                if !query.is_empty() && !show_all && !visible_folders.contains(&id) {
                    continue;
                }
                rows.push(ContainerSidebarRow::Folder(EngineFolderRow {
                    id,
                    name: folder.name.clone(),
                    depth,
                    collapsed: folder.collapsed && query.is_empty(),
                    engine_count: engine_counts.get(&id).copied().unwrap_or(0),
                    folder_count: folder_counts.get(&id).copied().unwrap_or(0),
                }));
                if query.is_empty() && folder.collapsed {
                    continue;
                }
                stack.push(Pending::EngineList(Some(id), depth + 1, show_all));
                if let Some(children) = child_folders.get(&Some(id)) {
                    for child in children.iter().rev() {
                        stack.push(Pending::Folder(child.id, depth + 1, show_all));
                    }
                }
            }
        }
    }
    rows
}

/// 统计每个引擎分组子树中的连接与子分组总数（沿父链累加，避免递归）。
fn engine_subtree_counts(
    catalog: &SessionCatalog,
) -> (HashMap<FolderId, usize>, HashMap<FolderId, usize>) {
    let mut direct_engines: HashMap<Option<FolderId>, usize> = HashMap::new();
    for profile in &catalog.engines {
        *direct_engines.entry(profile.folder_id).or_default() += 1;
    }
    let mut engine_counts: HashMap<FolderId, usize> = HashMap::new();
    let mut folder_counts: HashMap<FolderId, usize> = HashMap::new();
    for folder in &catalog.engine_folders {
        let direct = direct_engines.get(&Some(folder.id)).copied().unwrap_or(0);
        *engine_counts.entry(folder.id).or_default() += direct;
        let mut current = folder.parent;
        while let Some(parent) = current {
            *engine_counts.entry(parent).or_default() += direct;
            *folder_counts.entry(parent).or_default() += 1;
            current = catalog
                .engine_folder(parent)
                .and_then(|folder| folder.parent);
        }
    }
    (engine_counts, folder_counts)
}
