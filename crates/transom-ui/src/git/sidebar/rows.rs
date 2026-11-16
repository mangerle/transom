//! 侧栏行控件：分组行与项目行的几何、配色与拖放落点。
//!
//! 设计原理（Why）：尺寸与配色令牌整套对齐会话侧栏（见 `widgets/sidebar_row`）——
//! 通过 `Rems` 令牌实现（默认密度下为 38px 行槽位套 34px 圆角内容条并随 UI 密度缩放），
//! 普通、选中和当前三态由共享组件统一决定。

use super::{
    GitFolderRow, GitProjectRow, GitSidebarEvent, GitSidebarView, emit_sidebar,
    render_git_folder_menu, render_git_project_menu,
};
use crate::git::types::{GitBadge, GitDropTarget, GitSidebarDrag};
use crate::palette::Palette;
use crate::session_appearance::session_color;
use crate::theme::tokens::size;
use crate::widgets::{SidebarRowState, sidebar_group_row_band, sidebar_row_band};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::*;
use transom_core::{DropSide, GitFolderId};
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

const ROW_HEIGHT: Rems = rems(size::TREE_ROW_HEIGHT);
const ROW_ICON_SIZE: Rems = rems(size::TREE_ROW_ICON);
const ROW_CHEVRON_SIZE: Rems = rems(size::TREE_CHEVRON);
const INDENT_BASE: f32 = 6.0;
const INDENT_STEP: f32 = 18.0;

pub(super) struct GitProjectRowProps<'a> {
    pub(super) view: &'a Entity<GitSidebarView>,
    pub(super) row: &'a GitProjectRow,
    pub(super) badge: &'a GitBadge,
    pub(super) active_branch: Option<&'a str>,
    pub(super) row_state: SidebarRowState,
    pub(super) folders: &'a [(GitFolderId, String)],
}

/// 渲染一个分组行：折叠开关、组内计数、上下三分之一换序、中段收纳项目。
pub(super) fn render_git_folder_row(
    view: &Entity<GitSidebarView>,
    row: &GitFolderRow,
    p: &Palette,
) -> AnyElement {
    let folder_id = row.id;
    let click_view = view.clone();
    let menu_view = view.clone();
    let anchor = GitSidebarDrag::Folder(folder_id);
    let label = row.name.clone();

    let element = div()
        .id(ElementId::Name(format!("git-folder-{folder_id}").into()))
        .w_full()
        .h(ROW_HEIGHT)
        .pl(px(INDENT_BASE))
        .pr(px(4.0))
        .flex()
        .items_center()
        .cursor_pointer()
        .on_click(move |_, _, cx| {
            emit_sidebar(
                &click_view,
                GitSidebarEvent::ToggleFolderCollapsed(folder_id),
                cx,
            );
        })
        .on_drag(anchor, move |_, _, _, cx| {
            cx.new(|_| GitDragPreview::new(label.clone(), UiIcon::FOLDER.into()))
        })
        .context_menu(move |menu, _window, _cx| {
            render_git_folder_menu(menu, menu_view.clone(), folder_id)
        });

    let into_bg = p.drop_target;
    let band = sidebar_group_row_band(
        ElementId::Name(format!("git-folder-band-{folder_id}").into()),
        p,
    )
    .drag_over::<GitSidebarDrag>(move |style, _, _, _| style.bg(into_bg))
    .child(
        AppIconElement::new(if row.collapsed {
            UiIcon::CHEVRON_RIGHT
        } else {
            UiIcon::CHEVRON_DOWN
        })
        .with_rems(ROW_CHEVRON_SIZE),
    )
    .child(AppIconElement::new(UiIcon::FOLDER).with_rems(ROW_ICON_SIZE))
    .child(
        div()
            .flex_1()
            .overflow_hidden()
            .text_sm()
            .font_weight(FontWeight::SEMIBOLD)
            .truncate()
            .child(row.name.clone()),
    )
    .child(
        div()
            .text_xs()
            .text_color(p.text.muted)
            .child(row.project_count.to_string()),
    )
    .child(reorder_overlay(view, anchor, true, p));
    element.child(band).into_any_element()
}

