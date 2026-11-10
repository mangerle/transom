//! 容器工作区主视图：头部、列表/详情分栏与拉取抽屉的纯渲染。
//!
//! 设计原理（Why）：标签页正文（头部、列表、详情、抽屉）只依赖工作区实体、
//! 只读状态快照与 `Palette`，全部改为自由渲染函数；窗口级焦点、键盘包装、
//! 通知转弹与全屏覆盖层由外壳在 `workspace/panes.rs` 的最小入口完成。
//! 这样容器渲染切片不再需要 `Context<AppShell>`，也不会在渲染期反查外壳。

mod header;

use crate::app::AppShell;
use crate::container::detail::{ContainerDetailParams, render_container_detail};
use crate::container::list::render_container_list;
use crate::container::types::ContainerWorkspaceStatus;
use crate::container::{ContainerWorkspace, ContainerWorkspaceState};
use crate::divider::h_resizable;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::v_flex;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::resizable::resizable_panel;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use header::render_container_header;

/// 渲染容器标签页正文：头部、列表/详情分栏与拉取抽屉。
///
/// 设计原理（Why）：`list_panel` 由外壳预先包好焦点与键盘上下文，正文函数
/// 只负责布局；这样窗口级键盘语义留在外壳，正文渲染保持无副作用。
pub(crate) fn render_container_tab_body(
    view: &Entity<ContainerWorkspace>,
    tab_id: transom_core::TabId,
    state: &ContainerWorkspaceState,
    cx: &App,
    fullscreen: bool,
    list_panel: AnyElement,
    p: &Palette,
) -> AnyElement {
    let header = render_container_header(view, state, fullscreen, p);

    // 输入框与编辑器文本必须经 App 读取；详情渲染函数只消费快照。
    let compose_yaml_text = state
        .compose_yaml_editor
        .as_ref()
        .map(|(_, _, editor, _)| editor.read(cx).value())
        .unwrap_or_default();
    let logs_filter = state.logs_search_query(cx);
    let changes_search_value = state.changes_search.read(cx).value().to_string();
    let inspect_keyword = state.inspect_search.read(cx).value().trim().to_string();
    let detail = render_container_detail(
        ContainerDetailParams {
            tab_id,
            view,
            workspace: state,
            compose_yaml_text: &compose_yaml_text,
            logs_filter: &logs_filter,
            changes_search_value: &changes_search_value,
            inspect_keyword: &inspect_keyword,
        },
        p,
    );

    // 列表与详情宽度可拖拽：长容器名与镜像引用在固定宽度下必然截断，详情区在
    // 小窗下又铺不开，与主窗口其他分区一样交给用户拖。默认值只是首次打开的起点。
    let list_split = state.list_split.clone();
    let list_and_detail = h_resizable("container-split-list")
        .with_state(&list_split)
        .child(
            resizable_panel()
                .size(px(380.0))
                .size_range(px(260.0)..px(680.0))
                .fixed()
                // 面板本身不裁剪内容：工具条与行一旦超出宽度就整体压到右栏上
                // （真实窗口实测），这里补一层裁剪容器把中栏锁在自己的宽度内。
                .child(
                    div()
                        .size_full()
                        .min_w_0()
                        .overflow_hidden()
                        .child(list_panel),
                ),
        )
        .child(resizable_panel().min_w_0().child(detail));

    let pull_drawer = crate::container::render_pull_drawer(view, state, p);

    v_flex()
        .relative()
        .size_full()
        .overflow_hidden()
        .bg(p.background)
        .child(header)
        .child(div().flex_1().min_w_0().min_h_0().child(list_and_detail))
        .child(pull_drawer)
        .into_any_element()
}

/// 渲染列表栏原始内容；焦点与键盘包装由外壳完成。
pub(crate) fn render_container_list_content(
    app_entity: Entity<AppShell>,
    view: &Entity<ContainerWorkspace>,
    state: &ContainerWorkspaceState,
    cx: &App,
    rem: Pixels,
    p: &Palette,
) -> AnyElement {
    let query = state.query(cx);
    render_container_list(app_entity, view, state, &query, rem, p)
}

/// 渲染容器工作区的连接态占位页。
///
/// 重试按钮直接调用工作区实体的重连方法；渲染函数不再持有外壳上下文。
pub(crate) fn render_container_connection_state(
    view: &Entity<ContainerWorkspace>,
    state: &ContainerWorkspaceState,
    p: &Palette,
) -> AnyElement {
    let status = state.status.clone();
    let profile = state.profile.clone();
    let (icon, title, detail, retry) = match status {
        ContainerWorkspaceStatus::Connecting => (
            IconName::Loader,
            rust_i18n::t!("container.state.connecting").to_string(),
            profile.engine.describe(),
            false,
        ),
        ContainerWorkspaceStatus::Failed(error) => (
            IconName::CircleAlert,
            rust_i18n::t!("container.state.failed").to_string(),
            error,
            true,
        ),
        ContainerWorkspaceStatus::Disconnected => (
            IconName::CircleAlert,
            rust_i18n::t!("container.state.disconnected").to_string(),
            profile.engine.describe(),
            true,
        ),
        ContainerWorkspaceStatus::Connected => {
            (IconName::Container, String::new(), String::new(), false)
        }
    };
    let mut body = v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap_3()
        .child(
            Icon::new(icon)
                .with_size(px(28.0))
                .text_color(p.muted_foreground),
        )
        .child(div().text_sm().child(title))
        .child(div().text_xs().text_color(p.muted_foreground).child(detail));
    if retry {
        body = body.child(
            Button::new("container-retry")
                .primary()
                .small()
                .icon(IconName::RefreshCw)
                .label(rust_i18n::t!("common.retry").to_string())
                .on_click({
                    let view = view.clone();
                    move |_, _, cx| {
                        view.update(cx, |workspace, cx| workspace.reconnect(cx));
                    }
                }),
        );
    }
    body.into_any_element()
}
