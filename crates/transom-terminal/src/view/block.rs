//! 终端语义块悬浮操作与控制逻辑。

use super::TerminalView;
use alacritty_terminal::grid::Dimensions;
use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::*;
use transom_core::BlockViewSettings;

impl TerminalView {
    /// 应用语义块视图配置；`None` 或 `enabled == false` 时回到经典连续流。
    ///
    /// 热切换只改下一帧的行映射与自动折叠阈值，不重建会话、不清屏，
    /// 也不影响已建立的选择与历史缓冲。
    pub fn set_block_view(&mut self, settings: Option<BlockViewSettings>, cx: &mut Context<Self>) {
        if let Some(settings) = settings {
            self.state
                .engine
                .lock()
                .set_block_auto_collapse(settings.auto_collapse_rows);
        }
        self.block_view = settings;
        if !self.block_view.is_some_and(|settings| settings.enabled) {
            self.hover_block = None;
            *self.state.block_layout.lock() = None;
        }
        cx.notify();
    }

    /// 运行时一键切换语义块/经典连续流；返回切换后是否启用块视图。
    pub fn toggle_block_view(&mut self, cx: &mut Context<Self>) -> bool {
        let mut settings = self.block_view.unwrap_or_default();
        settings.enabled = !settings.enabled;
        let enabled = settings.enabled;
        self.set_block_view(Some(settings), cx);
        enabled
    }

    pub fn block_view_enabled(&self) -> bool {
        self.block_view.is_some_and(|settings| settings.enabled)
    }

    /// 切换指定命令块的折叠状态（横幅点击与悬浮胶囊共用）。
    pub fn toggle_block(&mut self, id: u64, cx: &mut Context<Self>) {
        if self.state.engine.lock().toggle_block(id) {
            cx.notify();
        }
    }

    pub(super) fn copy_block_output(&mut self, id: u64, cx: &mut Context<Self>) {
        let text = self.state.engine.lock().block_output_text(id);
        if let Some(text) = text.filter(|text| !text.is_empty()) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            cx.emit(crate::event::TerminalEvent::ClipboardStored);
        }
    }

    pub(super) fn copy_block_markdown(&mut self, id: u64, cx: &mut Context<Self>) {
        let text = self.state.engine.lock().block_markdown(id);
        if let Some(text) = text {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            cx.emit(crate::event::TerminalEvent::ClipboardStored);
        }
    }

    /// 重新执行命令块：把命令写进当前窗格并回车。
    pub(super) fn rerun_block(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(command) = self.state.engine.lock().block_command_text(id) else {
            return;
        };
        if command.trim().is_empty() {
            return;
        }
        let mut bytes = command.into_bytes();
        bytes.push(b'\r');
        if self.write_input(&bytes) {
            self.state.engine.lock().scroll_to_bottom();
            self.input_line.clear();
            self.clear_suggestions();
            cx.notify();
        }
    }

    /// 命中折叠横幅时返回块 ID。
    pub(super) fn banner_at_position(&self, position: Point<Pixels>) -> Option<u64> {
        let (row, _) = self.hit_test_screen_cell(position)?;
        self.state.block_layout.lock().as_ref()?.banner_at(row)
    }

    /// 依据指针位置更新悬浮命令块；只在跨越块边界时请求重绘。
    pub(super) fn update_hover_block(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let next = self.hover_block_at_position(position);
        if next != self.hover_block {
            self.hover_block = next;
            cx.notify();
        }
    }

    pub(super) fn hover_block_at_position(&self, position: Point<Pixels>) -> Option<(u64, usize)> {
        let (row, _) = self.hit_test_screen_cell(position)?;
        let layout = self.state.block_layout.lock().clone()?;
        if let Some(id) = layout.banner_at(row) {
            return Some((id, row));
        }
        let line = layout.line_at(row)?;
        let engine = self.state.engine.lock();
        let absolute = engine.grid().history_size() as i64 + i64::from(line.0);
        engine
            .command_block_at(absolute)
            .map(|block| (block.id, row))
    }

    /// 悬浮操作胶囊：复制输出、复制 Markdown、重跑与折叠切换。
    pub(super) fn render_block_actions(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (block_id, row) = self.hover_block?;
        let metrics = *self.state.metrics.lock();
        let bounds = metrics.bounds?;
        if metrics.cell_height <= px(0.0) {
            return None;
        }
        let top = metrics.origin.y - bounds.origin.y + metrics.cell_height * row as f32;
        let collapsed = self
            .state
            .engine
            .lock()
            .command_block(block_id)
            .is_some_and(|block| block.collapsed);
        let fold_icon = if collapsed {
            IconName::UnfoldVertical
        } else {
            IconName::FoldVertical
        };
        let fold_id = ElementId::Name(format!("block-fold-{block_id}").into());
        let copy_id = ElementId::Name(format!("block-copy-{block_id}").into());
        let markdown_id = ElementId::Name(format!("block-markdown-{block_id}").into());
        let rerun_id = ElementId::Name(format!("block-rerun-{block_id}").into());
        Some(
            div()
                .id(ElementId::Name(format!("block-actions-{block_id}").into()))
                .absolute()
                .top(top)
                .right(px(8.0))
                .flex()
                .gap_1()
                .child(
                    Button::new(copy_id)
                        .ghost()
                        .small()
                        .icon(IconName::Copy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.copy_block_output(block_id, cx);
                        })),
                )
                .child(
                    Button::new(markdown_id)
                        .ghost()
                        .small()
                        .icon(IconName::FileText)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.copy_block_markdown(block_id, cx);
                        })),
                )
                .child(
                    Button::new(rerun_id)
                        .ghost()
                        .small()
                        .icon(IconName::Play)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.rerun_block(block_id, cx);
                        })),
                )
                .child(
                    Button::new(fold_id)
                        .ghost()
                        .small()
                        .icon(fold_icon)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.toggle_block(block_id, cx);
                        })),
                )
                .into_any_element(),
        )
    }
}
