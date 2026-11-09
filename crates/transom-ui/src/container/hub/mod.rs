//! 远端镜像仓库探索、版本选择与详情拉取管理模块。
//!
//! 设计原理（Why）：
//! - 将镜像搜索与市场浏览（Hub）作为多源镜像中心呈现，支持在官方 Docker Hub 与用户
//!   自定义私有 Harbor / ACR 之间即时切换仓库源；
//! - 支持常用版本标签（Tags）秒级推荐与精准自定版本拉取，彻底解决“只能拉取默认版”的问题；
//! - 提供独立的全景详情视图（Detail View），支持查看完整说明、架构支持与 Docker CLI 命令复制；
//! - 采用组件拆分结构（Card / Detail / Tags / View / Dialog），各子模块单一职责且严格控制行数。
//!
//! 动作归属（Why）：搜索、详情选择、版本切换与入队只读写本标签页的 `hub_dialog`
//! 状态并投递命令，因此全部落在 [`ContainerWorkspace`] 上；成功提示经
//! [`ContainerWorkspaceEvent::ShowToast`] 交外壳弹出。

mod card;
mod detail;
mod dialog;
pub(crate) mod tags;
mod view;

pub(crate) use dialog::render_container_hub_dialog_body;

use super::*;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;
use std::collections::HashMap;
use transom_core::{ContainerCommand, ContainerRegistry, ContainerRequestId, ImageSearchResult};

impl ContainerWorkspace {
    /// 打开远端镜像仓库探索弹窗。
    pub(crate) fn open_hub_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let default_reg = app_settings(cx)
            .container_registries
            .iter()
            .find(|registry| registry.is_default)
            .map(|registry| registry.id.clone())
            .unwrap_or_else(|| ContainerRegistry::docker_hub().id);

