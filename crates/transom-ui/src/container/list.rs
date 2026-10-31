//! 工作区中栏：搜索、状态筛选、排序与带操作的列表行。
//!
//! 设计原理（Why）：容器管理工具的主干交互是「快速定位一个条目，然后立刻对它
//! 做一件事」（Portainer、Docker Desktop、lazydocker 都是这个形态）。原先的中栏
//! 只能点击选中、再去右侧头部找按钮，长列表里来回移动鼠标成本很高，也没有排序
//! 与状态筛选。本模块把五种条目统一成 [`ListRow`] 描述，渲染与动作分发因此只有
//! 一处实现，新增分类只要补一个构造函数。
//!
//! 代价与局限（Trade-offs）：行内只放启停与删除两个按钮，其余动作收进右键菜单，
//! 窄列表下仍要展开菜单；筛选与排序都在前端完成，不额外发请求。

use crate::app::AppShell;
use crate::container::types::{
    ContainerBatchAction, ContainerPruneTarget, ContainerQuickFilter, ContainerSection,
    ContainerSortKey, ContainerStateFilter, ContainerWorkspaceState, RowAction, RowTarget,
    SortFields, compare_fields, image_passes_filter, matches_query, row_key, volume_passes_filter,
};
use crate::format::format_size;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::{ContextMenuExt, DropdownMenu, PopupMenu, PopupMenuItem};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use std::collections::HashSet;
use transom_core::{
    ComposeProject, ContainerImageSummary, ContainerNetworkSummary, ContainerSummary,
    ContainerVolumeSummary, TabId, short_id,
};

/// 行内快捷按钮数量上限：再多会把标题挤到不可读。
const QUICK_LIMIT: usize = 3;

/// 工具条渲染参数：分类、筛选与排序档位较多，收敛为结构体避免长参数列表。
struct ToolbarParams<'a> {
    tab_id: TabId,
    section: ContainerSection,
    search: &'a Entity<InputState>,
    state_filter: ContainerStateFilter,
    quick: ContainerQuickFilter,
    sort_key: ContainerSortKey,
    sort_desc: bool,
    /// CLI 兜底后端拿不到挂载明细，「仅未使用卷」档位不可用。
    cli_backend: bool,
    /// 已按下一次清理、等待第二次确认的目标；`None` 表示没有待确认动作。
    prune_armed: Option<ContainerPruneTarget>,
}

/// 一行的展示数据与动作目标。
struct ListRow {
    /// 与条目主键拼成稳定 ElementId 的前缀。
    key: String,
    title: String,
    /// 副标题：容器只有名称，静态资源才带大小/驱动等次要信息。
    subtitle: Option<String>,
    badge: Option<String>,
    /// 运行态：容器与编排项目有，静态资源没有。
    running: Option<bool>,
    /// 多选勾选状态：只有容器分类需要。
    checked: Option<bool>,
    target: RowTarget,
    fields: SortFields,
}

