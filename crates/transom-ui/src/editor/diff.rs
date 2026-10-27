//! SFTP 编辑面板的保存前差异预览：把"最近与远端一致的内容"与当前缓冲区
//! 渲染成统一 diff，确认后再回传。
//!
//! 打开与读取在 `editor_open.rs`，面板状态在 `editor_panel.rs`，
//! 回传在 `editor_ops.rs`，渲染外壳在 `editor_view.rs`。
//!
//! 设计原理（Why）：
//! - 远端生产配置属于"改错一行代价很高"的对象，盲存的心理负担大；先看
//!   diff 再确认回传，把"到底改了什么"变成显式一步。
//! - 生成走 `similar`（组件库 diff 组件自身的同源依赖，无新增传递依赖），
//!   渲染复用 gpui-kit 的 `Diff`/`DiffState` 组件，不自绘行对比。
//! - 代价与局限：diff 按"整份文本与基线比对"生成，几 MB 文件的双列比对
//!   有可感知开销；预览实体在进入/离开视图时构建与丢弃，不做常驻缓存。

use super::panel::{EditorPanelState, EditorPhase};
use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::diff::{Diff, DiffFile, DiffState};
use gpui_kit::component::input::EditorState;
use gpui_kit::*;
use similar::TextDiff;

impl AppShell {
    /// 进入差异预览：按基线与缓冲区现值生成统一 diff 并构建预览实体。
    ///
    /// 内容与基线一致（改动被撤销回原样）时不进入视图——空 diff 对用户没有
    /// 信息量，保持编辑视图即可，保存路径照常可用。
    pub(crate) fn show_editor_diff(
        &mut self,
        editor: &Entity<EditorState>,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = self.editor_panel_mut(editor) else {
            return;
        };
        if !item.loaded || matches!(item.phase, EditorPhase::Reading | EditorPhase::Writing) {
            return;
        }
        let baseline = item.baseline.to_string();
        let current = editor.read(cx).value().to_string();
        let language = item.language.clone();
        let patch = TextDiff::from_lines(&baseline, &current)
            .unified_diff()
            .context_radius(3)
            .header("a/原内容", "b/当前内容")
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
        if let Some(item) = self.editor_panel_mut(editor) {
            item.diff = Some(diff);
            item.showing_diff = true;
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
        if let Some(item) = self.editor_panel_mut(editor) {
            item.showing_diff = false;
            item.diff = None;
        }
        editor.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }
}

/// 差异预览主体：统计行 + `Diff` 组件，确认回传与返回编辑的出口都在工具行上。
pub(crate) fn render_diff_view(
    item: &EditorPanelState,
    p: &Palette,
    cx: &mut Context<AppShell>,
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
        .px_1()
        .text_xs()
        .text_color(p.muted_foreground)
        .child(
            div()
                .flex_1()
                .child(format!("待回传变更：+{additions} 行，-{deletions} 行")),
        )
        .child(
            Button::new("editor-diff-back")
                .ghost()
                .compact()
                .icon(IconName::Pencil)
                .label("返回编辑")
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.close_editor_diff(&back_editor, window, cx);
                })),
        )
        .child(
            Button::new("editor-diff-save")
                .primary()
                .compact()
                .icon(IconName::Check)
                .label("确认回传")
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
            .child("差异尚未生成")
            .into_any_element(),
    };
    v_flex()
        .flex_1()
        .gap_1()
        .child(toolbar)
        .child(body)
        .into_any_element()
}

/// 状态栏"对比差异"按钮的可用性：仅已装载、无在途读写且有未保存修改时提供。
pub(crate) fn diff_entry_allowed(item: &EditorPanelState) -> bool {
    item.loaded
        && item.dirty
        && !item.showing_diff
        && !matches!(item.phase, EditorPhase::Reading | EditorPhase::Writing)
}
