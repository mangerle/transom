//! SFTP 编辑面板的保存前差异预览：把"最近与远端一致的内容"与当前缓冲区
//! 交由公共 `diff_pane` 在后台计算并在面板内渲染，确认后再回传。
//!
//! 打开与读取在 `open.rs`，面板状态在 `panel.rs`，回传在 `ops.rs`，
//! 渲染外壳在 `view.rs`。
//!
//! 设计原理（Why）：
//! - 远端生产配置属于"改错一行代价很高"的对象，盲存的心理负担大；先看
//!   diff 再确认回传，把"到底改了什么"变成显式一步；
//! - 复用公共 `diff_pane` 底座，与 Git 差异视图完全一致地支持后台线程计算、
//!   上/下一处差异跳转、折叠/展开上下文、并排/统一切换、4 档空白忽略、
//!   词级/整行/无着色切换、语法高亮与划词复制，不再在 UI 线程同步跑 Myers。

use std::rc::Rc;

use super::entity::EditorPanels;
use super::panel::{EditorPanelState, EditorPhase};
use crate::diff_pane::{
    DiffViewOptions, OptionsUpdater, create_diff_state, prepare_diff_document,
    render_change_nav_buttons, render_diff_body, render_option_controls, sync_diff_state_options,
};
use crate::palette::Palette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::EditorState;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::{ConnectionId, RemoteTextMeta, TabId};
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;

impl EditorPanels {
    /// 进入差异预览：按基线与缓冲区现值在后台计算差异并进入预览态。
    ///
    /// 内容与基线完全一致（改动被撤销回原样）时不进入视图——空 diff 对用户没有
    /// 信息量，保持编辑视图即可，保存路径照常可用。
    pub(crate) fn show_editor_diff(
        &mut self,
        editor: &Entity<EditorState>,
        cx: &mut Context<Self>,
    ) {
        let Some(baseline) = self
            .map_panel(editor, |item| {
                if !item.loaded || Self::phase_busy(item.phase) {
                    return None;
                }
                Some(item.baseline.to_string())
            })
            .flatten()
        else {
            return;
        };
        let current = editor.read(cx).value().to_string();
        if baseline == current {
            return;
        }
        self.update_panel(editor, |item| {
            item.showing_diff = true;
            item.diff = None;
            item.diff_error = None;
            item.diff_additions = 0;
            item.diff_deletions = 0;
        });
        self.schedule_editor_diff_prepare(editor, cx);
        cx.notify();
    }

    /// 调度后台线程计算当前编辑面板的保存前差异文档。
    fn schedule_editor_diff_prepare(
        &mut self,
        editor: &Entity<EditorState>,
        cx: &mut Context<Self>,
    ) {
        let current = editor.read(cx).value().to_string();
        let Some(job) = self.map_panel(editor, |item| {
            if !item.showing_diff {
                return None;
            }
            item.diff_compute_token = item.diff_compute_token.wrapping_add(1).max(1);
            item.diff_computing = true;
            item.diff_error = None;
            Some((
                item.baseline.to_string(),
                item.remote_path.clone(),
                item.language.clone(),
                item.diff_options.ignore_whitespace,
                item.diff_compute_token,
            ))
        }) else {
            return;
        };
        let Some((baseline, remote_path, language, ignore_whitespace, token)) = job else {
            return;
        };

        let target_editor = editor.clone();
        let task = cx.spawn(async move |this, cx| {
            let prepared = crate::app::spawn_blocking_local(move || {
                prepare_diff_document(
                    &baseline,
                    &current,
                    &remote_path,
                    &language,
                    ignore_whitespace,
                )
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                enum EditorPlan {
                    Ignore,
                    Failed,
                    ResetStale,
                    Ready(DiffViewOptions),
                }
                let plan = this.map_panel(&target_editor, |item| {
                    if !item.showing_diff || item.diff_compute_token != token {
                        return EditorPlan::Ignore;
                    }
                    item.diff_prepare_task = None;
                    if prepared.is_none() {
                        item.diff_computing = false;
                        item.diff_error = Some(rust_i18n::t!("git.diff.compute_failed").to_string());
                        return EditorPlan::Failed;
                    }
                    if item.diff_options.ignore_whitespace != ignore_whitespace {
                        item.diff_computing = false;
                        return EditorPlan::ResetStale;
                    }
                    EditorPlan::Ready(item.diff_options)
                });
                let Some(plan) = plan else {
                    return;
                };
                match plan {
                    EditorPlan::Ignore => {}
                    EditorPlan::Failed | EditorPlan::ResetStale => {
                        cx.notify();
                    }
                    EditorPlan::Ready(options) => {
                        let Some(prepared) = prepared else {
                            return;
                        };
                        let additions = prepared.additions;
                        let deletions = prepared.deletions;
                        let diff_state = (!prepared.is_empty())
                            .then(|| create_diff_state(prepared.files, options, cx));
                        this.update_panel(&target_editor, |item| {
                            if item.diff_compute_token != token || !item.showing_diff {
                                return;
                            }
                            item.diff_computing = false;
                            item.diff_error = None;
                            item.diff_additions = additions;
                            item.diff_deletions = deletions;
                            item.diff = diff_state;
                        });
                        cx.notify();
                    }
                }
            });
        });
        self.update_panel(editor, |item| {
            item.diff_prepare_task = Some(task);
        });
    }