impl AppShell {
    /// 中栏：工具条（搜索 + 筛选 + 排序）、列表、新建行与计数页脚。
    pub(crate) fn render_container_list(
        &mut self,
        tab_id: TabId,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(workspace) = self.container_workspace(tab_id) else {
            return div().into_any_element();
        };
        let section = workspace.section;
        let search = workspace.search.clone();
        let state_filter = workspace.state_filter;
        let sort_key = workspace.sort_key;
        let sort_desc = workspace.sort_desc;
        let busy = workspace.busy;
        let notice = workspace.notice.clone();
        let selected = workspace.selected.clone();
        let checked = workspace.checked.clone();
        let batch_armed = workspace.batch_armed;
        let quick = workspace.quick_filter;
        let cli_backend = workspace.info.as_ref().is_some_and(|info| info.cli_backend);
        let query = workspace.query(cx).to_lowercase();
        let mut rows = collect_rows(section, workspace, &query, state_filter, &checked);
        rows.sort_by(|left, right| {
            compare_fields(
                &left.fields,
                &right.fields,
                active_sort(section, sort_key),
                sort_desc,
            )
        });

        let checked_count = rows
            .iter()
            .filter(|row| row.checked.unwrap_or(false))
            .count();
        // 信息条常驻：优先显示批量选择，其次显示操作状态；两者都没有时只占位。
        // 容器分类始终占位（勾选/取消不跳行）；其它分类仅在有状态时才出现，
        // 不白占一行高度。
        let batch_bar = (section == ContainerSection::Containers || notice.is_some()).then(|| {
            render_status_bar(
                tab_id,
                section,
                checked_count,
                batch_armed,
                notice.as_deref(),
                p,
                cx,
            )
        });
        let toolbar = self.render_list_toolbar(
            &ToolbarParams {
                tab_id,
                section,
                search: &search,
                state_filter,
                quick,
                sort_key,
                sort_desc,
                cli_backend,
                prune_armed: workspace.prune_armed,
            },
            cx,
        );
        let body = render_rows(tab_id, &rows, &selected, p, cx);
        let mut column = v_flex()
            .h_full()
            .w_full()
            .min_w_0()
            .overflow_hidden()
            .gap_1p5()
            .p_2()
            .child(toolbar)
            .children(batch_bar)
            .child(body);
        if busy {
            column = column.child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(Icon::new(IconName::Loader).xsmall())
                    .child(rust_i18n::t!("container.loading").to_string()),
            );
        }
        column.into_any_element()
    }

    /// 工具条：一行摆下搜索框与状态、快速筛选、排序三个下拉。
    ///
    /// 设计原理（Why）：原先把三档状态与三个排序键平铺成芯片，在可拖到 260px 的
    /// 中栏里横向撑爆，行内容被推到右侧详情面板上（真实窗口实测）。收成下拉后
    /// 控制项与搜索框同处一行，中栏不再需要第二行工具条；代价是换档位多一次点击。
    /// 「运行容器」不再占工具条：起容器从镜像行的「运行」进入，同一件事不留两个入口。
    fn render_list_toolbar(
        &mut self,
        params: &ToolbarParams<'_>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut controls = h_flex()
            .gap_1()
            .items_center()
            .flex_wrap()
            .child(Input::new(params.search).flex_1().min_w_0())
            .child(self.render_state_filter_menu(params, cx))
            .child(self.render_quick_filter_menu(params, cx))
            .child(self.render_sort_menu(params, cx));
        // 新增入口收在工具条上：底部常驻一行输入框在三个分类里都占高度却几乎不用。
        // 卷与网络只留加号图标：中栏本来就窄，文字按钮会把搜索框压到不可用。
        if let Some(label) = create_button_label(params.section) {
            let tab_id = params.tab_id;
            let section = params.section;
            let button = Button::new("container-create-open")
                .xsmall()
                .ghost()
                .icon(IconName::Plus);
            let button = if matches!(
                section,
                ContainerSection::Volumes | ContainerSection::Networks
            ) {
                button.tooltip(label)
            } else {
                button.label(label)
            };
            controls = controls.child(button.on_click(cx.listener(move |this, _, window, cx| {
                this.open_container_create_dialog(tab_id, section, window, cx);
            })));
        }
        // 卷与镜像都有「清理」视角：卷清未使用，镜像清悬空且无标签的那批。
        let prune_target = match params.section {
            ContainerSection::Volumes => Some(ContainerPruneTarget::Volumes),
            ContainerSection::Images => Some(ContainerPruneTarget::Images),
            _ => None,
        };
        if let Some(target) = prune_target {
            let tab_id = params.tab_id;
            let armed = params.prune_armed == Some(target);
            controls = controls.child(
                Button::new(ElementId::Name(
                    format!("container-prune-{:?}", target).into(),
                ))
                .xsmall()
                .ghost()
                .icon(if armed {
                    IconName::CircleAlert
                } else {
                    IconName::Trash
                })
                .selected(armed)
                .tooltip(if armed {
                    rust_i18n::t!("container.prune.confirm").to_string()
                } else {
                    format!(
                        "{}：{}",
                        rust_i18n::t!(target.button_label_key()),
                        rust_i18n::t!(target.hint_key())
                    )
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.prune_container_target(tab_id, target, armed, cx);
                })),
            );
        }
        controls.into_any_element()
    }

    /// 状态下拉：只有容器分类有运行态可筛，其它分类渲染空元素占位。
    fn render_state_filter_menu(
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
                                    if let Some(workspace) = this.container_workspace_mut(tab_id) {
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
    fn render_quick_filter_menu(
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
                                    if let Some(workspace) = this.container_workspace_mut(tab_id) {
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
    fn render_sort_menu(
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
                                    let Some(workspace) = this.container_workspace_mut(tab_id)
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
                                if let Some(workspace) = this.container_workspace_mut(tab_id) {
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
        // 镜像拉取入口暂时收掉：先把列表、运行与详情打磨好，避免工具条继续变宽。
        ContainerSection::Images | ContainerSection::Containers | ContainerSection::Compose => {
            return None;
        }
        ContainerSection::Volumes => "container.create_dialog.volume",
        ContainerSection::Networks => "container.create_dialog.network",
    };
    Some(rust_i18n::t!(key).to_string())
}

/// 当前生效的排序键：分类切换后旧键可能不在新分类的可用档位里。
fn active_sort(section: ContainerSection, sort_key: ContainerSortKey) -> ContainerSortKey {
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
fn dropdown_trigger(
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

/// 按分类收集行数据，并套用状态筛选与关键词搜索。
fn collect_rows(
    section: ContainerSection,
    workspace: &ContainerWorkspaceState,
    query: &str,
    filter: ContainerStateFilter,
    checked: &HashSet<String>,
) -> Vec<ListRow> {
    let quick = workspace.quick_filter;
    let cli_backend = workspace.info.as_ref().is_some_and(|info| info.cli_backend);
    // CLI 兜底的容器摘要没有挂载明细，算不出在用卷，此时既不显示徽章也不给筛选。
    let usage_known = !cli_backend && !workspace.containers.is_empty();
    // 在用卷集合由已拉到的容器挂载计算：列表接口就带 Mounts，不需要逐个 inspect。
    let used_volumes: HashSet<String> = if usage_known {
        workspace
            .containers
            .iter()
            .flat_map(|item| item.mounts.iter())
            .filter(|mount| mount.kind == "volume")
            .map(|mount| mount.source.clone())
            .collect()
    } else {
        HashSet::default()
    };
    match section {
        ContainerSection::Containers => workspace
            .containers
            .iter()
            .filter(|item| filter.accepts(item.state.is_running()))
            .filter(|item| {
                matches_query(
                    query,
                    &[
                        item.name.clone(),
                        item.image.clone(),
                        item.state.key().to_string(),
                        item.compose_project.clone().unwrap_or_default(),
                    ],
                )
            })
            .map(|item| container_row_spec(item, checked.contains(&item.id)))
            .collect(),
        ContainerSection::Images => workspace
            .images
            .iter()
            .filter(|item| matches_query(query, &[item.display_name(), short_id(&item.id)]))
            .filter(|item| image_passes_filter(quick, &item.tags))
            .map(image_row_spec)
            .collect(),
        ContainerSection::Volumes => workspace
            .volumes
            .iter()
            .filter(|item| matches_query(query, &[item.name.clone(), item.driver.clone()]))
            .filter(|item| volume_passes_filter(quick, used_volumes.contains(&item.name)))
            .map(|item| volume_row_spec(item, usage_known, used_volumes.contains(&item.name)))
            .collect(),
        ContainerSection::Networks => workspace
            .networks
            .iter()
            .filter(|item| {
                matches_query(
                    query,
                    &[
                        item.name.clone(),
                        item.driver.clone(),
                        item.subnet.clone().unwrap_or_default(),
                    ],
                )
            })
            .map(network_row_spec)
            .collect(),
        ContainerSection::Compose => workspace
            .compose
            .iter()
            .filter(|item| {
                matches_query(
                    query,
                    &[
                        item.name.clone(),
                        item.working_dir.clone().unwrap_or_default(),
                    ],
                )
            })
            .map(compose_row_spec)
            .collect(),
    }
}

fn container_row_spec(item: &ContainerSummary, checked: bool) -> ListRow {
    ListRow {
        key: format!("container-row-{}", item.id),
        title: item.name.clone(),
        // 单行只剩名称：镜像引用（走加速仓库时三四十字符）与状态串在这一列宽度
        // 下必然截断成不可读片段，完整信息在右侧详情基础信息里看。
        subtitle: None,
        badge: None,
        running: Some(item.state.is_running()),
        checked: Some(checked),
        target: RowTarget::Container {
            id: item.id.clone(),
            name: item.name.clone(),
        },
        fields: SortFields {
            name: item.name.clone(),
            state: item.state.key().to_string(),
            created: item.created,
            size: 0,
        },
    }
}

fn image_row_spec(item: &ContainerImageSummary) -> ListRow {
    ListRow {
        key: format!("image-row-{}", item.id),
        // 短名做主标题：完整引用（含加速仓库主机）留在详情与起容器参数里。
        title: item.short_display_name(),
        subtitle: Some(format!(
            "{} · {}",
            format_size(item.size),
            short_id(&item.id)
        )),
        badge: None,
        running: None,
        checked: None,
        target: RowTarget::Image(item.id.clone()),
        fields: SortFields {
            name: item.display_name(),
            state: String::new(),
            created: item.created,
            size: item.size,
        },
    }
}

fn volume_row_spec(item: &ContainerVolumeSummary, usage_known: bool, in_use: bool) -> ListRow {
    ListRow {
        key: format!("volume-row-{}", item.name),
        title: item.name.clone(),
        subtitle: Some(format!("{} · {}", item.driver, item.mountpoint)),
        // 在用/空闲改用名称前的状态点，与容器行同一套语言；文字徽章去掉，
        // 避免同一件事在一行里说两遍。用量数据拿不到时不画点（不假装有把握）。
        badge: None,
        running: usage_known.then_some(in_use),
        checked: None,
        target: RowTarget::Volume(item.name.clone()),
        fields: SortFields {
            name: item.name.clone(),
            state: String::new(),
            created: parse_created(item.created.as_deref()),
            size: 0,
        },
    }
}

fn network_row_spec(item: &ContainerNetworkSummary) -> ListRow {
    ListRow {
        key: format!("network-row-{}", item.id),
        title: item.name.clone(),
        subtitle: Some(format!(
            "{} · {}{}",
            item.driver,
            item.scope,
            item.subnet
                .as_ref()
                .map(|subnet| format!(" · {subnet}"))
                .unwrap_or_default()
        )),
        badge: None,
        running: None,
        checked: None,
        target: RowTarget::Network(item.id.clone()),
        fields: SortFields {
            name: item.name.clone(),
            state: String::new(),
            created: 0,
            size: 0,
        },
    }
}

fn compose_row_spec(item: &ComposeProject) -> ListRow {
    ListRow {
        key: format!("compose-row-{}", item.name),
        title: item.name.clone(),
        subtitle: Some(format!(
            "{} · {}",
            item.state_label(),
            item.working_dir.clone().unwrap_or_default()
        )),
        badge: None,
        running: Some(item.total > 0 && item.running == item.total),
        checked: None,
        target: RowTarget::Compose(item.name.clone()),
        fields: SortFields {
            name: item.name.clone(),
            state: item.state_label().to_string(),
            created: 0,
            size: item.total as u64,
        },
    }
}

/// 卷的创建时间是 RFC3339 字符串；解析失败按 0 处理，只影响排序不影响展示。
fn parse_created(value: Option<&str>) -> i64 {
    value
        .and_then(|raw| chrono::DateTime::parse_from_rfc3339(raw).ok())
        .map(|timestamp| timestamp.timestamp())
        .unwrap_or_default()
}

/// 该行的完整动作清单（右键菜单），顺序即菜单顺序。
fn row_actions(row: &ListRow) -> Vec<RowAction> {
    let mut actions = vec![RowAction::Overview];
    if matches!(row.target, RowTarget::Container { .. }) {
        actions.extend([
            RowAction::Logs,
            RowAction::Terminal,
            RowAction::Stats,
            RowAction::Files,
        ]);
        actions.push(if row.running == Some(true) {
            RowAction::Stop
        } else {
            RowAction::Start
        });
        actions.push(RowAction::Restart);
        actions.push(RowAction::Rename);
    }
    if matches!(row.target, RowTarget::Image(_)) {
        actions.push(RowAction::Run);
    }
    actions.push(RowAction::CopyName);
    actions.push(RowAction::CopyId);
    actions.push(RowAction::Remove);
    actions
        .into_iter()
        .filter(|action| action.applies_to(&row.target))
        .collect()
}

/// 行内快捷按钮：容器给「启停 + 重启 + 删除」，镜像给「运行 + 删除」，卷与网络给「删除」。
///
/// 设计原理（Why）：这一列是用户扫一眼就要动手的地方，最高频的启停/删除不该只
/// 藏在详情头部或右键菜单里（真实窗口实测：只有名称的行要点完再找按钮，来回移动
/// 鼠标成本高）。其余动作（日志、终端、统计、复制）仍然只在头部与菜单，避免同一
/// 件事摆三个入口。
fn quick_actions(row: &ListRow) -> Vec<RowAction> {
    let actions: Vec<RowAction> = match row.target {
        RowTarget::Container { .. } => vec![
            if row.running == Some(true) {
                RowAction::Stop
            } else {
                RowAction::Start
            },
            RowAction::Restart,
            RowAction::Remove,
        ],
        RowTarget::Image(_) => vec![RowAction::Run, RowAction::Remove],
        RowTarget::Volume(_) | RowTarget::Network(_) => vec![RowAction::Remove],
        RowTarget::Compose(_) => Vec::new(),
    };
    actions
        .into_iter()
        .filter(|action| action.applies_to(&row.target))
        .take(QUICK_LIMIT)
        .collect()
}

/// 渲染列表；空结果给出可行动的提示而不是白屏。
fn render_rows(
    tab_id: TabId,
    rows: &[ListRow],
    selected: &Option<String>,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    if rows.is_empty() {
        return v_flex()
            .flex_1()
            .min_h_0()
            .items_center()
            .justify_center()
            .px_2()
            .gap_1()
            .text_sm()
            .text_color(p.muted_foreground)
            .child(Icon::new(IconName::Search).xsmall())
            .child(rust_i18n::t!("container.list.none").to_string())
            .into_any_element();
    }
    let app = cx.entity().clone();
    let mut list = v_flex().gap_0p5().w_full();
    for row in rows {
        let is_selected = selected.as_deref() == Some(row_key(&row.target));
        list = list.child(list_row(&app, tab_id, row, is_selected, p, cx));
    }
    div()
        .id("container-list-scroll")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .child(list)
        .into_any_element()
}

/// 单行渲染：状态点、标题副标题、快捷按钮与右键菜单。
fn list_row(
    app: &Entity<AppShell>,
    tab_id: TabId,
    row: &ListRow,
    selected: bool,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let group = format!("container-list-row-{}", row.key);
    let title = row.title.clone();
    let mut element = h_flex()
        .id(ElementId::Name(row.key.clone().into()))
        .group(group.as_str())
        .w_full()
        .gap_2()
        .px_2()
        .py_1p5()
        .rounded(p.radius.md)
        .items_center()
        .cursor_pointer()
        .hover(|style| style.bg(p.muted.opacity(0.5)));
    // 行首不再放类型图标：中栏已经按分类分页，每一行的类型是确定的，图标只是噪声。
    // 状态点也只给有运行态的条目，静态资源那枚灰点是纯装饰。
    if row.running.is_some() {
        element = element.child(status_dot(row, p));
    }
    let names = v_flex()
        .flex_1()
        .min_w_0()
        .gap_0p5()
        .child(div().text_sm().truncate().child(title));
    element = element.child(match &row.subtitle {
        Some(subtitle) => names
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .truncate()
                    .child(subtitle.clone()),
            )
            .into_any_element(),
        None => names.into_any_element(),
    });
    if let Some(badge) = row.badge.clone() {
        element = element.child(
            div()
                .text_xs()
                .flex_none()
                .text_color(p.muted_foreground)
                .child(badge),
        );
    }
    for action in quick_actions(row) {
        let target = row.target.clone();
        element = element.child(
            Button::new(ElementId::Name(
                format!("{}-action-{:?}", row.key, action).into(),
            ))
            .ghost()
            .xsmall()
            .icon(action.icon())
            // 同理：行内动作不该因为冒泡再触发一次「选中该行」。
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                this.run_row_action(tab_id, target.clone(), action, window, cx);
            })),
        );
    }
    // 勾选放在动作之后：列表扫描时先看到「这个容器能做什么」，多选是次要操作。
    if let Some(is_checked) = row.checked {
        let check_id = row_target_key_id(&row.target);
        element = element.child(
            Checkbox::new(ElementId::Name(format!("{}-check", row.key).into()))
                .small()
                .checked(is_checked)
                // 勾选只改勾选集合：不吞掉冒泡的话，行自身的点击会顺手把右侧详情
                // 切到这个容器，正在看的日志与统计就被清了。
                .on_click(cx.listener(move |this, &_, _, cx| {
                    cx.stop_propagation();
                    this.toggle_container_check(tab_id, check_id.clone(), cx);
                })),
        );
    }
    if selected {
        element = element
            .bg(p.sidebar_accent)
            .text_color(p.sidebar_accent_foreground);
    }
    let menu_target = row.target.clone();
    let menu_actions = row_actions(row);
    let menu_app = app.clone();
    let click_target = row.target.clone();
    let element = element.on_click(cx.listener(move |this, _, _, cx| {
        this.select_row(tab_id, &click_target, cx);
    }));
    element
        .context_menu(move |menu, _window, _cx| {
            let mut menu = menu;
            for action in menu_actions.clone() {
                // 破坏性动作与常规动作之间加分隔线，降低误点删除的概率。
                if action.is_danger() {
                    menu = menu.item(PopupMenuItem::separator());
                }
                let target = menu_target.clone();
                let entity = menu_app.clone();
                menu = menu.item(
                    PopupMenuItem::new(rust_i18n::t!(action.label_key()).to_string())
                        .icon(action.icon())
                        .on_click(move |_, window, cx| {
                            entity.update(cx, |this, cx| {
                                this.run_row_action(tab_id, target.clone(), action, window, cx);
                            });
                        }),
                );
            }
            menu
        })
        .into_any_element()
}

/// 状态色点：运行用主色、停止用弱色，静态资源用边框色。
fn status_dot(row: &ListRow, p: &Palette) -> AnyElement {
    let color = match row.running {
        Some(true) => p.success,
        Some(false) => p.muted_foreground,
        None => p.border,
    };
    div()
        .size_2()
        .flex_none()
        .rounded_full()
        .bg(color)
        .into_any_element()
}

/// 勾选行需要的主键：容器用 ID，与 `selected` 的取法一致。
fn row_target_key_id(target: &RowTarget) -> String {
    row_key(target).to_string()
}

/// 批量条：高度固定常驻，没有勾选时只占位不绘制内容。
///
/// 设计原理（Why）：条件渲染会让中栏在勾选/取消时上下跳一截，肉眼就是闪烁；
/// 固定高度后只有条内的文字与按钮切换，列表位置不变。
fn render_status_bar(
    tab_id: TabId,
    section: ContainerSection,
    count: usize,
    armed: bool,
    notice: Option<&str>,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let mut bar = h_flex()
        .gap_1()
        .items_center()
        .h(px(28.0))
        .px_2()
        .rounded(p.radius.md);
    if section != ContainerSection::Containers || count == 0 {
        // 没有批量选择时，这同一行用来放操作状态，不再额外加一条提示带。
        let Some(notice) = notice else {
            return bar.into_any_element();
        };
        return bar
            .bg(p.muted.opacity(0.35))
            .child(
                Icon::new(IconName::Info)
                    .xsmall()
                    .text_color(p.muted_foreground),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_xs()
                    .truncate()
                    .child(notice.to_string()),
            )
            .child(
                Button::new("container-notice-dismiss")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Close)
                    .tooltip(rust_i18n::t!("container.notice.dismiss").to_string())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.dismiss_container_notice(tab_id, cx);
                    })),
            )
            .into_any_element();
    }
    bar = bar.bg(p.muted.opacity(0.35)).child(
        div()
            .text_xs()
            .child(rust_i18n::t!("container.batch.selected", count = count).to_string()),
    );
    for action in [
        ContainerBatchAction::Start,
        ContainerBatchAction::Stop,
        ContainerBatchAction::Remove,
    ] {
        let label = if action.is_dangerous() && armed {
            rust_i18n::t!("container.batch.confirm", count = count).to_string()
        } else {
            rust_i18n::t!(action.label_key()).to_string()
        };
        bar = bar.child(
            Button::new(ElementId::Name(
                format!("container-batch-{}", action.label_key()).into(),
            ))
            .xsmall()
            .ghost()
            .icon(action.icon())
            .label(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.run_container_batch(tab_id, action, cx);
            })),
        );
    }
    bar = bar.child(div().flex_1()).child(
        Button::new("container-batch-clear")
            .xsmall()
            .ghost()
            .icon(IconName::Close)
            .label(rust_i18n::t!("container.batch.clear").to_string())
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(workspace) = this.container_workspace_mut(tab_id) {
                    workspace.checked.clear();
                    workspace.batch_armed = false;
                }
                cx.notify();
            })),
    );
    bar.into_any_element()
}
