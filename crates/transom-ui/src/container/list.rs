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
use crate::container::CONTENT_HEADER_HEIGHT;
use crate::container::types::{
    ContainerPruneTarget, ContainerQuickFilter, ContainerSection, ContainerSortKey,
    ContainerStateFilter, ContainerWorkspaceState, RowAction, RowTarget, SortFields,
    compare_fields, image_passes_filter, matches_query, row_key, volume_passes_filter,
};
use crate::divider::{DividerEdge, DividerStyled};
use crate::format::format_size;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::{ContextMenuExt, DropdownMenu, PopupMenuItem};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
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
    pub(crate) include_compose: bool,
    pub(crate) quick: ContainerQuickFilter,
    pub(crate) sort_key: ContainerSortKey,
    pub(crate) sort_desc: bool,
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
        let include_compose = workspace.include_compose;
        let sort_key = workspace.sort_key;
        let sort_desc = workspace.sort_desc;
        let busy = workspace.busy;
        let selected = workspace.selected.clone();
        let quick = workspace.quick_filter;
        let query = workspace.query(cx).to_lowercase();
        let mut rows = collect_rows(section, workspace, &query, state_filter);
        rows.sort_by(|left, right| {
            compare_fields(
                &left.fields,
                &right.fields,
                active_sort(section, sort_key),
                sort_desc,
            )
        });

        let toolbar = self.render_list_toolbar(
            &ToolbarParams {
                tab_id,
                section,
                search: &search,
                state_filter,
                include_compose,
                quick,
                sort_key,
                sort_desc,
            },
            p,
            cx,
        );
        let body = render_rows(tab_id, &rows, &selected, p, cx);
        let toolbar_header = h_flex()
            .h(rems(CONTENT_HEADER_HEIGHT))
            .flex_none()
            .px_2()
            .items_center()
            .overflow_hidden()
            .border_color(p.divider(p.border))
            .divider_edge(DividerEdge::Bottom, p.divider_width)
            .child(div().flex_1().min_w_0().child(toolbar));
        let mut content = v_flex()
            .flex_1()
            .min_h_0()
            .w_full()
            .overflow_hidden()
            .px_2()
            .pb_2()
            .child(body);
        if busy {
            content = content.child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(Icon::new(IconName::Loader).xsmall())
                    .child(rust_i18n::t!("container.loading").to_string()),
            );
        }
        v_flex()
            .h_full()
            .w_full()
            .min_w_0()
            .overflow_hidden()
            .child(toolbar_header)
            .child(content)
            .into_any_element()
    }

    /// 工具条：遵循「一屏一个主行动」原则，将原本 8 个拥挤控件严整收拢为 4 个：
    /// 1. 搜索框（flex-1 自适应）
    /// 2. 统合筛选（状态 + 快速清理 + 排序，带激活状态小圆点）
    /// 3. 主行动按钮（运行 / 拉取 / 新建）
    /// 4. 更多菜单（构建、探索、导入、深度清理等辅助动作）
    fn render_list_toolbar(
        &mut self,
        params: &ToolbarParams<'_>,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tab_id = params.tab_id;
        let main_action_btn = match params.section {
            ContainerSection::Containers => Button::new("container-main-run")
                .xsmall()
                .primary()
                .icon(IconName::Play)
                .tooltip(rust_i18n::t!("container.action.run").to_string())
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.open_container_run_form(tab_id, None, window, cx);
                })),
            ContainerSection::Images => Button::new("container-main-pull")
                .xsmall()
                .primary()
                .icon(IconName::Download)
                .tooltip(rust_i18n::t!("container.action.pull").to_string())
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.open_container_create_dialog(tab_id, ContainerSection::Images, window, cx);
                })),
            ContainerSection::Volumes => Button::new("container-main-create-volume")
                .xsmall()
                .primary()
                .icon(IconName::Plus)
                .tooltip(rust_i18n::t!("container.create_dialog.volume").to_string())
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.open_container_create_dialog(tab_id, ContainerSection::Volumes, window, cx);
                })),
            ContainerSection::Networks => Button::new("container-main-create-network")
                .xsmall()
                .primary()
                .icon(IconName::Plus)
                .tooltip(rust_i18n::t!("container.create_dialog.network").to_string())
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.open_container_create_dialog(tab_id, ContainerSection::Networks, window, cx);
                })),
            ContainerSection::Compose => Button::new("container-main-new-compose")
                .xsmall()
                .primary()
                .icon(IconName::FilePlus)
                .tooltip(rust_i18n::t!("container.compose.new_yaml").to_string())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.create_local_compose(tab_id, cx);
                })),
        };

        h_flex()
            .gap_1()
            .items_center()
            .child(Input::new(params.search).flex_1().min_w_0())
            .child(self.render_unified_filter_menu(params, p, cx))
            .child(main_action_btn)
            .child(self.render_more_menu(params, cx))
            .into_any_element()
    }
}