    /// 更新编辑器差异预览选项：空白忽略变化时后台重算，其余外观选项原地同步。
    pub(crate) fn update_editor_diff_options(
        &mut self,
        editor: &Entity<EditorState>,
        update: impl FnOnce(&mut DiffViewOptions),
        cx: &mut Context<Self>,
    ) {
        let mut update = Some(update);
        let result = self.map_panel(editor, |item| {
            let old_ignore = item.diff_options.ignore_whitespace;
            if let Some(f) = update.take() {
                f(&mut item.diff_options);
            }
            (
                old_ignore,
                item.diff_options,
                item.showing_diff,
                item.diff.clone(),
            )
        });
        let Some((old_ignore, new_options, showing, diff_state)) = result else {
            return;
        };
        if showing && new_options.ignore_whitespace != old_ignore {
            self.schedule_editor_diff_prepare(editor, cx);
        } else if let Some(ds) = diff_state {
            sync_diff_state_options(&ds, new_options, cx);
        }
        cx.notify();
    }

    /// 返回编辑视图：丢弃预览实体，缓冲区内容不动。
    pub(crate) fn close_editor_diff(
        &mut self,
        editor: &Entity<EditorState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.update_panel(editor, |item| {
            item.clear_diff_preview();
        });
        editor.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }
}

/// 对比面板合并写盘后，同步同文件编辑会话所需的远端快照。
pub(crate) struct EditorDiffWriteSync<'a> {
    pub(crate) tab_id: TabId,
    pub(crate) connection_id: ConnectionId,
    pub(crate) remote_path: &'a str,
    pub(crate) text: &'a str,
    pub(crate) meta: RemoteTextMeta,
}

