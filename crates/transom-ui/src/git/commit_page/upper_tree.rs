//! Git 提交页上半区变更树：改动计数、修正目标提交与文件列表。

use super::DIFF_FOLD_ROW_HEIGHT;
use crate::git::commit_modal::amend_file_row;
use crate::palette::Palette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::CommitFile;
use transom_ui_icons::{AppIconElement, UiIcon};

/// 上半区变更树的参数集合，避免七个平铺参数。
pub(super) struct UpperChangeTree<'a> {
    pub(super) count: usize,
    pub(super) amend: bool,
    pub(super) amend_files_loading: bool,
    pub(super) amend_files: &'a [CommitFile],
    pub(super) amend_head: Option<&'a (String, String)>,
    pub(super) files: AnyElement,
    pub(super) palette: &'a Palette,
}

pub(super) fn render_upper_change_tree(params: UpperChangeTree<'_>) -> impl IntoElement {
    let p = params.palette;
    let UpperChangeTree {
        count,
        amend,
        amend_files_loading,
        amend_files,
        amend_head,
        files,
        palette: _,
    } = params;
    v_flex()
        .w_full()
        .h_full()
        .min_h_0()
        .child(
            h_flex()
                .w_full()
                .h(rems(DIFF_FOLD_ROW_HEIGHT))
                .flex_none()
                .px_2()
                .gap_2()
                .items_center()
                .child(
                    div().text_xs().text_color(p.muted_foreground).child(
                        rust_i18n::t!("git.commit.changes_count", count = count).to_string(),
                    ),
                )
                .child(div().flex_1()),
        )
        // 修正时在上方只读展示目标提交的文件，方便核对这次会追加到哪次提交。
        .when(amend, |el| {
            let mut group = v_flex().w_full().gap_1().child(
                h_flex()
                    .w_full()
                    .px_2()
                    .py_1()
                    .gap_2()
                    .items_center()
                    .bg(p.surface.active)
                    .child(
                        AppIconElement::new(UiIcon::PATCH)
                            .xsmall()
                            .text_color(p.primary),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(
                                amend_head
                                    .map(|(oid, subject)| {
                                        rust_i18n::t!(
                                            "git.commit.amend_group",
                                            oid = oid.as_str(),
                                            subject = subject.as_str()
                                        )
                                        .to_string()
                                    })
                                    .unwrap_or_else(|| {
                                        rust_i18n::t!("git.commit.last_commit").to_string()
                                    }),
                            ),
                    )
                    .child(
                        div().text_xs().text_color(p.muted_foreground).child(
                            rust_i18n::t!("git.commit.file_count", count = amend_files.len())
                                .to_string(),
                        ),
                    ),
            );
            if amend_files_loading {
                group = group.child(crate::git::workspace::hint(
                    rust_i18n::t!("git.overview.loading").to_string(),
                    p,
                ));
            } else if amend_files.is_empty() {
                group = group.child(crate::git::workspace::hint(
                    rust_i18n::t!("git.commit.amend_empty").to_string(),
                    p,
                ));
            } else {
                group = group.children(amend_files.iter().map(|file| amend_file_row(file, p)));
            }
            el.child(group)
        })
        .child(files)
}
