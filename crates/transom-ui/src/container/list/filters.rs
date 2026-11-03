//! 列表工具栏的筛选与排序菜单。

use super::*;

impl AppShell {
    /// 状态下拉：只有容器分类有运行态可筛，其它分类渲染空元素占位。
    pub(crate) fn render_state_filter_menu(
        &mut self,
        params: &ToolbarParams<'_>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if params.section != ContainerSection::Containers {
            return div().into_any_element();
        }
        let tab_id = params.tab_id;
        let current = params.state_filter;
        let app = cx.entity().clone();
        dropdown_trigger(
            "container-filter-state",
            rust_i18n::t!(current.label_key()).to_string(),
            IconName::ChevronDown,
            None,
            move |menu, _, _| {
                let mut menu = menu;
                for filter in ContainerStateFilter::ALL {
                    let app = app.clone();
                    menu = menu.item(
                        PopupMenuItem::new(rust_i18n::t!(filter.label_key()).to_string())
                            .checked(filter == current)
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
                menu
            },
        )
    }

    /// 快速筛选下拉：镜像「仅悬空」、数据卷「仅未使用」。
    pub(crate) fn render_quick_filter_menu(
        &mut self,
        params: &ToolbarParams<'_>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let options = ContainerQuickFilter::for_section(params.section);
        if options.len() <= 1 {
            return div().into_any_element();
        }
        let tab_id = params.tab_id;
        let current = params.quick;
        // CLI 兜底后端拿不到挂载明细，「仅未使用卷」会把在用卷也说成空闲，
        // 因此该档位在 CLI 后端下禁用而不是悄悄收掉，否则用户会以为功能不存在。
        let unused_unavailable = params.cli_backend
            && current == ContainerQuickFilter::UnusedVolumes
            && options.contains(&ContainerQuickFilter::UnusedVolumes);
        let app = cx.entity().clone();
        dropdown_trigger(
            "container-filter-quick",
            rust_i18n::t!(current.label_key()).to_string(),
            IconName::ChevronDown,
            unused_unavailable
                .then(|| rust_i18n::t!("container.filter.unused_unavailable").to_string()),
            move |menu, _, _| {
                let mut menu = menu;
                for quick in options.iter().copied() {
                    let app = app.clone();
                    let disabled =
                        quick == ContainerQuickFilter::UnusedVolumes && unused_unavailable;
                    menu = menu.item(
                        PopupMenuItem::new(rust_i18n::t!(quick.label_key()).to_string())
                            .checked(quick == current)
                            .disabled(disabled)
                            .on_click(move |_, _, cx| {
                                app.update(cx, |this, cx| {
                                    if let Some(workspace) =
                                        this.state.container_workspace_mut(tab_id)
                                    {
                                        workspace.quick_filter = quick;
                                    }
                                    cx.notify();
                                });
                            }),
                    );
                }
                menu
            },
        )
    }

    /// 排序下拉：排序键与升降序收在同一个菜单里，触发按钮直接显示当前方向。
    pub(crate) fn render_sort_menu(
        &mut self,
        params: &ToolbarParams<'_>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tab_id = params.tab_id;
        let section = params.section;
        let current = active_sort(section, params.sort_key);
        let desc = params.sort_desc;
        let app = cx.entity().clone();
        dropdown_trigger(
            "container-sort-menu",
            rust_i18n::t!(current.label_key()).to_string(),
            if desc {
                IconName::ArrowDown
            } else {
                IconName::ArrowUp
            },
            None,
            move |menu, _, _| {
                let mut menu = menu;
                for key in ContainerSortKey::for_section(section) {
                    let app = app.clone();
                    menu = menu.item(
                        PopupMenuItem::new(rust_i18n::t!(key.label_key()).to_string())
                            .checked(*key == current)
                            .on_click(move |_, _, cx| {
                                app.update(cx, |this, cx| {
                                    let Some(workspace) =
                                        this.state.container_workspace_mut(tab_id)
                                    else {
                                        return;
                                    };
                                    // 同键再点一次只翻方向，换键则回到升序。
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
                        .checked(ascending != desc)
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
                menu
            },
        )
    }
}

/// 工具条「新增」按钮的文案；容器与编排分类没有这一档。
pub(crate) fn create_button_label(section: ContainerSection) -> Option<String> {
    let key = match section {
        ContainerSection::Containers | ContainerSection::Compose => {
            return None;
        }
        ContainerSection::Images => "container.action.build",
        ContainerSection::Volumes => "container.create_dialog.volume",
        ContainerSection::Networks => "container.create_dialog.network",
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
/// 下拉触发按钮：统一的「当前值 + 箭头」外观，菜单内容由 builder 填充。
///
/// 设计原理（Why）：中栏宽度可拖到 240px，控制项必须尽量窄；三个下拉共用同一个
/// 触发样式，避免出现三种不同的收起形态。`tooltip` 只用于说明不可用的档位。
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
