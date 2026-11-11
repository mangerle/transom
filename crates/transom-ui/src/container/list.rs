//! 工作区中栏：搜索、状态筛选、排序与带操作的列表行。
//!
//! 设计原理（Why）：容器管理工具的主干交互是「快速定位一个条目，然后立刻对它
//! 做一件事」（Portainer、Docker Desktop、lazydocker 都是这个形态）。原先的中栏
//! 只能点击选中、再去右侧头部找按钮，长列表里来回移动鼠标成本很高，也没有排序
//! 与状态筛选。本模块把五种条目统一成 [`ListRow`] 描述，渲染与动作分发因此只有
//! 一处实现，新增分类只要补一个构造函数。
//!
//! 渲染函数只依赖工作区实体句柄、只读状态快照与 `Palette`：筛选/排序直接
//! `update` 实体，需要外壳的表单与弹窗经 [`ContainerWorkspaceEvent`] 上报；
//! 列表的键盘焦点与按键分发仍由外壳持有，键盘事件到来时按当前状态重算行集合。
//!
//! 代价与局限：行内只放启停与删除两个按钮，其余动作收进右键菜单，
//! 窄列表下仍要展开菜单；筛选与排序都在前端完成，不额外发请求。

use crate::app::AppShell;
use crate::container::CONTENT_HEADER_HEIGHT;
use crate::container::types::{
    ContainerPruneTarget, ContainerQuickFilter, ContainerSection, ContainerSortKey,
    ContainerStateFilter, ContainerWorkspaceState, RowAction, RowTarget, SortFields,
    compare_fields, image_passes_filter, matches_query, row_key, volume_passes_filter,
};
use crate::container::{ContainerWorkspace, ContainerWorkspaceEvent};
use crate::divider::{DividerEdge, DividerStyled};
use crate::format::format_size;
use crate::palette::Palette;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::{ContextMenuExt, DropdownMenu, PopupMenuItem};
use gpui_kit::*;
use transom_core::{
    ComposeProject, ContainerImageSummary, ContainerNetworkSummary, ContainerSummary,
    ContainerVolumeSummary, TabId, short_id,
};
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::CustomIcon;
use transom_ui_icons::UiIcon;

mod filters;
mod keyboard;
mod render;
mod rows;

pub(crate) use filters::*;
pub(crate) use keyboard::*;
pub(crate) use render::*;
pub(crate) use rows::*;

/// 行内快捷按钮数量上限：再多会把标题挤到不可读。
const QUICK_LIMIT: usize = 3;

/// 工具条渲染参数：分类、筛选与排序档位较多，收敛为结构体避免长参数列表。
pub(crate) struct ToolbarParams<'a> {
    pub(crate) section: ContainerSection,
    pub(crate) search: &'a Entity<InputState>,
    pub(crate) focus_handle: FocusHandle,
    pub(crate) state_filter: ContainerStateFilter,
    pub(crate) include_compose: bool,
    pub(crate) quick: ContainerQuickFilter,
    pub(crate) sort_key: ContainerSortKey,
    pub(crate) sort_desc: bool,
}

/// 一行的展示数据与动作目标。
#[derive(Clone)]
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