impl EditorPanels {
    /// 对比面板合并写盘后同步同文件编辑会话的基线。
    ///
    /// 未改动的编辑器缓冲区直接跟随远端新内容；有未保存修改时保留用户输入，
    /// 下一次保存会以新指纹为基准，而不是把这次合并当成冲突。
    pub(crate) fn sync_after_diff_write(
        &mut self,
        sync: EditorDiffWriteSync<'_>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let EditorDiffWriteSync {
            tab_id,
            connection_id,
            remote_path,
            text,
            meta,
        } = sync;
        let Some(index) = self
            .manager()
            .panels
            .iter()
            .position(|item| item.same_file(tab_id, connection_id, remote_path))
        else {
            return false;
        };
        let item = &mut self.manager_mut().panels[index];
        item.baseline = SharedString::from(text.to_string());
        item.remote_meta = Some(meta);
        // 合并已成功落盘，编辑器上旧的冲突/失败横幅针对的是过期远端状态。
        item.conflict = false;
        item.error = None;
        if item.phase == EditorPhase::Failed {
            item.phase = EditorPhase::Idle;
        }
        // 远端基线变了，编辑器里正在展示的"保存前 diff"随即过期。
        item.clear_diff_preview();
        if !item.dirty {
            item.editor
                .update(cx, |state, cx| state.set_value(text, window, cx));
            item.caret = super::caret_of(&item.editor, cx);
            item.written_size = Some(meta.size);
        }
        let editor = item.editor.clone();
        let dirty = item.dirty;
        self.clear_change_decorations(&editor, cx);
        if dirty {
            self.schedule_change_decorations(&editor, cx);
        }
        cx.notify();
        true
    }

    /// 差异预览主体：统一工具条（上/下一处差异、模式/空白/高亮/外观）+ 公共 `Diff` 组件。
    pub(super) fn render_diff_view(
        &self,
        item: &EditorPanelState,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let save_editor = item.editor.clone();
        let back_editor = item.editor.clone();
        let opt_editor = item.editor.clone();
        let panels = cx.entity();
        let on_update: OptionsUpdater = Rc::new(move |updater, _window, cx| {
            panels.update(cx, |this, cx| {
                this.update_editor_diff_options(&opt_editor, updater, cx);
            });
        });

        let has_changes = item.diff.is_some();
        let whitespace_only =
            !item.diff_computing && item.diff_error.is_none() && item.diff.is_none();
        let summary_text = if item.diff_computing {
            rust_i18n::t!("git.diff.computing").to_string()
        } else if let Some(err) = item.diff_error.as_ref() {
            err.clone()
        } else if whitespace_only {
            rust_i18n::t!("editor.diff_whitespace_only").to_string()
        } else {
            rust_i18n::t!(
                "editor.diff_summary",
                additions = item.diff_additions,
                deletions = item.diff_deletions
            )
            .to_string()
        };

        let toolbar = h_flex()
            .items_center()
            .gap_1p5()
            .px_2()
            .py_1()
            .border_color(p.divider(p.border))
            .border_b_1()
            .text_xs()
            .text_color(p.muted_foreground)
            .child(render_change_nav_buttons(
                "editor-diff",
                item.diff.clone(),
                has_changes,
            ))
            .child(div().w(px(1.0)).h(px(12.0)).bg(p.divider(p.border)))
            .child(div().flex_1().min_w_0().truncate().child(summary_text))
            .child(render_option_controls(
                "editor-diff",
                item.diff_options,
                on_update,
            ))
            .child(div().w(px(1.0)).h(px(12.0)).bg(p.divider(p.border)))
            .child(
                Button::new("editor-diff-back")
                    .ghost()
                    .compact()
                    .child(AppIconElement::new(AppIcon::Ui(UiIcon::EDIT)).small())
                    .label(rust_i18n::t!("editor.diff_back").to_string())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.close_editor_diff(&back_editor, window, cx);
                    })),
            )
            .child(
                Button::new("editor-diff-save")
                    .primary()
                    .compact()
                    .child(AppIconElement::new(AppIcon::Ui(UiIcon::CHECK)).small())
                    .label(rust_i18n::t!("editor.diff_confirm_upload").to_string())
                    .when(whitespace_only, |btn| {
                        btn.tooltip(rust_i18n::t!("editor.diff_whitespace_upload_hint").to_string())
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.save_editor_panel(&save_editor, window, cx);
                    })),
            );

        let body: AnyElement = if item.diff_computing {
            v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .gap_2()
                .text_sm()
                .text_color(p.muted_foreground)
                .child(Spinner::new().small().color(p.primary))
                .child(rust_i18n::t!("git.diff.computing").to_string())
                .into_any_element()
        } else if let Some(err) = item.diff_error.as_ref() {
            v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(p.danger)
                .child(err.clone())
                .into_any_element()
        } else {
            match item.diff.as_ref() {
                Some(state) => render_diff_body(state, item.diff_options)
                    .flex_1()
                    .w_full()
                    .into_any_element(),
                None => v_flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .text_sm()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("editor.diff_whitespace_only").to_string())
                    .into_any_element(),
            }
        };

        v_flex()
            .flex_1()
            .min_h_0()
            .child(toolbar)
            .child(body)
            .into_any_element()
    }
}

/// 状态栏"对比差异"按钮的可用性：仅已装载、无在途读写且有未保存修改时提供。
pub(crate) fn diff_entry_allowed(item: &EditorPanelState) -> bool {
    item.loaded
        && item.dirty
        && !item.showing_diff
        && !matches!(item.phase, EditorPhase::Reading | EditorPhase::Writing)
}
