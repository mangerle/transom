//! AI 面板单轮落库定稿、工具卡片增量更新与状态单元测试。

use std::sync::Arc;

use transom_core::AiToolCardDto;

use super::markdown_blocks::parse_markdown_segments;
use super::state::AiPanelState;
use super::state_types::UiChatItem;

impl AiPanelState {
    pub(super) fn finalize_turn(
        &mut self,
        reasoning: String,
        text: String,
        tokens_per_sec: Option<f32>,
    ) {
        // 移除末尾的流式暂态条目，以规范化后的 TurnEnd 替换
        while matches!(
            self.items.last(),
            Some(
                UiChatItem::Reasoning {
                    streaming: true,
                    ..
                } | UiChatItem::AssistantText {
                    streaming: true,
                    ..
                }
            )
        ) {
            self.items.pop();
        }
        if !reasoning.trim().is_empty() {
            self.items.push(UiChatItem::Reasoning {
                id: format!("turn-r-{}", self.items.len()),
                text: reasoning,
                streaming: false,
            });
        }
        if !text.trim().is_empty() {
            let model_name = self
                .settings
                .resolve_model(&self.selected_model_id)
                .map(|(_, m)| m.name.clone());
            let cached_segments = Some(Arc::new(parse_markdown_segments(&text)));
            self.items.push(UiChatItem::AssistantText {
                id: format!("turn-t-{}", self.items.len()),
                text,
                model: model_name,
                tokens_per_sec,
                streaming: false,
                cached_segments,
            });
        }
    }

    pub(super) fn push_tool_card(&mut self, dto: AiToolCardDto) {
        if !dto.call_id.is_empty()
            && let Some(UiChatItem::ToolCard {
                running,
                tool_name,
                host_label,
                risk_label,
                summary,
                output,
                ok,
                duration_ms,
                diff_preview,
                revert_target_path,
                revert_backup_path,
                ..
            }) = self.items.iter_mut().rev().find(
                |it| matches!(it, UiChatItem::ToolCard { call_id, .. } if call_id == &dto.call_id),
            )
        {
            *running = dto.running;
            *tool_name = dto.tool_name;
            *host_label = dto.host_label;
            *risk_label = dto.risk_label;
            *summary = dto.summary;
            *output = dto.output;
            *ok = dto.ok;
            *duration_ms = dto.duration_ms;
            *diff_preview = dto.diff_preview;
            *revert_target_path = dto.revert_target_path;
            *revert_backup_path = dto.revert_backup_path;
            return;
        }
        self.items.push(UiChatItem::ToolCard {
            id: format!("tool-{}", self.items.len()),
            call_id: dto.call_id,
            running: dto.running,
            tool_name: dto.tool_name,
            host_label: dto.host_label,
            risk_label: dto.risk_label,
            summary: dto.summary,
            output: dto.output,
            ok: dto.ok,
            duration_ms: dto.duration_ms,
            diff_preview: dto.diff_preview,
            revert_target_path: dto.revert_target_path,
            revert_backup_path: dto.revert_backup_path,
        });
    }
}
