//! SFTP 编辑面板的保存前差异预览：把"最近与远端一致的内容"与当前缓冲区
//! 渲染成统一 diff，确认后再回传。
//!
//! 打开与读取在 `open.rs`，面板状态在 `panel.rs`，回传在 `ops.rs`，
//! 渲染外壳在 `view.rs`。
//!
//! 设计原理（Why）：
//! - 远端生产配置属于"改错一行代价很高"的对象，盲存的心理负担大；先看
//!   diff 再确认回传，把"到底改了什么"变成显式一步。
//! - 生成走 `similar`（组件库 diff 组件自身的同源依赖，无新增传递依赖），
//!   渲染复用 gpui-kit 的 `Diff`/`DiffState` 组件，不自绘行对比。
//! - 代价与局限：diff 按"整份文本与基线比对"生成，几 MB 文件的双列比对
//!   有可感知开销；预览实体在进入/离开视图时构建与丢弃，不做常驻缓存。

use super::entity::EditorPanels;
use super::panel::{EditorPanelState, EditorPhase};
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::diff::{Diff, DiffFile, DiffState};
use gpui_kit::component::input::EditorState;
use gpui_kit::*;
use similar::TextDiff;
use transom_core::{ConnectionId, RemoteTextMeta, TabId};

impl EditorPanels {
    /// 进入差异预览：按基线与缓冲区现值生成统一 diff 并构建预览实体。
    ///
    /// 内容与基线一致（改动被撤销回原样）时不进入视图——空 diff 对用户没有
    /// 信息量，保持编辑视图即可，保存路径照常可用。
    pub(crate) fn show_editor_diff(
        &mut self,
        editor: &Entity<EditorState>,
        cx: &mut Context<Self>,
    ) {
        let Some((baseline, language)) = self
            .map_panel(editor, |item| {
                if !item.loaded || Self::phase_busy(item.phase) {
                    return None;
                }
                Some((item.baseline.to_string(), item.language.clone()))
            })
            .flatten()
        else {
            return;
        };
        let current = editor.read(cx).value().to_string();
        let header_old = rust_i18n::t!("editor.diff_header_old");
        let header_new = rust_i18n::t!("editor.diff_header_new");
        let patch = TextDiff::from_lines(&baseline, &current)
            .unified_diff()
            .context_radius(3)
            .header(&header_old, &header_new)
            .to_string();
        let files = DiffFile::parse(&patch)
            .map(|files| {
                files
                    .into_iter()
                    .map(|file| file.with_language(language.clone()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if files.is_empty() {
            return;
        }
        let diff = cx.new(|cx| DiffState::new(files, cx));
        self.update_panel(editor, |item| {
            item.diff = Some(diff);
            item.showing_diff = true;
        });
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
            item.showing_diff = false;
            item.diff = None;
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
        item.showing_diff = false;
        item.diff = None;
        if !item.dirty {
            item.editor
                .update(cx, |state, cx| state.set_value(text, window, cx));
            item.caret = super::caret_of(&item.editor, cx);
            item.written_size = Some(meta.size);
        }
        cx.notify();
        true
    }

    /// 差异预览主体：统计行 + `Diff` 组件，确认回传与返回编辑的出口都在工具行上。
    pub(super) fn render_diff_view(
        &self,
        item: &EditorPanelState,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let save_editor = item.editor.clone();
        let back_editor = item.editor.clone();
        let (additions, deletions) = item
            .diff
            .as_ref()
            .and_then(|state| state.read(cx).files().first())
            .map(|file| (file.additions(), file.deletions()))
            .unwrap_or((0, 0));
        let toolbar = h_flex()
            .items_center()
            .gap_2()
            .px_2()
            .pt_1()
            .text_xs()
            .text_color(p.muted_foreground)
            .child(
                div().flex_1().child(
                    rust_i18n::t!(
                        "editor.diff_summary",
                        additions = additions,
                        deletions = deletions
                    )
                    .to_string(),
                ),
            )
            .child(
                Button::new("editor-diff-back")
                    .ghost()
                    .compact()
                    .icon(IconName::Pencil)
                    .label(rust_i18n::t!("editor.diff_back").to_string())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.close_editor_diff(&back_editor, window, cx);
                    })),
            )
            .child(
                Button::new("editor-diff-save")
                    .primary()
                    .compact()
                    .icon(IconName::Check)
                    .label(rust_i18n::t!("editor.diff_confirm_upload").to_string())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.save_editor_panel(&save_editor, window, cx);
                    })),
            );
        let body: AnyElement = match item.diff.as_ref() {
            Some(state) => Diff::new(state)
                .line_number(true)
                .syntax_highlight(true)
                .change_background(true)
                .flex_1()
                .w_full()
                .into_any_element(),
            None => v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("editor.diff_empty").to_string())
                .into_any_element(),
        };
        v_flex()
            .flex_1()
            .gap_1()
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
