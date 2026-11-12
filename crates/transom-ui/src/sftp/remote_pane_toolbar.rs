//! 远端列第二行操作工具栏：主远端与并列远端共用同一套按钮与目标路由。

use super::remote_pane::{RemoteToolbarProps, pane_key};
use crate::editor::render_editor_list_popover;
use crate::palette::Palette;
use crate::sftp::SftpRemotePaneRef;
use crate::sftp::entity::SftpPanelView;
use crate::widgets::{IconButton, IconButtonSize};
use gpui_kit::base::h_flex;
use gpui_kit::component::separator::Separator;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_ui_icons::AppIcon;
use transom_ui_icons::CustomIcon;
use transom_ui_icons::DomainIcon;
use transom_ui_icons::UiIcon;

impl SftpPanelView {
    /// 渲染远端列的操作工具栏（主目录、下载与文件管理入口）。
    pub(super) fn render_remote_toolbar(
        &self,
        props: RemoteToolbarProps<'_>,
        p: &Palette,
        cx: &mut Context<SftpPanelView>,
    ) -> AnyElement {
        let key = pane_key(props.target);
        self.render_toolbar_buttons(&props, &key, p, cx)
    }

    /// 工具栏按钮组：文件操作、编辑、权限、上传、显隐、同步导航与过滤。
    fn render_toolbar_buttons(
        &self,
        props: &RemoteToolbarProps<'_>,
        key: &str,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let RemoteToolbarProps { tab_id, target, .. } = *props;
        let home_btn = IconButton::new(
            format!("remote-home-{key}"),
            AppIcon::Ui(UiIcon::HOME_FOLDER),
            rust_i18n::t!("sftp.home").to_string(),
        )
        .size(IconButtonSize::Regular)
        .render()
        .on_click(cx.listener(move |this, _, window, cx| {
            this.go_home_pane(tab_id, target, window, cx);
        }));

        let file_ops = self.render_file_op_buttons(props, key, p, cx);
        let view_ops = self.render_view_op_buttons(props, key, cx);
        crate::widgets::toolbar()
            .child(home_btn)
            .child(
                Separator::vertical()
                    .h(px(14.0))
                    .mx_1()
                    .color(p.sidebar_border),
            )
            .child(file_ops)
            .child(view_ops)
            .into_any_element()
    }

    /// 文件操作组：下载与新建目录/文件按钮。
    fn render_browse_buttons(
        &self,
        props: &RemoteToolbarProps<'_>,
        key: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let RemoteToolbarProps { target, .. } = *props;
        let download_btn = IconButton::new(
            format!("remote-download-{key}"),
            AppIcon::Ui(UiIcon::DOWNLOAD),
            rust_i18n::t!("sftp.download").to_string(),
        )
        .size(IconButtonSize::Regular)
        .render()
        .on_click(cx.listener(move |this, _, _, cx| {
            this.download_pane(target, cx);
        }));

        let mkdir_btn = IconButton::new(
            format!("remote-mkdir-{key}"),
            AppIcon::Ui(UiIcon::NEW_FOLDER),
            rust_i18n::t!("sftp.new_folder").to_string(),
        )
        .size(IconButtonSize::Regular)
        .render()
        .on_click(cx.listener(move |this, _, window, cx| {
            this.open_mkdir_modal_for(target, window, cx);
        }));

        let new_file_btn = IconButton::new(
            format!("remote-new-file-{key}"),
            AppIcon::Ui(UiIcon::ADD_FILE),
            rust_i18n::t!("sftp.remote.new_file_edit").to_string(),
        )
        .size(IconButtonSize::Regular)
        .render()
        .on_click(cx.listener(move |this, _, window, cx| {
            this.open_editor_new_file_modal_for(target, window, cx);
        }));

        h_flex()
            .gap_1()
            .items_center()
            .child(download_btn)
            .child(mkdir_btn)
            .child(new_file_btn)
            .into_any_element()
    }