/// 渲染一个项目行：活动底色、拖拽源与上下半区落点。
pub(super) fn render_git_project_row(props: GitProjectRowProps<'_>, p: &Palette) -> AnyElement {
    let GitProjectRowProps {
        view,
        row,
        badge,
        active_branch,
        row_state,
        folders,
    } = props;
    let project = &row.project;
    let id = project.id;
    let name = project.display_name();

    let branch = badge
        .branch
        .as_deref()
        .or(active_branch)
        .map(str::to_string)
        .unwrap_or_else(|| {
            if badge.error.is_some() {
                rust_i18n::t!("git.sidebar.unavailable").to_string()
            } else if badge.loading {
                rust_i18n::t!("git.sidebar.loading").to_string()
            } else {
                "-".to_string()
            }
        });

    let click_view = view.clone();
    let menu_view = view.clone();
    let menu_project = project.clone();
    let menu_folders = folders.to_vec();
    let project_for_click = project.clone();
    let drag_label = name.clone();
    let icon_name = UiIcon::VCS.into();

    let element = div()
        .id(ElementId::Name(format!("git-project-{id}").into()))
        .w_full()
        .h(ROW_HEIGHT)
        .pl(px(
            INDENT_BASE + if row.in_folder { INDENT_STEP } else { 0.0 }
        ))
        .pr(px(4.0))
        .flex()
        .items_center()
        .cursor_pointer()
        .on_click(move |event: &ClickEvent, _, cx| {
            if event.click_count() >= 2 {
                emit_sidebar(
                    &click_view,
                    GitSidebarEvent::OpenProjectTab(project_for_click.clone()),
                    cx,
                );
            } else {
                emit_sidebar(&click_view, GitSidebarEvent::SelectProject(id), cx);
            }
        })
        .on_drag(GitSidebarDrag::Project(id), move |_, _, _, cx| {
            cx.new(|_| GitDragPreview::new(drag_label.clone(), icon_name))
        })
        .context_menu(move |menu, _window, _cx| {
            render_git_project_menu(
                menu,
                menu_view.clone(),
                menu_project.clone(),
                menu_folders.clone(),
            )
        });

    let mut band = sidebar_row_band(
        ElementId::Name(format!("git-project-band-{id}").into()),
        row_state,
        p,
    )
    .child(
        AppIconElement::new(icon_name)
            .with_rems(ROW_ICON_SIZE)
            .text_color(
                session_color(project.color)
                    .unwrap_or_else(|| state_icon_color(badge, row_state, p)),
            ),
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
                    .text_color(if badge.error.is_some() {
                        p.danger
                    } else {
                        p.muted_foreground
                    })
                    .child(branch),
            ),
    );
    if badge.dirty > 0 {
        band = band.child(
            div()
                .text_xs()
                .text_color(p.text.muted)
                .child(badge.dirty.to_string()),
        );
    }
    let band = band.child(reorder_overlay(view, GitSidebarDrag::Project(id), false, p));
    element.child(band).into_any_element()
}

fn state_icon_color(badge: &GitBadge, row_state: SidebarRowState, p: &Palette) -> Hsla {
    if badge.error.is_some() {
        p.danger
    } else if badge.loading {
        p.muted_foreground
    } else if badge.dirty > 0 {
        p.warning
    } else if row_state.is_highlighted() {
        p.sidebar_accent_foreground
    } else {
        p.primary
    }
}

/// 拖动时跟随鼠标的预览卡片：与会话侧栏同款。
pub(crate) struct GitDragPreview {
    label: String,
    icon: AppIcon,
}

impl GitDragPreview {
    fn new(label: String, icon: AppIcon) -> Self {
        Self { label, icon }
    }
}

impl Render for GitDragPreview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::from(cx);
        h_flex()
            .px_3()
            .py_1p5()
            .gap_2()
            .items_center()
            .rounded(p.radius.md)
            .bg(p.popover)
            .border_1()
            .border_color(p.border)
            .shadow_md()
            .text_sm()
            .text_color(p.popover_foreground)
            .child(
                AppIconElement::new(self.icon)
                    .xsmall()
                    .text_color(p.muted_foreground),
            )
            .child(self.label.clone())
    }
}

fn reorder_overlay(
    view: &Entity<GitSidebarView>,
    anchor: GitSidebarDrag,
    keep_middle: bool,
    p: &Palette,
) -> AnyElement {
    let overlay = div()
        .absolute()
        .inset_0()
        .flex()
        .flex_col()
        .child(reorder_half(view, anchor, DropSide::Before, true, p));
    let overlay = if keep_middle {
        overlay.child(div().flex_1())
    } else {
        overlay
    };
    overlay
        .child(reorder_half(view, anchor, DropSide::After, false, p))
        .into_any_element()
}

fn reorder_half(
    view: &Entity<GitSidebarView>,
    anchor: GitSidebarDrag,
    side: DropSide,
    top: bool,
    p: &Palette,
) -> AnyElement {
    let target = GitDropTarget::Relative { anchor, side };
    let line_color = p.sidebar_accent_foreground;
    let zone = div().flex_1().border_color(line_color);
    let zone = if top {
        zone.drag_over::<GitSidebarDrag>(move |style, _, _, _| {
            style.border_t_2().border_color(line_color)
        })
    } else {
        zone.drag_over::<GitSidebarDrag>(move |style, _, _, _| {
            style.border_b_2().border_color(line_color)
        })
    };
    let view = view.clone();
    zone.on_drop(move |drag: &GitSidebarDrag, window, cx| {
        let drag = *drag;
        emit_sidebar(&view, GitSidebarEvent::Drop(drag, target), cx);
        window.refresh();
    })
    .into_any_element()
}
