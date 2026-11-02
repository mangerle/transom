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

mod filters;
mod render;
mod rows;

pub(crate) use filters::*;
pub(crate) use render::*;
pub(crate) use rows::*;

/// 行内快捷按钮数量上限：再多会把标题挤到不可读。
const QUICK_LIMIT: usize = 3;

/// 工具条渲染参数：分类、筛选与排序档位较多，收敛为结构体避免长参数列表。
pub(crate) struct ToolbarParams<'a> {
    pub(crate) tab_id: TabId,
    pub(crate) section: ContainerSection,
    pub(crate) search: &'a Entity<InputState>,
    pub(crate) state_filter: ContainerStateFilter,
    pub(crate) quick: ContainerQuickFilter,
    pub(crate) sort_key: ContainerSortKey,
    pub(crate) sort_desc: bool,
    /// CLI 兜底后端拿不到挂载明细，「仅未使用卷」档位不可用。
    pub(crate) cli_backend: bool,
    /// 已按下一次清理、等待第二次确认的目标；`None` 表示没有待确认动作。
    pub(crate) prune_armed: Option<ContainerPruneTarget>,
}

/// 一行的展示数据与动作目标。
pub(crate) struct ListRow {
    /// 与条目主键拼成稳定 ElementId 的前缀。
    pub(crate) key: String,
    pub(crate) title: String,
    /// 副标题：容器只有名称，静态资源才带大小/驱动等次要信息。
    pub(crate) subtitle: Option<String>,
    pub(crate) badge: Option<String>,
    /// 运行态：容器与编排项目有，静态资源没有。
    pub(crate) running: Option<bool>,
    /// 多选勾选状态：只有容器分类需要。
    pub(crate) checked: Option<bool>,
    pub(crate) target: RowTarget,
    pub(crate) fields: SortFields,
}

impl AppShell {
    /// 中栏：工具条（搜索 + 筛选 + 排序）、列表、新建行与计数页脚。
    pub(crate) fn render_container_list(
        &mut self,
        tab_id: TabId,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(workspace) = self.state.container_workspace(tab_id) else {
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
}
