//! API 接口树拖拽排序与文件夹归属调整。

use super::ApiWorkspace;
use crate::palette::Palette;
use gpui_kit::base::h_flex;
use gpui_kit::*;
use transom_api::{ApiFolderId, ApiProject, ApiRequestId};
use transom_core::DropSide;
use transom_ui_icons::{AppIcon, AppIconElement};

/// 接口树中正在被拖拽的条目标识。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ApiTreeDrag {
    Request(ApiRequestId),
    Folder(ApiFolderId),
}

/// 接口树中的拖放目标位置。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ApiTreeDropTarget {
    Folder(ApiFolderId),
    Root,
    Relative { anchor: ApiTreeDrag, side: DropSide },
}

/// 拖动时跟随鼠标的预览卡片；与会话/Git 侧栏同款。
pub(super) struct ApiDragPreview {
    label: String,
    icon: AppIcon,
}

impl ApiDragPreview {
    pub(super) fn new(label: String, icon: AppIcon) -> Self {
        Self { label, icon }
    }
}

impl Render for ApiDragPreview {
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

/// 为接口树条目叠加的上下半区/三分之一插入线落点层。
pub(super) fn reorder_overlay(
    view: &Entity<ApiWorkspace>,
    anchor: ApiTreeDrag,
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
    view: &Entity<ApiWorkspace>,
    anchor: ApiTreeDrag,
    side: DropSide,
    top: bool,
    p: &Palette,
) -> AnyElement {
    let target = ApiTreeDropTarget::Relative { anchor, side };
    let line_color = p.sidebar_accent_foreground;
    let zone = div().flex_1().border_color(line_color);
    let zone = if top {
        zone.drag_over::<ApiTreeDrag>(move |style, _, _, _| {
            style.border_t_2().border_color(line_color)
        })
    } else {
        zone.drag_over::<ApiTreeDrag>(move |style, _, _, _| {
            style.border_b_2().border_color(line_color)
        })
    };
    let view = view.clone();
    zone.on_drop(move |drag: &ApiTreeDrag, window, cx| {
        let drag = *drag;
        view.update(cx, |this, cx| this.handle_tree_drop(drag, target, cx));
        window.refresh();
    })
    .into_any_element()
}

impl ApiWorkspace {
    /// 处理接口树拖拽落点：支持请求排序、跨文件夹移动、移回根目录与文件夹排序。
    pub(super) fn handle_tree_drop(
        &mut self,
        drag: ApiTreeDrag,
        target: ApiTreeDropTarget,
        cx: &mut Context<Self>,
    ) {
        if !apply_tree_drop(&mut self.project, drag, target) {
            return;
        }
        if let ApiTreeDrag::Request(_) = drag {
            match target {
                ApiTreeDropTarget::Folder(folder_id)
                | ApiTreeDropTarget::Relative {
                    anchor: ApiTreeDrag::Folder(folder_id),
                    ..
                } => {
                    self.ui.collapsed_folders.remove(&folder_id);
                }
                _ => {}
            }
        }
        self.save_project(cx);
        cx.notify();
    }
}

fn apply_tree_drop(project: &mut ApiProject, drag: ApiTreeDrag, target: ApiTreeDropTarget) -> bool {
    match (drag, target) {
        (ApiTreeDrag::Request(request_id), ApiTreeDropTarget::Folder(folder_id))
        | (
            ApiTreeDrag::Request(request_id),
            ApiTreeDropTarget::Relative {
                anchor: ApiTreeDrag::Folder(folder_id),
                ..
            },
        ) => move_request_to_folder(project, request_id, Some(folder_id)),
        (ApiTreeDrag::Request(request_id), ApiTreeDropTarget::Root) => {
            move_request_to_folder(project, request_id, None)
        }
        (
            ApiTreeDrag::Request(request_id),
            ApiTreeDropTarget::Relative {
                anchor: ApiTreeDrag::Request(anchor_id),
                side,
            },
        ) => reorder_request(project, request_id, anchor_id, side),
        (
            ApiTreeDrag::Folder(folder_id),
            ApiTreeDropTarget::Relative {
                anchor: ApiTreeDrag::Folder(anchor_id),
                side,
            },
        ) => reorder_folder(project, folder_id, anchor_id, side),
        _ => false,
    }
}

fn move_request_to_folder(
    project: &mut ApiProject,
    request_id: ApiRequestId,
    folder_id: Option<ApiFolderId>,
) -> bool {
    let Some(request) = project
        .requests
        .iter_mut()
        .find(|request| request.id == request_id)
    else {
        return false;
    };
    if request.folder_id == folder_id {
        return false;
    }
    request.folder_id = folder_id;
    project.touch();
    true
}

fn reorder_request(
    project: &mut ApiProject,
    request_id: ApiRequestId,
    anchor_id: ApiRequestId,
    side: DropSide,
) -> bool {
    if request_id == anchor_id {
        return false;
    }
    let Some(from_index) = project
        .requests
        .iter()
        .position(|request| request.id == request_id)
    else {
        return false;
    };
    let Some(anchor_folder) = project
        .requests
        .iter()
        .find(|request| request.id == anchor_id)
        .map(|request| request.folder_id)
    else {
        return false;
    };
    let mut item = project.requests.remove(from_index);
    item.folder_id = anchor_folder;
    let Some(anchor_index) = project
        .requests
        .iter()
        .position(|request| request.id == anchor_id)
    else {
        project.requests.insert(from_index, item);
        return false;
    };
    let insert_at = match side {
        DropSide::Before => anchor_index,
        DropSide::After => anchor_index + 1,
    };
    project.requests.insert(insert_at, item);
    project.touch();
    true
}

fn reorder_folder(
    project: &mut ApiProject,
    folder_id: ApiFolderId,
    anchor_id: ApiFolderId,
    side: DropSide,
) -> bool {
    if folder_id == anchor_id {
        return false;
    }
    let Some(from_index) = project
        .folders
        .iter()
        .position(|folder| folder.id == folder_id)
    else {
        return false;
    };
    let item = project.folders.remove(from_index);
    let Some(anchor_index) = project
        .folders
        .iter()
        .position(|folder| folder.id == anchor_id)
    else {
        project.folders.insert(from_index, item);
        return false;
    };
    let insert_at = match side {
        DropSide::Before => anchor_index,
        DropSide::After => anchor_index + 1,
    };
    project.folders.insert(insert_at, item);
    project.touch();
    true
}

#[cfg(test)]
mod tests {
    use super::{ApiTreeDrag, ApiTreeDropTarget, apply_tree_drop};
    use transom_api::{ApiProject, ApiRequest};
    use transom_core::DropSide;

