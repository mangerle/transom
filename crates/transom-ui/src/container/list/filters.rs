//! 列表工具栏的统合筛选、排序与更多操作菜单。
//!
//! 设计原理（Why）：此前中栏工具条将状态筛选、快速清理筛选、排序、新建、构建、
//! 探索、恢复、Prune 等 8 个控件同时平铺，在 380px 的窄列表下造成极其拥挤和溢出。
//! 这里按照“简洁到不简单”原则收敛为两个统一菜单：
//! 1. 统一筛选菜单：将状态、快速筛选、排序组合收拢，激活时带主色小圆点；
//! 2. 更多操作菜单：将低频管理动作（Prune、导入导出、构建、探索等）归入更多。

use gpui_kit::component::menu::PopupMenu;
use gpui_kit::prelude::FluentBuilder;

use super::*;

impl AppShell {
    /// 统合筛选菜单：状态筛选、快速清理筛选与排序方式收于一处，激活时展示主色指示圆点。
    pub(crate) fn render_unified_filter_menu(
        &mut self,
        params: &ToolbarParams<'_>,
        _p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tab_id = params.tab_id;
        let section = params.section;
        let current_state = params.state_filter;
        let current_include_compose = params.include_compose;
        let current_quick = params.quick;
        let current_sort = active_sort(section, params.sort_key);
        let sort_desc = params.sort_desc;
        let app = cx.entity().clone();

        let is_filtered = (section == ContainerSection::Containers
            && (current_state != ContainerStateFilter::All || current_include_compose))
            || current_quick != ContainerQuickFilter::All
            || current_sort != ContainerSortKey::default_for(section)
            || sort_desc;

        Button::new("container-filter-unified")
            .xsmall()
            .ghost()
            .icon(IconName::ListFilter)
            .label(rust_i18n::t!("container.filter.title").to_string())
            .when(is_filtered, |this| this.selected(true))
            .dropdown_menu(move |menu, _, _| {
                let mut menu = menu;

                // 1. 容器运行状态筛选与 Compose 容器包含项
                if section == ContainerSection::Containers {
                    for filter in ContainerStateFilter::ALL {
                        let app = app.clone();
                        menu = menu.item(
                            PopupMenuItem::new(rust_i18n::t!(filter.label_key()).to_string())
                                .checked(filter == current_state)
                                .on_click(move |_, _, cx| {
                                    app.update(cx, |this, cx| {
                                        if let Some(workspace) =
                                            this.state.container_workspace_mut(tab_id)
                                        {
                                            workspace.state_filter = filter;
                                        }
                                        cx.notify();
                                    });
                                }),
                        );
                    }
                    menu = menu.item(PopupMenuItem::separator());
                    let app = app.clone();
                    menu = menu.item(
                        PopupMenuItem::new(
                            rust_i18n::t!("container.filter.include_compose").to_string(),
                        )
                        .checked(current_include_compose)
                        .on_click(move |_, _, cx| {
                            app.update(cx, |this, cx| {
                                if let Some(workspace) = this.state.container_workspace_mut(tab_id)
                                {
                                    workspace.include_compose = !workspace.include_compose;
                                }
                                cx.notify();
                            });
                        }),
                    );
                    menu = menu.item(PopupMenuItem::separator());
                }

                // 2. 快速清理视角（镜像悬空、未用卷等）
                let quick_options = ContainerQuickFilter::for_section(section);
                if quick_options.len() > 1 {
                    for quick in quick_options {
                        let app = app.clone();
                        menu = menu.item(
                            PopupMenuItem::new(rust_i18n::t!(quick.label_key()).to_string())
                                .checked(*quick == current_quick)
                                .on_click(move |_, _, cx| {
                                    app.update(cx, |this, cx| {
                                        if let Some(workspace) =
                                            this.state.container_workspace_mut(tab_id)
                                        {
                                            workspace.quick_filter = *quick;
                                        }
                                        cx.notify();
                                    });
                                }),
                        );
                    }
                    menu = menu.item(PopupMenuItem::separator());
                }

                // 3. 排序键
                for key in ContainerSortKey::for_section(section) {
                    let app = app.clone();
                    menu = menu.item(
                        PopupMenuItem::new(rust_i18n::t!(key.label_key()).to_string())
                            .checked(*key == current_sort)
                            .on_click(move |_, _, cx| {
                                app.update(cx, |this, cx| {
                                    let Some(workspace) =
                                        this.state.container_workspace_mut(tab_id)
                                    else {
                                        return;
                                    };
                                    if active_sort(workspace.section, workspace.sort_key) == *key {
                                        workspace.sort_desc = !workspace.sort_desc;
                                    } else {
                                        workspace.sort_key = *key;
                                        workspace.sort_desc = false;
                                    }
                                    cx.notify();
                                });
                            }),
                    );
                }

                menu = menu.item(PopupMenuItem::separator());
                for ascending in [true, false] {
                    let app = app.clone();
                    menu = menu.item(
                        PopupMenuItem::new(
                            rust_i18n::t!(if ascending {
                                "container.sort.ascending"
                            } else {
                                "container.sort.descending"
                            })
                            .to_string(),
                        )
                        .checked(ascending != sort_desc)
                        .on_click(move |_, _, cx| {
                            app.update(cx, |this, cx| {
                                if let Some(workspace) = this.state.container_workspace_mut(tab_id)
                                {
                                    workspace.sort_desc = !ascending;
                                }
                                cx.notify();
                            });
                        }),
                    );
                }

                // 4. 重置筛选
                if is_filtered {
                    menu = menu.item(PopupMenuItem::separator());
                    let app = app.clone();
                    menu = menu.item(
                        PopupMenuItem::new(rust_i18n::t!("container.filter.reset").to_string())
                            .on_click(move |_, _, cx| {
                                app.update(cx, |this, cx| {
                                    if let Some(workspace) =
                                        this.state.container_workspace_mut(tab_id)
                                    {
                                        workspace.state_filter = ContainerStateFilter::All;
                                        workspace.include_compose = false;
                                        workspace.quick_filter = ContainerQuickFilter::All;
                                        workspace.sort_key =
                                            ContainerSortKey::default_for(workspace.section);
                                        workspace.sort_desc = false;
                                    }
                                    cx.notify();
                                });
                            }),
                    );
                }

                menu
            })
            .into_any_element()
    }

