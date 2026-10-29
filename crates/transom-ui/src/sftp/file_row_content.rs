//! 文件列表行的内容渲染：图标、截断文件名、POSIX 权限徽章与容量。
//!
//! 设计原理（Why）：本地栏、主远端栏与并列远端共用同一套行内布局，
//! 图标选择、权限徽章与容量格式化在这里单点维护；行的外壳样式与交互
//! 由 [`super::file_row`] 负责，两者组合成最终条目。

use super::{format_size, sftp_file_icon};
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::h_flex;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::format_octal_permissions;

/// 虚拟文件项渲染参数配置。
pub(crate) struct FileRowContentOptions<'a> {
    pub(crate) name: &'a str,
    pub(crate) is_dir: bool,
    pub(crate) is_parent: bool,
    pub(crate) size: u64,
    pub(crate) permissions: Option<u32>,
    pub(crate) palette: &'a Palette,
}

/// 渲染标准虚拟文件条目的内层视觉元素（图标、文件名、权限徽章、格式化大小）。
pub(crate) fn render_file_row_content(options: FileRowContentOptions<'_>) -> impl IntoElement {
    let p = options.palette;
    let is_dir = options.is_dir;
    let is_parent = options.is_parent;
    let name = options.name;

    h_flex()
        .flex_1()
        .gap_2()
        .items_center()
        .overflow_hidden()
        .child(if is_parent {
            Icon::new(IconName::Folder)
                .small()
                .text_color(p.muted_foreground)
                .into_any_element()
        } else {
            sftp_file_icon(is_dir, p)
        })
        .child(
            div()
                .flex_1()
                .text_sm()
                .overflow_hidden()
                .child(if is_parent {
                    rust_i18n::t!("sftp.row.parent_entry").to_string()
                } else {
                    name.to_string()
                }),
        )
        .when_some(options.permissions, |this, perms| {
            this.child(
                div()
                    .text_xs()
                    .px_1()
                    .rounded(p.radius.sm)
                    .bg(p.muted)
                    .text_color(p.muted_foreground)
                    .child(format_octal_permissions(perms)),
            )
        })
        .child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(if is_dir {
                    String::new()
                } else {
                    format_size(options.size)
                }),
        )
}
