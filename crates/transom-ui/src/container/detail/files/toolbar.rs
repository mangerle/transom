//! 文件页面包屑与工具按钮。

use super::super::*;

pub(super) fn render_toolbar(
    tab_id: TabId,
    current_path: &str,
    readonly: bool,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let parent = super::parent_path(current_path);
    let mut crumbs = h_flex().gap_1().items_center().child(
        Button::new("container-file-up")
            .ghost()
            .xsmall()
            .icon(IconName::ArrowLeft)
            .on_click(cx.listener({
                let parent = parent.clone();
                move |this, _, _, cx| {
                    this.load_container_dir(tab_id, parent.clone());
                    cx.notify();
                }
            })),
    );
    crumbs = crumbs.child(
        div()
            .text_sm()
            .text_color(p.muted_foreground)
            .child(current_path.to_string()),
    );
    crumbs = crumbs.child(div().flex_1()).child(
        Button::new("container-file-refresh")
            .ghost()
            .xsmall()
            .icon(IconName::RefreshCw)
            .on_click(cx.listener({
                let path = current_path.to_string();
                move |this, _, _, cx| {
                    this.load_container_dir(tab_id, path.clone());
                    cx.notify();
                }
            })),
    );
    if !readonly {
        crumbs = crumbs.child(
            Button::new("container-file-upload")
                .ghost()
                .xsmall()
                .icon(IconName::Upload)
                .label(rust_i18n::t!("container.files.upload").to_string())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.upload_container_file(tab_id);
                    cx.notify();
                })),
        );
        crumbs = crumbs.child(
            Button::new("container-file-upload-dir")
                .ghost()
                .xsmall()
                .icon(IconName::Upload)
                .label(rust_i18n::t!("container.files.upload_directory").to_string())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.upload_container_directory(tab_id);
                    cx.notify();
                })),
        );
        crumbs = crumbs.child(
            Button::new("container-file-mkdir")
                .ghost()
                .xsmall()
                .icon(IconName::FolderPlus)
                .label(rust_i18n::t!("container.files.new_dir").to_string())
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.toggle_container_mkdir(tab_id, window, cx);
                })),
        );
    }
    crumbs.into_any_element()
}