    /// 更多操作菜单：收纳导入、导出、构建、探索、恢复及深度清理等低频入口。
    pub(crate) fn render_more_menu(
        &mut self,
        params: &ToolbarParams<'_>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tab_id = params.tab_id;
        let section = params.section;
        let app = cx.entity().clone();

        Button::new("container-more-menu")
            .xsmall()
            .ghost()
            .icon(IconName::Ellipsis)
            .tooltip(rust_i18n::t!("common.more").to_string())
            .dropdown_menu(move |menu, _, _| {
                let mut menu = menu;
                match section {
                    ContainerSection::Containers => {
                        let app = app.clone();
                        menu = menu.item(
                            PopupMenuItem::new(
                                rust_i18n::t!("container.system_df.title").to_string(),
                            )
                            .on_click(move |_, _, cx| {
                                app.update(cx, |this, cx| {
                                    this.open_container_system_df_dialog(tab_id, cx);
                                });
                            }),
                        );
                    }
                    ContainerSection::Images => {
                        let app_pull = app.clone();
                        menu = menu.item(
                            PopupMenuItem::new(rust_i18n::t!("container.action.pull").to_string())
                                .on_click(move |_, window, cx| {
                                    app_pull.update(cx, |this, cx| {
                                        this.open_container_create_dialog(
                                            tab_id,
                                            ContainerSection::Images,
                                            window,
                                            cx,
                                        );
                                    });
                                }),
                        );
                        let app_load = app.clone();
                        menu = menu.item(
                            PopupMenuItem::new(
                                rust_i18n::t!("container.action.load_image").to_string(),
                            )
                            .on_click(move |_, _, cx| {
                                app_load.update(cx, |this, cx| {
                                    this.load_image(tab_id, cx);
                                });
                            }),
                        );
                        let app_build = app.clone();
                        menu = menu.item(
                            PopupMenuItem::new(rust_i18n::t!("container.build.title").to_string())
                                .on_click(move |_, window, cx| {
                                    app_build.update(cx, |this, cx| {
                                        this.open_container_build_dialog(tab_id, window, cx);
                                    });
                                }),
                        );
                        menu = menu.item(PopupMenuItem::separator());
                        let app_prune = app.clone();
                        menu = menu.item(
                            PopupMenuItem::new(
                                rust_i18n::t!(ContainerPruneTarget::Images.button_label_key())
                                    .to_string(),
                            )
                            .on_click(move |_, _, cx| {
                                app_prune.update(cx, |this, cx| {
                                    this.prune_container_target(
                                        tab_id,
                                        ContainerPruneTarget::Images,
                                        true,
                                        cx,
                                    );
                                });
                            }),
                        );
                    }
                    ContainerSection::Volumes => {
                        let app_restore = app.clone();
                        menu = menu.item(
                            PopupMenuItem::new(
                                rust_i18n::t!("container.action.restore_volume").to_string(),
                            )
                            .on_click(move |_, _, cx| {
                                app_restore.update(cx, |this, cx| {
                                    this.restore_volume(tab_id, None, cx);
                                });
                            }),
                        );
                        menu = menu.item(PopupMenuItem::separator());
                        let app_prune = app.clone();
                        menu = menu.item(
                            PopupMenuItem::new(
                                rust_i18n::t!(ContainerPruneTarget::Volumes.button_label_key())
                                    .to_string(),
                            )
                            .on_click(move |_, _, cx| {
                                app_prune.update(cx, |this, cx| {
                                    this.prune_container_target(
                                        tab_id,
                                        ContainerPruneTarget::Volumes,
                                        true,
                                        cx,
                                    );
                                });
                            }),
                        );
                    }
                    ContainerSection::Networks => {
                        let app_prune = app.clone();
                        menu = menu.item(
                            PopupMenuItem::new(
                                rust_i18n::t!(ContainerPruneTarget::Networks.button_label_key())
                                    .to_string(),
                            )
                            .on_click(move |_, _, cx| {
                                app_prune.update(cx, |this, cx| {
                                    this.prune_container_target(
                                        tab_id,
                                        ContainerPruneTarget::Networks,
                                        true,
                                        cx,
                                    );
                                });
                            }),
                        );
                    }
                    ContainerSection::Compose => {
                        let app_import = app.clone();
                        menu = menu.item(
                            PopupMenuItem::new(
                                rust_i18n::t!("container.compose.import_yaml").to_string(),
                            )
                            .on_click(move |_, _, cx| {
                                app_import.update(cx, |this, cx| {
                                    this.import_local_compose(tab_id, cx);
                                });
                            }),
                        );
                    }
                }
                menu
            })
            .into_any_element()
    }
}

