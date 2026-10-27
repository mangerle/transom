//! 虚拟文件系统通用列表项行视觉组件。
//!
//! 设计原理（Why）：
//! - 实现初衷：消除本地文件浏览器、主远端 SFTP 与并列远端列中单行 UI 布局与样式的重复；
//! - 核心优势：
//!   1. 统一图标、文字截断、POSIX 权限徽章与字节单位格式化布局；
//!   2. 视觉样式集中维护，修改文件行交互与外观时仅需调整单一组件；
//!   3. 零成本适配 `VfsEntry` 契约，解耦具体的底层文件系统来源。

use super::{format_size, sftp_file_icon};
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::h_flex;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::format_octal_permissions;

/// 虚拟文件项渲染参数配置。
pub(crate) struct VfsRowRenderOptions<'a> {
    pub(crate) name: &'a str,
    pub(crate) is_dir: bool,
    pub(crate) is_parent: bool,
    pub(crate) size: u64,
    pub(crate) permissions: Option<u32>,
    pub(crate) palette: &'a Palette,
}

/// 渲染标准虚拟文件条目的内层视觉元素（图标、文件名、权限徽章、格式化大小）。
pub(crate) fn render_vfs_row_content(options: VfsRowRenderOptions<'_>) -> impl IntoElement {
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
                    ".. (返回上一级)".to_string()
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
