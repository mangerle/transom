//! 文件页面包屑与工具按钮。

use super::super::*;
use crate::container::ContainerWorkspace;

pub(super) fn render_toolbar(
    view: &Entity<ContainerWorkspace>,
    current_path: &str,
    readonly: bool,
    p: &Palette,
) -> AnyElement {
    let parent = super::parent_path(current_path);
    let up_view = view.clone();
    let mut crumbs = h_flex().gap_1().items_center().child(
        Button::new("container-file-up")
            .ghost()
            .xsmall()
            .icon(IconName::ArrowLeft)
            .accessibility_label("Open parent folder")
            .on_click(move |_, _, cx| {
                up_view.update(cx, |workspace, cx| {
                    workspace.load_container_dir(parent.clone(), cx);
                });
            }),
    );
    crumbs = crumbs.child(
        div()
            .text_sm()
            .text_color(p.muted_foreground)
            .child(current_path.to_string()),
    );
    let refresh_view = view.clone();
    let refresh_path = current_path.to_string();
    crumbs = crumbs.child(div().flex_1()).child(
        Button::new("container-file-refresh")
            .ghost()
            .xsmall()
            .icon(IconName::RefreshCw)
            .accessibility_label("Refresh container files")
            .on_click(move |_, _, cx| {
                refresh_view.update(cx, |workspace, cx| {
                    workspace.load_container_dir(refresh_path.clone(), cx);
                });
            }),
    );
    if !readonly {
        let upload_view = view.clone();
        crumbs = crumbs.child(
            Button::new("container-file-upload")
                .ghost()
                .xsmall()
                .icon(IconName::Upload)
                .label(rust_i18n::t!("container.files.upload").to_string())
                .on_click(move |_, _, cx| {
                    upload_view.update(cx, |workspace, cx| {
                        workspace.upload_container_file(cx);
                    });
                }),
        );
        let upload_dir_view = view.clone();
        crumbs = crumbs.child(
            Button::new("container-file-upload-dir")
                .ghost()
                .xsmall()
                .icon(IconName::Upload)
                .label(rust_i18n::t!("container.files.upload_directory").to_string())
                .on_click(move |_, _, cx| {
                    upload_dir_view.update(cx, |workspace, cx| {
                        workspace.upload_container_directory(cx);
                    });
                }),
        );
        let mkdir_view = view.clone();
        crumbs = crumbs.child(
            Button::new("container-file-mkdir")
                .ghost()
                .xsmall()
                .icon(IconName::FolderPlus)
                .label(rust_i18n::t!("container.files.new_dir").to_string())
                .on_click(move |_, window, cx| {
                    mkdir_view.update(cx, |workspace, cx| {
                        workspace.toggle_container_mkdir(window, cx);
                    });
                }),
        );
    }
    crumbs.into_any_element()
}
