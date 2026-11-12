//! 侧栏行控件：分组行与项目行的几何、配色与拖放落点。
//!
//! 设计原理（Why）：尺寸与配色令牌整套对齐会话侧栏（见 `widgets/sidebar_row`）——
//! 通过 `Rems` 令牌实现（默认密度下为 38px 行槽位套 34px 圆角内容条并随 UI 密度缩放），
//! 普通、选中和当前三态由共享组件统一决定。
//! 两个面板是同一个壳里的同类清单，用户切换活动栏条目时不该重新学一遍「哪一行是我正在看的」。
//! 行槽位与内容条分离还有第二个理由：槽位负责命中与缩进，内容条负责高亮，
//! 高亮背景才不会贴到面板边缘、也不会盖住缩进区。

use super::{GitFolderRow, GitProjectRow, render_git_folder_menu, render_git_project_menu};
use crate::app::AppShell;
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

/// 行槽位高度：通过 `tokens::size` 与会话侧栏对齐，切换面板与调整 UI 密度时行高同步缩放。
const ROW_HEIGHT: Rems = rems(size::TREE_ROW_HEIGHT);
/// 行首图标尺寸：与内容条匹配并随 `rem` 缩放。
const ROW_ICON_SIZE: Rems = rems(size::TREE_ROW_ICON);
/// 折叠箭头尺寸：比行图标小一档，避免与文件夹图标抢视觉重心。
const ROW_CHEVRON_SIZE: Rems = rems(size::TREE_CHEVRON);
/// 缩进基准与每级步长：分组下的项目比根层缩进一级。
const INDENT_BASE: f32 = 6.0;
const INDENT_STEP: f32 = 18.0;

impl AppShell {
    /// 渲染一个分组行：折叠开关、组内计数、上下三分之一换序、中段收纳项目。
    pub(crate) fn render_git_folder_row(
        &mut self,
        row: &GitFolderRow,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let folder_id = row.id;
        let app_entity = cx.entity().clone();
        // 遮罩与菜单都要握住面板句柄，各自留一份：闭包会移走它。
        let overlay_app = app_entity.clone();
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
            .on_click(cx.listener(move |this, _, _, cx| {
                this.toggle_git_folder_collapsed(folder_id, cx);
            }))
            .on_drag(anchor, move |_, _, _, cx| {
                cx.new(|_| GitDragPreview::new(label.clone(), UiIcon::FOLDER.into()))
            })
            .context_menu(move |menu, _window, _cx| {
                render_git_folder_menu(menu, app_entity.clone(), folder_id)
            });

        let into_bg = p.drop_target;
        let mut band = sidebar_group_row_band(
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
        );
        band = band.child(reorder_overlay(&overlay_app, anchor, true, p));
        element.child(band).into_any_element()
    }

    /// 渲染一个项目行：活动底色、拖拽源与上下半区落点。
    ///
    /// `folders` 是渲染期取好的分组列表，直接带进右键菜单——菜单闭包运行时再回查
    /// `self.state.git` 的借用不成立。
    pub(crate) fn render_git_project_row(
        &mut self,
        row: &GitProjectRow,
        row_state: SidebarRowState,
        folders: &[(GitFolderId, String)],
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let project = &row.project;
        let id = project.id;
        let badge = self.state.git.badges.get(&id).cloned().unwrap_or_default();
        let name = project.display_name();
        let active_branch = self
            .state
            .git_workspace_of_project(id, cx)
            .and_then(|workspace| {
                let state = &workspace.read(cx).state;
                state.status.as_ref().and_then(|status| match &status.head {
                    transom_core::HeadState::OnBranch(name) => Some(name.clone()),
                    transom_core::HeadState::Initial { branch } => Some(branch.clone()),
                    transom_core::HeadState::Detached => status
                        .oid
                        .as_deref()
                        .map(|oid| oid.chars().take(7).collect()),
                })
            });

        let branch = badge.branch.clone().or(active_branch).unwrap_or_else(|| {
            if badge.error.is_some() {
                rust_i18n::t!("git.sidebar.unavailable").to_string()
            } else if badge.loading {
                rust_i18n::t!("git.sidebar.loading").to_string()
            } else {
                "-".to_string()
            }
        });
        let app_entity = cx.entity().clone();
        let menu_app = app_entity.clone();
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
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                if event.click_count() >= 2 {
                    this.open_git_tab(project_for_click.clone(), window, cx);
                } else {
                    this.select_git_project(id, cx);
                }
            }))
            .on_drag(GitSidebarDrag::Project(id), move |_, _, _, cx| {
                cx.new(|_| GitDragPreview::new(drag_label.clone(), icon_name))
            })
            .context_menu(move |menu, _window, _cx| {
                render_git_project_menu(menu, menu_app.clone(), id, menu_folders.clone())
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
                        .unwrap_or_else(|| state_icon_color(&badge, row_state, p)),
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
            // 计数只用弱色：语义色留给行首图标，两处同时上色会把一行变成告警条。
            band = band.child(
                div()
                    .text_xs()
                    .text_color(p.text.muted)
                    .child(badge.dirty.to_string()),
            );
        }
        let band = band.child(reorder_overlay(
            &app_entity,
            GitSidebarDrag::Project(id),
            false,
            p,
        ));
        element.child(band).into_any_element()
    }
}

/// 行首图标的状态色：读不到仓库标红，刷新中标灰，有改动标警示，活动行跟随激活前景色。
///
/// 与会话行同口径——状态靠图标说，脏文件数靠行尾弱色数字说，两处不抢同一个颜色。
/// 用户给项目贴了环境色时这段不生效：环境标记优先于状态提示（与会话行一致）。
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

/// 覆盖整行的排序落点：把行按份数等分，用户不需要瞄准行边缘。
///
/// 项目行整行二分（上半插到之前、下半插到之后）；分组行三分并留出中段，
/// 中段不注册任何监听，因此不产生命中框，拖放会继续落到分组行本身，
/// 即「放入该分组」。等分用 `flex_1` 而非百分比宽度，gpui 没有宽度百分比。
fn reorder_overlay(
    app: &Entity<AppShell>,
    anchor: GitSidebarDrag,
    keep_middle: bool,
    p: &Palette,
) -> AnyElement {
    let overlay = div()
        .absolute()
        .inset_0()
        .flex()
        .flex_col()
        .child(reorder_half(app, anchor, DropSide::Before, true, p));
    let overlay = if keep_middle {
        overlay.child(div().flex_1())
    } else {
        overlay
    };
    overlay
        .child(reorder_half(app, anchor, DropSide::After, false, p))
        .into_any_element()
}

/// 半区落点本身：`top` 决定插入线画在上沿还是下沿。
///
/// 插入线用 `sidebar_accent_foreground`：accent 已经是行底色，同色描边看不出。
fn reorder_half(
    app: &Entity<AppShell>,
    anchor: GitSidebarDrag,
    side: DropSide,
    top: bool,
    p: &Palette,
) -> AnyElement {
    let target = GitDropTarget::Relative { anchor, side };
    // `drag_over` 的闭包要求 `'static`，颜色先取成局部值再捕获。
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
    let app = app.clone();
    zone.on_drop(move |drag: &GitSidebarDrag, window, cx| {
        let drag = *drag;
        app.update(cx, |this, cx| {
            this.apply_git_drop(drag, target, cx);
            window.refresh();
        });
    })
    .into_any_element()
}
