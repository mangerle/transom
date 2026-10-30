//! 工作区中栏：搜索、状态筛选、排序与带操作的列表行。
//!
//! 设计原理（Why）：容器管理工具的主干交互是「快速定位一个条目，然后立刻对它
//! 做一件事」（Portainer、Docker Desktop、lazydocker 都是这个形态）。原先的中栏
//! 只能点击选中、再去右侧头部找按钮，长列表里来回移动鼠标成本很高，也没有排序
//! 与状态筛选。本模块把五种条目统一成 [`ListRow`] 描述，渲染与动作分发因此只有
//! 一处实现，新增分类只要补一个构造函数。
//!
//! 代价与局限（Trade-offs）：行内只放三个快捷按钮，其余动作收进右键菜单，窄列表
//! 下仍要展开菜单；筛选与排序都在前端完成，不额外发请求。

use crate::app::AppShell;
use crate::container::types::{
    ContainerBatchAction, ContainerSection, ContainerSortKey, ContainerStateFilter,
    ContainerWorkspaceState, RowAction, RowTarget, SortFields, compare_fields, matches_query,
    row_key,
};
use crate::format::format_size;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
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
    sort_key: ContainerSortKey,
    sort_desc: bool,
}

/// 一行的展示数据与动作目标。
struct ListRow {
    /// 与条目主键拼成稳定 ElementId 的前缀。
    key: String,
    icon: IconName,
    title: String,
    subtitle: String,
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
        let create_input = workspace.create_input.clone();
        let state_filter = workspace.state_filter;
        let sort_key = workspace.sort_key;
        let sort_desc = workspace.sort_desc;
        let busy = workspace.busy;
        let selected = workspace.selected.clone();
        let checked = workspace.checked.clone();
        let batch_armed = workspace.batch_armed;
        let query = workspace.query(cx).to_lowercase();
        let mut rows = collect_rows(section, workspace, &query, state_filter, &checked);
        let total = rows.len();
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
        let batch_bar = if checked_count == 0 {
            None
        } else {
            Some(render_batch_bar(tab_id, checked_count, batch_armed, p, cx))
        };
        let toolbar = self.render_list_toolbar(
            &ToolbarParams {
                tab_id,
                section,
                search: &search,
                state_filter,
                sort_key,
                sort_desc,
            },
            cx,
        );
        let body = render_rows(tab_id, &rows, &selected, p, cx);
        let footer = render_footer(&rows, total, &query, state_filter, p);
        let mut column = v_flex()
            .h_full()
            .gap_1p5()
            .p_2()
            .child(toolbar)
            .children(batch_bar)
            .child(body);
        if matches!(
            section,
            ContainerSection::Images | ContainerSection::Volumes | ContainerSection::Networks
        ) {
            column = column.child(self.render_create_row(tab_id, section, &create_input, cx));
        }
        column = column.child(footer);
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

