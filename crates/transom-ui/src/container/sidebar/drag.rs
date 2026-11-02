//! 引擎侧栏拖拽：落点判定、移动与排序。

use super::*;

impl AppShell {
    /// 处理引擎树拖放：换分组或同级排序。
    pub(crate) fn handle_container_sidebar_drop(
        &mut self,
        drag: ContainerSidebarDrag,
        target: ContainerSidebarTarget,
        cx: &mut Context<Self>,
    ) {
        log::info!("引擎侧栏拖放，载荷: {:?}，落点: {:?}", drag, target);
        match self.apply_container_sidebar_drop(drag, target) {
            Ok(landed) => {
                // 目标分组被折叠时自动展开，保证拖进去的条目可见。
                if let Some(folder_id) = landed
                    && let Some(folder) = self
                        .state
                        .catalog
                        .engine_folders
                        .iter_mut()
                        .find(|folder| folder.id == folder_id)
                {
                    folder.collapsed = false;
                }
                self.state.persist_catalog();
            }
            Err(error) => self.state.set_error(error.to_string()),
        }
        cx.notify();
    }

    pub(crate) fn apply_container_sidebar_drop(
        &mut self,
        drag: ContainerSidebarDrag,
        target: ContainerSidebarTarget,
    ) -> Result<Option<FolderId>, CatalogError> {
        match (drag, target) {
            (ContainerSidebarDrag::Engine(profile_id), ContainerSidebarTarget::Root) => {
                self.state.catalog.move_engine(profile_id, None)?;
                Ok(None)
            }
            (ContainerSidebarDrag::Folder(folder_id), ContainerSidebarTarget::Root) => {
                self.state.catalog.move_engine_folder(folder_id, None)?;
                Ok(None)
            }
            (
                ContainerSidebarDrag::Engine(profile_id),
                ContainerSidebarTarget::Folder(folder_id),
            ) => {
                self.state
                    .catalog
                    .move_engine(profile_id, Some(folder_id))?;
                Ok(Some(folder_id))
            }
            (ContainerSidebarDrag::Folder(moved), ContainerSidebarTarget::Folder(parent)) => {
                if moved == parent {
                    return Ok(Some(parent));
                }
                self.state.catalog.move_engine_folder(moved, Some(parent))?;
                Ok(Some(parent))
            }
            (drag, ContainerSidebarTarget::Relative { anchor, side }) => match (drag, anchor) {
                (
                    ContainerSidebarDrag::Engine(profile_id),
                    ContainerSidebarDrag::Engine(anchor_id),
                ) => {
                    self.state
                        .catalog
                        .reorder_engine(profile_id, anchor_id, side)?;
                    Ok(self
                        .state
                        .catalog
                        .engines
                        .iter()
                        .find(|profile| profile.id == anchor_id)
                        .and_then(|profile| profile.folder_id))
                }
                (
                    ContainerSidebarDrag::Folder(folder_id),
                    ContainerSidebarDrag::Folder(anchor_id),
                ) => {
                    self.state
                        .catalog
                        .reorder_engine_folder(folder_id, anchor_id, side)?;
                    Ok(self
                        .state
                        .catalog
                        .engine_folder(folder_id)
                        .and_then(|folder| folder.parent))
                }
                (
                    ContainerSidebarDrag::Engine(profile_id),
                    ContainerSidebarDrag::Folder(anchor_id),
                ) => {
                    self.state
                        .catalog
                        .move_engine(profile_id, Some(anchor_id))?;
                    Ok(Some(anchor_id))
                }
                (
                    ContainerSidebarDrag::Folder(folder_id),
                    ContainerSidebarDrag::Engine(anchor_id),
                ) => {
                    let parent = self
                        .state
                        .catalog
                        .engines
                        .iter()
                        .find(|profile| profile.id == anchor_id)
                        .and_then(|profile| profile.folder_id);
                    self.state.catalog.move_engine_folder(folder_id, parent)?;
                    Ok(parent)
                }
            },
        }
    }
}

/// 引擎行/分组行的排序落点遮罩：会话行二分，分组行保留中段作为“放入该分组”。
pub(crate) fn engine_reorder_overlay(
    app: &Entity<AppShell>,
    anchor: ContainerSidebarDrag,
    keep_middle: bool,
    p: &Palette,
) -> AnyElement {
    let overlay = div()
        .absolute()
        .inset_0()
        .flex()
        .flex_col()
        .child(engine_reorder_half(app, anchor, DropSide::Before, true, p));
    let overlay = if keep_middle {
        overlay.child(div().flex_1())
    } else {
        overlay
    };
    overlay
        .child(engine_reorder_half(app, anchor, DropSide::After, false, p))
        .into_any_element()
}

pub(crate) fn engine_reorder_half(
    app: &Entity<AppShell>,
    anchor: ContainerSidebarDrag,
    side: DropSide,
    top: bool,
    p: &Palette,
) -> AnyElement {
    let target = ContainerSidebarTarget::Relative { anchor, side };
    let line_color = p.sidebar_accent_foreground;
    let zone = div().flex_1().border_color(line_color);
    let zone = if top {
        zone.drag_over::<ContainerSidebarDrag>(move |style, _, _, _| {
            style.border_t_2().border_color(line_color)
        })
    } else {
        zone.drag_over::<ContainerSidebarDrag>(move |style, _, _, _| {
            style.border_b_2().border_color(line_color)
        })
    };
    let app = app.clone();
    zone.on_drop(move |drag: &ContainerSidebarDrag, window, cx| {
        let drag = *drag;
        app.update(cx, |this, cx| {
            this.handle_container_sidebar_drop(drag, target, cx);
            window.refresh();
        });
    })
    .into_any_element()
}
