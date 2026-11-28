//! AI 消息流单一数据源模型（纯 Rust 状态，零 GPUI 依赖）。
//!
//! 设计原理（Why）：
//! - 将消息条目列表（`items`）、卡片展开集合（`expanded_cards`）、分组版本（`group_revision`）
//!   与渲染窗口大小（`render_window_blocks`）集中收口在 [`MessageStreamModel`] 中，
//!   由 [`crate::app::ai::message_stream::MessageStreamView`] 唯一持有；
//! - 父视图 [`crate::app::ai::view::AiAssistantView`] 不再保存第二份 `items`，彻底消除
//!   每次状态同步时的整表 `items.clone()` 与流式期间的双份更新漂移。

use std::collections::HashSet;

use transom_core::{AiAppEvent, AiToolCardDto};

use super::super::markdown_blocks::StreamingMarkdownCache;
use super::super::state_types::UiChatItem;

/// 默认消息流末尾展示窗口块数。
pub(in crate::app::ai) const DEFAULT_RENDER_WINDOW_BLOCKS: usize = 160;

/// AI 消息流单一数据源模型。
#[derive(Debug)]
pub(in crate::app::ai) struct MessageStreamModel {
    items: Vec<UiChatItem>,
    expanded_cards: HashSet<String>,
    group_revision: u64,
    render_window_blocks: usize,
}

impl Default for MessageStreamModel {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            expanded_cards: HashSet::new(),
            group_revision: 0,
            render_window_blocks: DEFAULT_RENDER_WINDOW_BLOCKS,
        }
    }
}

impl MessageStreamModel {
    /// 返回全部消息条目切片。
    pub(in crate::app::ai) fn items(&self) -> &[UiChatItem] {
        &self.items
    }

    /// 返回当前消息条目总数。
    pub(in crate::app::ai) fn item_count(&self) -> usize {
        self.items.len()
    }

    /// 消息流是否为空。
    pub(in crate::app::ai) fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// 返回已展开卡片 ID 集合。
    pub(in crate::app::ai) fn expanded_cards(&self) -> &HashSet<String> {
        &self.expanded_cards
    }

    /// 返回当前分组结构版本号。
    pub(in crate::app::ai) fn group_revision(&self) -> u64 {
        self.group_revision
    }

    /// 返回当前末尾展示窗口块数上限。
    pub(in crate::app::ai) fn render_window_blocks(&self) -> usize {
        self.render_window_blocks
    }

    /// 返回最后一条已完成回复的 `(id, text)`（若存在且非流式）。
    pub(in crate::app::ai) fn last_completed_assistant(&self) -> Option<(&str, &str)> {
        match self.items.last()? {
            UiChatItem::AssistantText {
                id,
                text,
                streaming: false,
                ..
            } => Some((id.as_str(), text.as_str())),
            _ => None,
        }
    }

    /// 返回最后一条助手消息的 ID（无论是否流式）。
    pub(in crate::app::ai) fn last_assistant_id(&self) -> Option<&str> {
        match self.items.last()? {
            UiChatItem::AssistantText { id, .. } => Some(id.as_str()),
            _ => None,
        }
    }

    /// 清空全部消息流并重置窗口与分组版本。
    pub(in crate::app::ai) fn clear(&mut self) {
        self.items.clear();
        self.expanded_cards.clear();
        self.group_revision = self.group_revision.wrapping_add(1);
        self.render_window_blocks = DEFAULT_RENDER_WINDOW_BLOCKS;
    }

    /// 用历史会话条目整表替换当前消息流并重置窗口。
    pub(in crate::app::ai) fn replace_items(&mut self, items: Vec<UiChatItem>) {
        self.items = items;
        self.expanded_cards.clear();
        self.group_revision = self.group_revision.wrapping_add(1);
        self.render_window_blocks = DEFAULT_RENDER_WINDOW_BLOCKS;
    }

    /// 追加单条消息条目。
    pub(in crate::app::ai) fn push_item(&mut self, item: UiChatItem) {
        self.items.push(item);
    }

    /// 按消息 ID 删除条目。
    pub(in crate::app::ai) fn remove_item_by_id(&mut self, target_id: &str) {
        self.items.retain(|item| match item {
            UiChatItem::UserText { id, .. }
            | UiChatItem::Reasoning { id, .. }
            | UiChatItem::AssistantText { id, .. }
            | UiChatItem::ToolCard { id, .. }
            | UiChatItem::AuditNotice { id, .. } => id != target_id,
        });
    }

    /// 切换指定卡片或输出块的展开/折叠状态。
    pub(in crate::app::ai) fn toggle_expanded(&mut self, id: String) {
        if !self.expanded_cards.remove(&id) {
            self.expanded_cards.insert(id);
        }
    }

    /// 向上扩充历史消息可见块窗口。
    pub(in crate::app::ai) fn expand_older_window(&mut self, step: usize) {
        self.render_window_blocks = self.render_window_blocks.saturating_add(step);
    }