    /// 工具条：搜索框、状态三档筛选与排序键（点击同键切换升降序）。
    fn render_list_toolbar(
        &mut self,
        params: &ToolbarParams<'_>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tab_id = params.tab_id;
        let section = params.section;
        let sort_desc = params.sort_desc;
        let mut toolbar = h_flex()
            .gap_1()
            .items_center()
            .child(Input::new(params.search).flex_1().min_w_0());
        if section == ContainerSection::Containers {
            toolbar = toolbar.child(
                Button::new("container-run-open")
                    .xsmall()
                    .ghost()
                    .icon(IconName::Play)
                    .label(rust_i18n::t!("container.run.title").to_string())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_container_run_form(tab_id, None, window, cx);
                    })),
            );
        }
        if section == ContainerSection::Containers {
            for filter in ContainerStateFilter::ALL {
                toolbar = toolbar.child(
                    Button::new(ElementId::Name(
                        format!("container-filter-{}", filter.label_key()).into(),
                    ))
                    .xsmall()
                    .ghost()
                    .selected(filter == params.state_filter)
                    .label(rust_i18n::t!(filter.label_key()).to_string())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(workspace) = this.container_workspace_mut(tab_id) {
                            workspace.state_filter = filter;
                        }
                        cx.notify();
                    })),
                );
            }
        }
        let keys = ContainerSortKey::for_section(section);
        let mut sorts = h_flex().gap_0p5().items_center();
        for key in keys {
            sorts = sorts.child(
                Button::new(ElementId::Name(
                    format!("container-sort-{}", key.label_key()).into(),
                ))
                .xsmall()
                .ghost()
                .selected(*key == active_sort(section, params.sort_key))
                .label(rust_i18n::t!(key.label_key()).to_string())
                .on_click(cx.listener(move |this, _, _, cx| {
                    let Some(workspace) = this.container_workspace_mut(tab_id) else {
                        return;
                    };
                    let current = active_sort(workspace.section, workspace.sort_key);
                    if current == *key {
                        workspace.sort_desc = !workspace.sort_desc;
                    } else {
                        workspace.sort_key = *key;
                        workspace.sort_desc = false;
                    }
                    cx.notify();
                })),
            );
        }
        toolbar = toolbar.child(sorts).child(
            Button::new("container-sort-direction")
                .xsmall()
                .ghost()
                .icon(if sort_desc {
                    IconName::ArrowDown
                } else {
                    IconName::ArrowUp
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(workspace) = this.container_workspace_mut(tab_id) {
                        workspace.sort_desc = !workspace.sort_desc;
                    }
                    cx.notify();
                })),
        );
        toolbar.into_any_element()
    }

    /// 拉取/新建行：镜像填引用，卷与网络填名称。
    fn render_create_row(
        &mut self,
        tab_id: TabId,
        section: ContainerSection,
        create_input: &Entity<InputState>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        h_flex()
            .gap_1()
            .items_center()
            .child(Input::new(create_input).flex_1().min_w_0())
            .child(
                Button::new("container-create-submit")
                    .primary()
                    .xsmall()
                    .icon(IconName::Plus)
                    .label(if section == ContainerSection::Images {
                        rust_i18n::t!("container.pull").to_string()
                    } else {
                        rust_i18n::t!("container.create").to_string()
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.submit_container_create(tab_id, window, cx);
                    })),
            )
            .into_any_element()
    }
}

/// 当前生效的排序键：分类切换后旧键可能不在新分类的可用档位里。
fn active_sort(section: ContainerSection, sort_key: ContainerSortKey) -> ContainerSortKey {
    if ContainerSortKey::for_section(section).contains(&sort_key) {
        sort_key
    } else {
        ContainerSortKey::default_for(section)
    }
}