/// 中栏：工具条（搜索 + 筛选 + 排序）、列表与加载提示。
///
/// 设计原理（Why）：外壳只读一次实体状态并把句柄、快照与搜索词传进来；本函数
/// 不再反查 `AppShell`。焦点与按键装饰留在外壳的标签骨架里，避免把窗口级
/// 焦点管理塞进纯渲染函数。
pub(crate) fn render_container_list(
    app_entity: Entity<AppShell>,
    view: &Entity<ContainerWorkspace>,
    workspace: &ContainerWorkspaceState,
    query: &str,
    rem: Pixels,
    p: &Palette,
) -> AnyElement {
    let rows = list_rows(workspace, query);
    let toolbar = render_list_toolbar(
        view,
        &ToolbarParams {
            section: workspace.section,
            search: &workspace.search,
            focus_handle: workspace.list_focus_handle.clone(),
            state_filter: workspace.state_filter,
            include_compose: workspace.include_compose,
            quick: workspace.quick_filter,
            sort_key: workspace.sort_key,
            sort_desc: workspace.sort_desc,
        },
        p,
    );
    let body = render_rows(ContainerRowsParams {
        app_entity,
        view,
        rows,
        selected: workspace.selected.clone(),
        focus_handle: workspace.list_focus_handle.clone(),
        scroll: workspace.list_scroll.clone(),
        rem,
        palette: p,
    });
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
    if workspace.busy {
        content = content.child(
            h_flex()
                .gap_1()
                .items_center()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(AppIconElement::new(AppIcon::Ui(UiIcon::REFRESH)).xsmall())
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
    view: &Entity<ContainerWorkspace>,
    params: &ToolbarParams<'_>,
    p: &Palette,
) -> AnyElement {
    let main_action_btn: AnyElement = match params.section {
        ContainerSection::Containers => {
            let view = view.clone();
            Button::new("container-main-run")
                .xsmall()
                .ghost()
                .child(AppIconElement::new(AppIcon::Ui(UiIcon::RUN)).small())
                .accessibility_label(rust_i18n::t!("container.action.run").to_string())
                .tooltip(rust_i18n::t!("container.action.run").to_string())
                .on_click(move |_, _, cx| {
                    view.update(cx, |_workspace, cx| {
                        cx.emit(ContainerWorkspaceEvent::OpenRunForm(None));
                    });
                })
                .into_any_element()
        }
        ContainerSection::Images => {
            let view = view.clone();
            Button::new("container-main-explore-hub")
                .xsmall()
                .ghost()
                .child(AppIconElement::new(AppIcon::Custom(CustomIcon::DOCKER)).small())
                .accessibility_label(
                    rust_i18n::t!("container.pull_manager.explore_market").to_string(),
                )
                .tooltip(rust_i18n::t!("container.pull_manager.explore_market").to_string())
                .on_click(move |_, _, cx| {
                    view.update(cx, |_workspace, cx| {
                        cx.emit(ContainerWorkspaceEvent::OpenImageHub);
                    });
                })
                .into_any_element()
        }
        ContainerSection::Volumes => {
            let view = view.clone();
            Button::new("container-main-create-volume")
                .xsmall()
                .ghost()
                .child(AppIconElement::new(AppIcon::Ui(UiIcon::ADD)).small())
                .accessibility_label(rust_i18n::t!("container.create_dialog.volume").to_string())
                .tooltip(rust_i18n::t!("container.create_dialog.volume").to_string())
                .on_click(move |_, _, cx| {
                    view.update(cx, |_workspace, cx| {
                        cx.emit(ContainerWorkspaceEvent::OpenCreateDialog(
                            ContainerSection::Volumes,
                        ));
                    });
                })
                .into_any_element()
        }
        ContainerSection::Networks => {
            let view = view.clone();
            Button::new("container-main-create-network")
                .xsmall()
                .ghost()
                .child(AppIconElement::new(AppIcon::Ui(UiIcon::ADD)).small())
                .accessibility_label(rust_i18n::t!("container.create_dialog.network").to_string())
                .tooltip(rust_i18n::t!("container.create_dialog.network").to_string())
                .on_click(move |_, _, cx| {
                    view.update(cx, |_workspace, cx| {
                        cx.emit(ContainerWorkspaceEvent::OpenCreateDialog(
                            ContainerSection::Networks,
                        ));
                    });
                })
                .into_any_element()
        }
        ContainerSection::Compose => {
            let menu_view = view.clone();
            Button::new("container-main-new-compose")
                .xsmall()
                .ghost()
                .child(AppIconElement::new(AppIcon::Ui(UiIcon::ADD_FILE)).small())
                .accessibility_label(rust_i18n::t!("container.compose.new_yaml").to_string())
                .tooltip(rust_i18n::t!("container.compose.new_yaml").to_string())
                .dropdown_menu(move |menu, _, _| {
                    let mut menu = menu;
                    for template in crate::container::compose_templates::default_templates() {
                        let view = menu_view.clone();
                        let yaml = template.yaml.to_string();
                        menu = menu.item(PopupMenuItem::new(template.name.to_string()).on_click(
                            move |_, _, cx| {
                                view.update(cx, |_workspace, cx| {
                                    cx.emit(ContainerWorkspaceEvent::CreateComposeFromTemplate(
                                        yaml.clone(),
                                    ));
                                });
                            },
                        ));
                    }
                    menu = menu.item(PopupMenuItem::separator());
                    let view_import = menu_view.clone();
                    menu = menu.item(
                        PopupMenuItem::new(
                            rust_i18n::t!("container.compose.import_yaml").to_string(),
                        )
                        .on_click(move |_, _, cx| {
                            view_import.update(cx, |_workspace, cx| {
                                cx.emit(ContainerWorkspaceEvent::ImportCompose);
                            });
                        }),
                    );
                    menu
                })
                .into_any_element()
        }
    };

    let list_focus = params.focus_handle.clone();
    let search_input = div()
        .flex_1()
        .min_w_0()
        .on_action(move |_: &gpui_kit::component::input::Escape, window, cx| {
            window.focus(&list_focus, cx);
            cx.stop_propagation();
        })
        .child(Input::new(params.search));

    h_flex()
        .gap_1()
        .items_center()
        .child(search_input)
        .child(render_unified_filter_menu(view, params, p))
        .child(main_action_btn)
        .child(render_more_menu(view, params))
        .into_any_element()
}

impl AppShell {
    /// 键盘事件到来时按当前状态重算行集合，再交给列表键盘导航处理。
    ///
    /// 设计原理（Why）：行集合随搜索、筛选与排序实时变化；如果只在渲染帧捕获，
    /// 输入与按键同一帧交错时容易导航到旧行。这里复用 [`list_rows`] 的唯一
    /// 排序规则，保证键盘选择与画面一致。渲染函数本身不再挂 `impl AppShell`。
    pub(crate) fn handle_container_list_key(
        &mut self,
        tab_id: TabId,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.state.container_workspace(tab_id) else {
            return;
        };
        let rows = {
            let workspace = view.read(cx);
            let query = workspace.state.query(cx);
            list_rows(&workspace.state, &query)
        };
        handle_list_key_down(self, tab_id, &rows, event, window, cx);
    }
}