    /// 重命名、编辑、编辑列表、权限、删除与上传按钮。
    fn render_file_edit_buttons(
        &self,
        props: &RemoteToolbarProps<'_>,
        key: &str,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let RemoteToolbarProps {
            tab_id,
            target,
            pane,
            show_editor_list,
            ..
        } = *props;
        let rename_btn = IconButton::new(
            format!("remote-rename-{key}"),
            AppIcon::Ui(UiIcon::EDIT),
            rust_i18n::t!("sftp.rename").to_string(),
        )
        .size(IconButtonSize::Regular)
        .render()
        .on_click(cx.listener(move |this, _, window, cx| {
            this.open_rename_modal_for(target, window, cx);
        }));

        let selected = pane.selected.clone();
        let edit_btn = IconButton::new(
            format!("remote-edit-{key}"),
            AppIcon::Ui(UiIcon::REAL_INTENTION_BULB),
            rust_i18n::t!("common.edit").to_string(),
        )
        .size(IconButtonSize::Regular)
        .render()
        .on_click(cx.listener(move |this, _, window, cx| {
            let Some(path) = selected.clone() else {
                this.set_pane_error(target, rust_i18n::t!("sftp.select_file").to_string(), cx);
                return;
            };
            this.edit_remote_file(target, path, window, cx);
        }));

        let chmod_btn = IconButton::new(
            format!("remote-chmod-{key}"),
            AppIcon::Ui(UiIcon::SETTINGS),
            rust_i18n::t!("sftp.remote.chmod").to_string(),
        )
        .size(IconButtonSize::Regular)
        .render()
        .on_click(cx.listener(move |this, _, window, cx| {
            this.open_chmod_modal(target, window, cx);
        }));

        let delete_btn = IconButton::new(
            format!("remote-delete-{key}"),
            AppIcon::Ui(UiIcon::DELETE),
            rust_i18n::t!("sftp.delete").to_string(),
        )
        .size(IconButtonSize::Regular)
        .render()
        .on_click(cx.listener(move |this, _, _, cx| {
            this.confirm_remote_delete(target, cx);
        }));

        let upload_btn = IconButton::new(
            format!("remote-upload-{key}"),
            AppIcon::Ui(UiIcon::UPLOAD),
            rust_i18n::t!("sftp.upload").to_string(),
        )
        .size(IconButtonSize::Regular)
        .render()
        .on_click(cx.listener(move |this, _, _, cx| {
            this.upload_local_selection(target, cx);
        }));

        let editor_list = show_editor_list.then(|| {
            let Some(editors) = self.state.editors.clone() else {
                return div().into_any_element();
            };
            render_editor_list_popover(editors.read(cx), &editors, tab_id, p)
        });

        h_flex()
            .gap_1()
            .items_center()
            .child(rename_btn)
            .child(edit_btn)
            .when_some(editor_list, |this, el| this.child(el))
            .child(chmod_btn)
            .child(delete_btn)
            .child(upload_btn)
            .into_any_element()
    }

    fn render_file_op_buttons(
        &self,
        props: &RemoteToolbarProps<'_>,
        key: &str,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let browse = self.render_browse_buttons(props, key, cx);
        let edit = self.render_file_edit_buttons(props, key, p, cx);
        h_flex()
            .gap_1()
            .items_center()
            .child(browse)
            .child(edit)
            .into_any_element()
    }

    /// 视图选项组：显隐、同步导航、目录对比与统一搜索入口。
    fn render_view_op_buttons(
        &self,
        props: &RemoteToolbarProps<'_>,
        key: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let RemoteToolbarProps {
            target,
            show_filter,
            show_hidden,
            sync_navigation,
            ..
        } = *props;
        let hidden_btn = IconButton::new(
            format!("remote-toggle-hidden-{key}"),
            if show_hidden {
                AppIcon::Ui(UiIcon::SHOW)
            } else {
                AppIcon::Custom(CustomIcon::EYE_OFF)
            },
            if show_hidden {
                rust_i18n::t!("sftp.remote.hide_dotfiles").to_string()
            } else {
                rust_i18n::t!("sftp.remote.show_dotfiles").to_string()
            },
        )
        .selected(show_hidden)
        .size(IconButtonSize::Regular)
        .render()
        .on_click(cx.listener(move |this, _, _, cx| {
            this.toggle_hidden_pane(target, cx);
        }));

        let sync_btn = IconButton::new(
            format!("remote-sync-nav-{key}"),
            AppIcon::Domain(DomainIcon::FORWARD),
            if sync_navigation {
                rust_i18n::t!("sftp.remote.sync_nav_on").to_string()
            } else {
                rust_i18n::t!("sftp.remote.sync_nav_off").to_string()
            },
        )
        .selected(sync_navigation)
        .size(IconButtonSize::Regular)
        .render()
        .on_click(cx.listener(move |this, _, _, cx| {
            this.toggle_sync_navigation(cx);
        }));
        // 同步导航属于"主远端 ↔ 本地"这一对列的关系；并列列不显示该按钮，
        // 避免一列切换时所有列的按钮同时变亮。

        let compare_btn = IconButton::new(
            format!("remote-compare-{key}"),
            AppIcon::Ui(UiIcon::DIFF),
            rust_i18n::t!("sftp.sync.title").to_string(),
        )
        .size(IconButtonSize::Regular)
        .render()
        .on_click(cx.listener(move |this, _, _, cx| {
            this.open_sync_modal_pane(target, cx);
        }));

        let filter_btn = IconButton::new(
            format!("remote-filter-{key}"),
            AppIcon::Ui(UiIcon::SEARCH),
            rust_i18n::t!("sftp.remote.filter").to_string(),
        )
        .selected(show_filter)
        .size(IconButtonSize::Regular)
        .render()
        .on_click(cx.listener(move |this, _, window, cx| {
            this.toggle_filter_pane(target, window, cx);
        }));

        // 搜索按钮只保留一个入口：点击展开目录过滤，输入框回车进入远程检索。
        // 编辑列表浮层归面板实体渲染，这里只提供实体句柄与当前标签。
        h_flex()
            .gap_1()
            .items_center()
            .child(hidden_btn)
            .when(target == SftpRemotePaneRef::Main, |this| {
                this.child(sync_btn)
            })
            .child(compare_btn)
            .child(filter_btn)
            .into_any_element()
    }
}