    /// 原地追加流式思考或正文增量。
    pub(in crate::app::ai) fn apply_streaming_delta(&mut self, is_reasoning: bool, delta: &str) {
        if is_reasoning {
            if let Some(UiChatItem::Reasoning {
                text,
                streaming: true,
                ..
            }) = self.items.last_mut()
            {
                text.push_str(delta);
            } else {
                self.items.push(UiChatItem::Reasoning {
                    id: format!("stream-r-{}", self.items.len()),
                    text: delta.to_owned(),
                    streaming: true,
                });
            }
            return;
        }

        if let Some(UiChatItem::Reasoning { streaming, .. }) = self.items.last_mut() {
            *streaming = false;
        }
        if let Some(UiChatItem::AssistantText {
            text,
            streaming: true,
            cached_segments,
            ..
        }) = self.items.last_mut()
        {
            text.push_str(delta);
            cached_segments
                .get_or_insert_with(StreamingMarkdownCache::default)
                .update(text);
        } else {
            let mut cache = StreamingMarkdownCache::default();
            cache.update(delta);
            self.items.push(UiChatItem::AssistantText {
                id: format!("stream-t-{}", self.items.len()),
                text: delta.to_owned(),
                model: None,
                tokens_per_sec: None,
                streaming: true,
                cached_segments: Some(cache),
            });
        }
    }

    /// 将控制平面通知的事件路由到消息流模型原地更新。
    pub(in crate::app::ai) fn apply_app_event(
        &mut self,
        event: &AiAppEvent,
        active_model_name: Option<String>,
        is_current_conversation: bool,
    ) {
        match event {
            AiAppEvent::RunStarted {
                run_id,
                user_entry_id,
                ..
            } => {
                if !user_entry_id.trim().is_empty() {
                    let temp_id = format!("local-u-{run_id}");
                    for item in &mut self.items {
                        if let UiChatItem::UserText { id, .. } = item
                            && *id == temp_id
                        {
                            *id = user_entry_id.clone();
                        }
                    }
                }
            }
            AiAppEvent::ReasoningDelta { delta, .. } => {
                self.apply_streaming_delta(true, delta);
            }
            AiAppEvent::TextDelta { delta, .. } => {
                self.apply_streaming_delta(false, delta);
            }
            AiAppEvent::TurnEnd {
                reasoning,
                text,
                tokens_per_sec,
                ..
            } => {
                self.group_revision = self.group_revision.wrapping_add(1);
                self.finalize_turn(reasoning, text, active_model_name, *tokens_per_sec);
            }
            AiAppEvent::ToolCard(dto) => {
                self.push_tool_card(dto);
            }
            AiAppEvent::RunFinished { .. } if is_current_conversation => {
                self.finish_running_items();
            }
            _ => {}
        }
    }

    fn finalize_turn(
        &mut self,
        reasoning: &str,
        text: &str,
        model_name: Option<String>,
        tokens_per_sec: Option<f32>,
    ) {
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
                text: reasoning.to_owned(),
                streaming: false,
            });
        }
        if !text.trim().is_empty() {
            let cached_segments = Some(StreamingMarkdownCache::from_complete(text));
            self.items.push(UiChatItem::AssistantText {
                id: format!("turn-t-{}", self.items.len()),
                text: text.to_owned(),
                model: model_name,
                tokens_per_sec,
                streaming: false,
                cached_segments,
            });
        }
    }

    fn push_tool_card(&mut self, dto: &AiToolCardDto) {
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
            *tool_name = dto.tool_name.clone();
            *host_label = dto.host_label.clone();
            *risk_label = dto.risk_label.clone();
            *summary = dto.summary.clone();
            *output = dto.output.clone();
            *ok = dto.ok;
            *duration_ms = dto.duration_ms;
            *diff_preview = dto.diff_preview.clone();
            *revert_target_path = dto.revert_target_path.clone();
            *revert_backup_path = dto.revert_backup_path.clone();
            return;
        }
        self.items.push(UiChatItem::ToolCard {
            id: format!("tool-{}", self.items.len()),
            call_id: dto.call_id.clone(),
            running: dto.running,
            tool_name: dto.tool_name.clone(),
            host_label: dto.host_label.clone(),
            risk_label: dto.risk_label.clone(),
            summary: dto.summary.clone(),
            output: dto.output.clone(),
            ok: dto.ok,
            duration_ms: dto.duration_ms,
            diff_preview: dto.diff_preview.clone(),
            revert_target_path: dto.revert_target_path.clone(),
            revert_backup_path: dto.revert_backup_path.clone(),
        });
    }

    fn finish_running_items(&mut self) {
        for item in &mut self.items {
            match item {
                UiChatItem::Reasoning { streaming, .. } => {
                    *streaming = false;
                }
                UiChatItem::AssistantText {
                    text,
                    streaming,
                    cached_segments,
                    ..
                } => {
                    *streaming = false;
                    if let Some(cache) = cached_segments {
                        cache.update(text);
                    } else {
                        *cached_segments = Some(StreamingMarkdownCache::from_complete(text));
                    }
                }
                UiChatItem::ToolCard { running, .. } => {
                    *running = false;
                }
                _ => {}
            }
        }
    }
}