/// 工具条「新增」按钮的文案。
pub(crate) fn create_button_label(section: ContainerSection) -> Option<String> {
    let key = match section {
        ContainerSection::Containers => "container.action.run",
        ContainerSection::Images => "container.action.pull",
        ContainerSection::Volumes => "container.create_dialog.volume",
        ContainerSection::Networks => "container.create_dialog.network",
        ContainerSection::Compose => "container.compose.new_yaml",
    };
    Some(rust_i18n::t!(key).to_string())
}

/// 当前生效的排序键：分类切换后旧键可能不在新分类的可用档位里。
pub(crate) fn active_sort(
    section: ContainerSection,
    sort_key: ContainerSortKey,
) -> ContainerSortKey {
    if ContainerSortKey::for_section(section).contains(&sort_key) {
        sort_key
    } else {
        ContainerSortKey::default_for(section)
    }
}

/// 兼容弹窗中复用的下拉菜单触发器按钮构建辅助函数。
pub(crate) fn dropdown_trigger(
    id: &'static str,
    label: String,
    icon: IconName,
    tooltip: Option<String>,
    builder: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
) -> AnyElement {
    let button = Button::new(ElementId::Name(id.into()))
        .xsmall()
        .ghost()
        .label(label);
    let button = match tooltip {
        Some(tooltip) => button.tooltip(tooltip),
        None => button,
    };
    button.icon(icon).dropdown_menu(builder).into_any_element()
}