/// 按分类收集行数据，并套用状态筛选与关键词搜索。
fn collect_rows(
    section: ContainerSection,
    workspace: &ContainerWorkspaceState,
    query: &str,
    filter: ContainerStateFilter,
    checked: &HashSet<String>,
) -> Vec<ListRow> {
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
            .map(image_row_spec)
            .collect(),
        ContainerSection::Volumes => workspace
            .volumes
            .iter()
            .filter(|item| matches_query(query, &[item.name.clone(), item.driver.clone()]))
            .map(volume_row_spec)
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
        icon: IconName::Container,
        title: item.name.clone(),
        subtitle: format!("{} · {}", item.image, item.status),
        badge: Some(item.state.key().to_string()),
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
        icon: IconName::Layers,
        title: item.display_name(),
        subtitle: format!("{} · {}", format_size(item.size), short_id(&item.id)),
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

fn volume_row_spec(item: &ContainerVolumeSummary) -> ListRow {
    ListRow {
        key: format!("volume-row-{}", item.name),
        icon: IconName::Database,
        title: item.name.clone(),
        subtitle: format!("{} · {}", item.driver, item.mountpoint),
        badge: None,
        running: None,
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
        icon: IconName::Network,
        title: item.name.clone(),
        subtitle: format!(
            "{} · {}{}",
            item.driver,
            item.scope,
            item.subnet
                .as_ref()
                .map(|subnet| format!(" · {subnet}"))
                .unwrap_or_default()
        ),
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
        icon: IconName::Boxes,
        title: item.name.clone(),
        subtitle: format!(
            "{} · {}",
            item.state_label(),
            item.working_dir.clone().unwrap_or_default()
        ),
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

/// 行内快捷按钮：容器给「启停 + 日志 + 终端」，静态资源给「复制 + 删除」。
fn quick_actions(row: &ListRow) -> Vec<RowAction> {
    let actions: Vec<RowAction> = match row.target {
        RowTarget::Container { .. } => vec![
            if row.running == Some(true) {
                RowAction::Stop
            } else {
                RowAction::Start
            },
            RowAction::Logs,
            RowAction::Terminal,
        ],
        RowTarget::Image(_) => vec![RowAction::Run, RowAction::CopyId, RowAction::Remove],
        RowTarget::Volume(_) | RowTarget::Network(_) => vec![RowAction::CopyId, RowAction::Remove],
        RowTarget::Compose(_) => vec![RowAction::Overview],
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
    let mut list = v_flex().gap_0p5();
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
    let subtitle = row.subtitle.clone();
    let mut element = h_flex()
        .id(ElementId::Name(row.key.clone().into()))
        .group(group.as_str())
        .gap_2()
        .px_2()
        .py_1p5()
        .rounded(p.radius.md)
        .items_center()
        .cursor_pointer()
        .hover(|style| style.bg(p.muted.opacity(0.5)))
        .child(status_dot(row, p))
        .child(Icon::new(row.icon).xsmall().text_color(p.muted_foreground))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(div().text_sm().truncate().child(title))
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .truncate()
                        .child(subtitle),
                ),
        );
    if let Some(is_checked) = row.checked {
        let check_id = row_target_key_id(&row.target);
        let app = app.clone();
        element = element.child(
            Checkbox::new(ElementId::Name(format!("{}-check", row.key).into()))
                .small()
                .checked(is_checked)
                .on_click(move |&_, _, cx| {
                    let check_id = check_id.clone();
                    app.clone().update(cx, |this, cx| {
                        this.toggle_container_check(tab_id, check_id, cx);
                    });
                }),
        );
    }
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
        let entity = app.clone();
        element = element.child(
            Button::new(ElementId::Name(
                format!("{}-action-{:?}", row.key, action).into(),
            ))
            .ghost()
            .xsmall()
            .icon(action.icon())
            .on_click(move |_, window, cx| {
                entity.clone().update(cx, |this, cx| {
                    this.run_row_action(tab_id, target.clone(), action, window, cx);
                });
            }),
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

/// 页脚：筛选生效时报告「显示 / 总数」，让用户知道少掉的是筛掉的而不是丢了。
fn render_footer(
    rows: &[ListRow],
    total: usize,
    query: &str,
    filter: ContainerStateFilter,
    p: &Palette,
) -> AnyElement {
    let filtered = !query.is_empty() || filter != ContainerStateFilter::All;
    if !filtered || rows.len() == total {
        return div().into_any_element();
    }
    div()
        .text_xs()
        .text_color(p.muted_foreground)
        .child(
            rust_i18n::t!("container.list.matching", shown = rows.len(), total = total).to_string(),
        )
        .into_any_element()
}

/// 勾选行需要的主键：容器用 ID，与 `selected` 的取法一致。
fn row_target_key_id(target: &RowTarget) -> String {
    row_key(target).to_string()
}

/// 批量条：勾选后出现，删除按钮需要连点两次。
fn render_batch_bar(
    tab_id: TabId,
    count: usize,
    armed: bool,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let mut bar = h_flex()
        .gap_1()
        .items_center()
        .px_2()
        .py_1()
        .rounded(p.radius.md)
        .bg(p.muted.opacity(0.35))
        .child(
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