    #[test]
    fn drag_request_reorders_and_inherits_target_folder() {
        let mut project = ApiProject::new("拖拽测试");
        let folder = project.add_folder("目录A").id;
        let r1 = ApiRequest::new("R1", None);
        let r2 = ApiRequest::new("R2", Some(folder));
        let r1_id = r1.id;
        let r2_id = r2.id;
        project.upsert_request(r1);
        project.upsert_request(r2);

        assert!(apply_tree_drop(
            &mut project,
            ApiTreeDrag::Request(r1_id),
            ApiTreeDropTarget::Relative {
                anchor: ApiTreeDrag::Request(r2_id),
                side: DropSide::After,
            },
        ));
        assert_eq!(project.requests[0].id, r2_id);
        assert_eq!(project.requests[1].id, r1_id);
        assert_eq!(project.requests[1].folder_id, Some(folder));
    }

    #[test]
    fn drag_request_to_root_and_folder() {
        let mut project = ApiProject::new("目录移动");
        let folder = project.add_folder("目录A").id;
        let r1 = ApiRequest::new("R1", None);
        let r1_id = r1.id;
        project.upsert_request(r1);

        assert!(apply_tree_drop(
            &mut project,
            ApiTreeDrag::Request(r1_id),
            ApiTreeDropTarget::Folder(folder),
        ));
        assert_eq!(
            project.request(r1_id).and_then(|r| r.folder_id),
            Some(folder)
        );

        assert!(apply_tree_drop(
            &mut project,
            ApiTreeDrag::Request(r1_id),
            ApiTreeDropTarget::Root,
        ));
        assert_eq!(project.request(r1_id).and_then(|r| r.folder_id), None);
    }

    #[test]
    fn drag_folder_reorders_folders() {
        let mut project = ApiProject::new("目录排序");
        let f1 = project.add_folder("F1").id;
        let f2 = project.add_folder("F2").id;

        assert!(apply_tree_drop(
            &mut project,
            ApiTreeDrag::Folder(f2),
            ApiTreeDropTarget::Relative {
                anchor: ApiTreeDrag::Folder(f1),
                side: DropSide::Before,
            },
        ));
        assert_eq!(project.folders[0].id, f2);
        assert_eq!(project.folders[1].id, f1);
    }
}