        let search_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("container.hub.search_placeholder").to_string())
        });

        let subscription = cx.subscribe(&search_input, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.execute_hub_search(cx);
            }
        });

        self.state.hub_dialog = Some(ContainerHubDialog {
            search_input,
            selected_registry_id: default_reg,
            results: Vec::new(),
            searching: false,
            search_error: None,
            search_request: None,
            selected_detail: None,
            tags: Vec::new(),
            tags_loading: false,
            tags_error: None,
            tags_request: None,
            selected_tags: HashMap::new(),
            custom_tag_input: None,
            copy_toast: false,
            _subscription: subscription,
        });
        cx.notify();
    }

    /// 关闭镜像中心弹窗。
    pub(crate) fn close_hub_dialog(&mut self, cx: &mut Context<Self>) {
        self.state.hub_dialog = None;
        cx.notify();
    }

    /// 切换当前搜索使用的镜像仓库源。
    pub(crate) fn select_hub_registry(&mut self, registry_id: String, cx: &mut Context<Self>) {
        if let Some(hub) = self.state.hub_dialog.as_mut() {
            hub.selected_registry_id = registry_id;
        }
        cx.notify();
    }

    /// 用推荐标签填充搜索框并立即搜索。
    ///
    /// 设计原理（Why）：推荐芯片点击等价于「输入并回车」，搜索框文本必须真正
    /// 写回实体，否则用户再按一次搜索会回到旧词。
    pub(crate) fn search_hub_for_term(
        &mut self,
        term: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(input) = self
            .state
            .hub_dialog
            .as_ref()
            .map(|hub| hub.search_input.clone())
        else {
            return;
        };
        input.update(cx, |input, cx| input.set_value(term, window, cx));
        self.execute_hub_search(cx);
    }

    /// 发起远端镜像搜索请求，支持从 `mysql:8.0` 等精准输入中自动提取预选 Tag。
    pub(crate) fn execute_hub_search(&mut self, cx: &mut Context<Self>) {
        let Some(hub) = self.state.hub_dialog.as_ref() else {
            return;
        };
        let raw_input = hub.search_input.read(cx).value().trim().to_string();
        let registry = app_settings(cx)
            .container_registries
            .iter()
            .find(|registry| registry.id == hub.selected_registry_id)
            .cloned()
            .unwrap_or_else(ContainerRegistry::docker_hub);

        if raw_input.is_empty() {
            return;
        }

        let (term, tag_hint) = tags::parse_search_term_and_tag(&raw_input);

        let request_id = ContainerRequestId::new();
        if let Some(hub) = self.state.hub_dialog.as_mut() {
            hub.searching = true;
            hub.search_error = None;
            hub.search_request = Some(request_id);
            // 搜索时重置详情视图与旧标签，避免把上一个仓库的结果带过来。
            hub.selected_detail = None;
            hub.tags.clear();
            hub.tags_error = None;
            hub.tags_loading = false;
            hub.tags_request = None;
            if let Some(tag) = tag_hint {
                hub.selected_tags.insert(term.clone(), tag);
            }
        }

        self.send_tracked(
            request_id,
            ContainerCommand::SearchImages {
                tab_id: self.tab_id,
                request_id,
                registry,
                term,
                limit: Some(25),
            },
        );
        cx.notify();
    }

    /// 选中镜像查看详细信息与全部版本。
    pub(crate) fn select_hub_image_detail(
        &mut self,
        item: ImageSearchResult,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(hub) = self.state.hub_dialog.as_ref() else {
            return;
        };
        let current_tag = hub
            .selected_tags
            .get(&item.name)
            .cloned()
            .unwrap_or_else(|| "latest".to_string());
        let registry = app_settings(cx)
            .container_registries
            .iter()
            .find(|registry| registry.id == hub.selected_registry_id)
            .cloned()
            .unwrap_or_else(ContainerRegistry::docker_hub);

        let custom_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("container.hub.custom_tag_placeholder").to_string())
                .default_value(current_tag)
        });

        let request_id = ContainerRequestId::new();
        let repository = item.name.clone();
        if let Some(hub) = self.state.hub_dialog.as_mut() {
            hub.selected_detail = Some(item);
            hub.custom_tag_input = Some(custom_input);
            hub.copy_toast = false;
            hub.tags.clear();
            hub.tags_loading = true;
            hub.tags_error = None;
            hub.tags_request = Some(request_id);
        }
        self.send_tracked(
            request_id,
            ContainerCommand::ListImageTags {
                tab_id: self.tab_id,
                request_id,
                registry,
                repository,
            },
        );
        cx.notify();
    }

    /// 从镜像详情页返回搜索结果列表。
    pub(crate) fn back_to_hub_results(&mut self, cx: &mut Context<Self>) {
        if let Some(hub) = self.state.hub_dialog.as_mut() {
            hub.selected_detail = None;
            hub.custom_tag_input = None;
            hub.copy_toast = false;
            hub.tags.clear();
            hub.tags_loading = false;
            hub.tags_error = None;
            hub.tags_request = None;
        }
        cx.notify();
    }

    /// 切换某个镜像当前选中的版本标签。
    pub(crate) fn set_hub_selected_tag(
        &mut self,
        image_name: String,
        tag: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(hub) = self.state.hub_dialog.as_mut() {
            hub.selected_tags.insert(image_name, tag);
        }
        cx.notify();
    }

    /// 记录「Docker 命令已复制」提示。
    pub(crate) fn mark_hub_copy_toast(&mut self, cx: &mut Context<Self>) {
        if let Some(hub) = self.state.hub_dialog.as_mut() {
            hub.copy_toast = true;
        }
        cx.notify();
    }

    /// 将选中的远端镜像加入下载队列并自动调度。
    ///
    /// `repository` 与 `tag` 分开传入，由仓库配置负责补全私有仓库域名；
    /// 认证凭据随任务保存，排队中的任务被自动调度时不会丢失私有仓库登录态。
    pub(crate) fn enqueue_image_pull(
        &mut self,
        repository: String,
        tag: String,
        registry: ContainerRegistry,
        cx: &mut Context<Self>,
    ) {
        let reference = registry.qualify_reference(&repository, &tag);
        if reference.is_empty() {
            return;
        }
        let auth = registry.auth();
        let registry_name = registry.name.clone();
        let (task_id, should_start) = self.state.pull_manager.enqueue(
            self.tab_id,
            reference.clone(),
            registry_name,
            auth.clone(),
        );

        // 自动打开下载抽屉以展现分层下载进度。
        self.state.pull_drawer_open = true;

        if should_start {
            self.send(ContainerCommand::PullImageTask {
                tab_id: self.tab_id,
                task_id,
                reference: reference.clone(),
                auth,
            });
            cx.emit(ContainerWorkspaceEvent::ShowToast {
                message: rust_i18n::t!("container.hub.pull_started", reference = reference)
                    .to_string(),
                success: true,
            });
        } else {
            cx.emit(ContainerWorkspaceEvent::ShowToast {
                message: rust_i18n::t!("container.hub.pull_queued", reference = reference)
                    .to_string(),
                success: true,
            });
        }
        cx.notify();
    }

    /// 应用远端搜索响应；请求已被替换或弹窗已关闭时丢弃结果。
    pub(crate) fn apply_images_searched(
        &mut self,
        request_id: ContainerRequestId,
        results: Vec<ImageSearchResult>,
    ) -> bool {
        let Some(hub) = self.state.hub_dialog.as_mut() else {
            return false;
        };
        if hub.search_request != Some(request_id) {
            return false;
        }
        hub.searching = false;
        hub.search_request = None;
        hub.results = results;
        hub.search_error = None;
        true
    }

    /// 应用仓库标签列表响应；选中标签失效时回落到第一个可用标签。
    pub(crate) fn apply_image_tags_listed(
        &mut self,
        request_id: ContainerRequestId,
        tags: Vec<String>,
    ) -> bool {
        let Some(hub) = self.state.hub_dialog.as_mut() else {
            return false;
        };
        if hub.tags_request != Some(request_id) {
            return false;
        }
        hub.tags = tags;
        hub.tags_loading = false;
        hub.tags_error = None;
        hub.tags_request = None;
        // 若当前选中的 tag 不在实时列表里（仓库已删除该 tag），回落到第一个。
        if let Some(detail) = hub.selected_detail.as_ref() {
            let selected = hub
                .selected_tags
                .entry(detail.name.clone())
                .or_insert_with(|| "latest".to_string());
            if !hub.tags.is_empty() && !hub.tags.iter().any(|tag| tag == selected) {
                *selected = hub.tags[0].clone();
            }
        }
        true
    }
}
