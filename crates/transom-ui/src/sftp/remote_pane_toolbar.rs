//! 远端列第二行操作工具栏：主远端与并列远端共用同一套按钮与目标路由。

use super::remote_pane::{RemoteToolbarProps, pane_key};
use crate::app::AppShell;
use crate::editor::render_editor_list_popover;
use crate::palette::Palette;
use crate::sftp::SftpRemotePaneRef;
use gpui_kit::assets::IconName;
use gpui_kit::base::h_flex;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::separator::Separator;
use gpui_kit::component::{Selectable, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

impl AppShell {
    /// 渲染远端列的操作工具栏（主目录、下载与文件管理入口）。
    pub(super) fn render_remote_toolbar(
        &self,
        props: RemoteToolbarProps<'_>,
        p: &Palette,
        cx: &mut Context<AppShell>,
    ) -> AnyElement {
        let RemoteToolbarProps {
            tab_id,
            target,
            pane,
            show_filter,
            show_hidden,
            sync_navigation,
            show_editor_list,
        } = props;
        let key = pane_key(target);

        let home_btn = Button::new(format!("remote-home-{key}"))
            .ghost()
            .xsmall()
            .icon(IconName::House)
            .tooltip(rust_i18n::t!("sftp.home").to_string())
            .on_click(cx.listener(move |this, _, window, cx| {
                this.sftp_go_home_pane(tab_id, target, window, cx);
            }));

        let download_btn = Button::new(format!("remote-download-{key}"))
            .primary()
            .xsmall()
            .icon(IconName::Download)
            .tooltip(rust_i18n::t!("sftp.download").to_string())
            .on_click(cx.listener(move |this, _, _, cx| {
                this.sftp_queue_download_pane(tab_id, target, cx);
            }));

        let mkdir_btn = Button::new(format!("remote-mkdir-{key}"))
            .ghost()
            .xsmall()
            .icon(IconName::FolderPlus)
            .tooltip(rust_i18n::t!("sftp.new_folder").to_string())
            .on_click(cx.listener(move |this, _, window, cx| {
                this.sftp_open_mkdir_modal_for(tab_id, target, window, cx);
            }));

        let new_file_btn = Button::new(format!("remote-new-file-{key}"))
            .ghost()
            .xsmall()
            .icon(IconName::FilePlus)
            .tooltip(rust_i18n::t!("sftp.remote.new_file_edit").to_string())
            .on_click(cx.listener(move |this, _, window, cx| {
                this.sftp_open_editor_new_file_modal_for(tab_id, target, window, cx);
            }));

        let search_btn = Button::new(format!("remote-search-{key}"))
            .ghost()
            .xsmall()
            .icon(IconName::Search)
            .tooltip(rust_i18n::t!("sftp.remote.search").to_string())
            .on_click(cx.listener(move |this, _, window, cx| {
                this.sftp_open_search_modal_for(tab_id, target, window, cx);
            }));

        let rename_btn = Button::new(format!("remote-rename-{key}"))
            .ghost()
            .xsmall()
            .icon(IconName::Pencil)
            .tooltip(rust_i18n::t!("sftp.rename").to_string())
            .on_click(cx.listener(move |this, _, window, cx| {
                this.sftp_open_rename_modal_for(tab_id, target, window, cx);
            }));

        let selected = pane.selected.clone();
        let edit_btn = Button::new(format!("remote-edit-{key}"))
            .ghost()
            .xsmall()
            .icon(IconName::FilePen)
            .tooltip(rust_i18n::t!("common.edit").to_string())
            .on_click(cx.listener(move |this, _, window, cx| {
                let Some(path) = selected.clone() else {
                    this.set_sftp_pane_error(
                        tab_id,
                        target,
                        rust_i18n::t!("sftp.select_file").to_string(),
                        cx,
                    );
                    return;
                };
                this.sftp_edit_remote_file_pane(tab_id, target, path, window, cx);
            }));

        let chmod_btn = Button::new(format!("remote-chmod-{key}"))
            .ghost()
            .xsmall()
            .icon(IconName::Settings)
            .tooltip(rust_i18n::t!("sftp.remote.chmod").to_string())
            .on_click(cx.listener(move |this, _, window, cx| {
                this.sftp_open_chmod_modal_for(tab_id, target, window, cx);
            }));

        let delete_btn = Button::new(format!("remote-delete-{key}"))
            .ghost()
            .xsmall()
            .icon(IconName::Trash)
            .tooltip(rust_i18n::t!("sftp.delete").to_string())
            .on_click(cx.listener(move |this, _, _, cx| {
                this.sftp_delete_selection_for(tab_id, target, cx);
            }));

        let upload_btn = Button::new(format!("remote-upload-{key}"))
            .ghost()
            .xsmall()
            .icon(IconName::Upload)
            .tooltip(rust_i18n::t!("sftp.upload").to_string())
            .on_click(cx.listener(move |this, _, _, cx| {
                this.sftp_upload_local_selection_pane(tab_id, target, cx);
            }));

        let hidden_btn = Button::new(format!("remote-toggle-hidden-{key}"))
            .ghost()
            .xsmall()
            .icon(if show_hidden {
                IconName::Eye
            } else {
                IconName::EyeOff
            })
            .selected(show_hidden)
            .tooltip(if show_hidden {
                rust_i18n::t!("sftp.remote.hide_dotfiles").to_string()
            } else {
                rust_i18n::t!("sftp.remote.show_dotfiles").to_string()
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.sftp_toggle_hidden_pane(tab_id, target, cx);
            }));

        let sync_btn = Button::new(format!("remote-sync-nav-{key}"))
            .ghost()
            .xsmall()
            .icon(IconName::ArrowLeftRight)
            .selected(sync_navigation)
            .tooltip(if sync_navigation {
                rust_i18n::t!("sftp.remote.sync_nav_on").to_string()
            } else {
                rust_i18n::t!("sftp.remote.sync_nav_off").to_string()
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.sftp_toggle_sync_navigation(tab_id, cx);
            }));
        // 同步导航属于"主远端 ↔ 本地"这一对列的关系；并列列不显示该按钮，
        // 避免一列切换时所有列的按钮同时变亮。
        let is_main = target == SftpRemotePaneRef::Main;

        let compare_btn = Button::new(format!("remote-compare-{key}"))
            .ghost()
            .xsmall()
            .icon(IconName::GitCompareArrows)
            .tooltip(rust_i18n::t!("sftp.sync.title").to_string())
            .on_click(cx.listener(move |this, _, _, cx| {
                this.sftp_open_sync_modal_pane(tab_id, target, cx);
            }));

        let filter_btn = Button::new(format!("remote-filter-{key}"))
            .ghost()
            .xsmall()
            .icon(IconName::Search)
            .selected(show_filter)
            .tooltip(rust_i18n::t!("sftp.remote.filter").to_string())
            .on_click(cx.listener(move |this, _, window, cx| {
                this.sftp_toggle_filter_pane(tab_id, target, window, cx);
            }));

        // 主远端与并列列使用同一组按钮：列宽度不足时由横向裁切吸收，
        // 不再为某类列偷偷删减入口。
        h_flex()
            .gap_1()
            .items_center()
            .child(home_btn)
            .child(
                Separator::vertical()
                    .h(px(14.0))
                    .mx_1()
                    .color(p.sidebar_border),
            )
            .child(download_btn)
            .child(mkdir_btn)
            .child(new_file_btn)
            .child(search_btn)
            .child(rename_btn)
            .child(edit_btn)
            .when(show_editor_list, |this| {
                this.child(render_editor_list_popover(self, tab_id, p, cx))
            })
            .child(chmod_btn)
            .child(delete_btn)
            .child(upload_btn)
            .child(hidden_btn)
            .when(is_main, |this| this.child(sync_btn))
            .child(compare_btn)
            .child(filter_btn)
            .into_any_element()
    }
}
