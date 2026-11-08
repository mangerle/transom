//! 远端镜像仓库探索、版本选择与详情拉取管理模块。
//!
//! 设计原理（Why）：
//! - 将镜像搜索与市场浏览（Hub）作为多源镜像中心呈现，支持在官方 Docker Hub 与用户
//!   自定义私有 Harbor / ACR 之间即时切换仓库源；
//! - 支持常用版本标签（Tags）秒级推荐与精准自定版本拉取，彻底解决“只能拉取默认版”的问题；
//! - 提供独立的全景详情视图（Detail View），支持查看完整说明、架构支持与 Docker CLI 命令复制；
//! - 采用组件拆分结构（Card / Detail / Tags / View / Dialog），各子模块单一职责且严格控制行数。

mod card;
mod detail;
mod dialog;
pub(crate) mod tags;
mod view;

use super::*;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;
use std::collections::HashMap;
use transom_core::{
    ContainerCommand, ContainerRegistry, ContainerRequestId, ImageSearchResult, TabId,
};

impl AppShell {
    /// 打开远端镜像仓库探索弹窗。
    pub(crate) fn open_container_hub_dialog(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let default_reg = app_settings(cx)
            .container_registries
            .iter()
            .find(|r| r.is_default)
            .map(|r| r.id.clone())
            .unwrap_or_else(|| ContainerRegistry::docker_hub().id);

        let search_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("container.hub.search_placeholder").to_string())
        });

        let subscription = cx.subscribe(&search_input, move |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.execute_hub_search(tab_id, cx);
            }
        });

        self.with_container_mut(tab_id, cx, |workspace| {
            workspace.hub_dialog = Some(ContainerHubDialog {
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
        });
        cx.notify();
    }

    /// 发起远端镜像搜索请求，支持从 `mysql:8.0` 等精准输入中自动提取预选 Tag。
    pub(crate) fn execute_hub_search(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let (raw_input, registry) = {
            let Some(workspace) = self.state.container_workspace(tab_id) else {
                return;
            };
            let workspace = &workspace.read(cx).state;
            let Some(hub) = workspace.hub_dialog.as_ref() else {
                return;
            };
            let term = hub.search_input.read(cx).value().trim().to_string();
            let registry = app_settings(cx)
                .container_registries
                .iter()
                .find(|registry| registry.id == hub.selected_registry_id)
                .cloned()
                .unwrap_or_else(ContainerRegistry::docker_hub);
            (term, registry)
        };

        if raw_input.is_empty() {
            return;
        }

        let (term, tag_hint) = tags::parse_search_term_and_tag(&raw_input);

        let request_id = ContainerRequestId::new();
        self.with_container_mut(tab_id, cx, |workspace| {
            if let Some(hub) = workspace.hub_dialog.as_mut() {
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
        });

        self.send_tracked(
            tab_id,
            request_id,
            ContainerCommand::SearchImages {
                tab_id,
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
        tab_id: TabId,
        item: ImageSearchResult,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (current_tag, registry) = {
            let Some(workspace) = self.state.container_workspace(tab_id) else {
                return;
            };
            let workspace = &workspace.read(cx).state;
            let Some(hub) = workspace.hub_dialog.as_ref() else {
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
            (current_tag, registry)
        };

        let custom_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("container.hub.custom_tag_placeholder").to_string())
                .default_value(current_tag)
        });

        let request_id = ContainerRequestId::new();
        let repository = item.name.clone();
        self.with_container_mut(tab_id, cx, |workspace| {
            if let Some(hub) = workspace.hub_dialog.as_mut() {
                hub.selected_detail = Some(item);
                hub.custom_tag_input = Some(custom_input);
                hub.copy_toast = false;
                hub.tags.clear();
                hub.tags_loading = true;
                hub.tags_error = None;
                hub.tags_request = Some(request_id);
            }
        });
        self.send_tracked(
            tab_id,
            request_id,
            ContainerCommand::ListImageTags {
                tab_id,
                request_id,
                registry,
                repository,
            },
        );
        cx.notify();
    }

    /// 从镜像详情页返回搜索结果列表。
    pub(crate) fn back_to_hub_results(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        self.with_container_mut(tab_id, cx, |workspace| {
            if let Some(hub) = workspace.hub_dialog.as_mut() {
                hub.selected_detail = None;
                hub.custom_tag_input = None;
                hub.copy_toast = false;
                hub.tags.clear();
                hub.tags_loading = false;
                hub.tags_error = None;
                hub.tags_request = None;
            }
        });
        cx.notify();
    }

    /// 切换某个镜像当前选中的版本标签。
    pub(crate) fn set_hub_selected_tag(
        &mut self,
        tab_id: TabId,
        image_name: String,
        tag: String,
        cx: &mut Context<Self>,
    ) {
        self.with_container_mut(tab_id, cx, |workspace| {
            if let Some(hub) = workspace.hub_dialog.as_mut() {
                hub.selected_tags.insert(image_name, tag);
            }
        });
        cx.notify();
    }

    /// 将选中的远端镜像加入下载队列并自动调度。
    ///
    /// `repository` 与 `tag` 分开传入，由仓库配置负责补全私有仓库域名；
    /// 认证凭据随任务保存，排队中的任务被自动调度时不会丢失私有仓库登录态。
    pub(crate) fn enqueue_image_pull(
        &mut self,
        tab_id: TabId,
        repository: String,
        tag: String,
        registry: ContainerRegistry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let reference = registry.qualify_reference(&repository, &tag);
        if reference.is_empty() {
            return;
        }
        let auth = registry.auth();
        let registry_name = registry.name.clone();
        let Some((task_id, should_start)) = self.with_container_mut(tab_id, cx, |workspace| {
            workspace
                .pull_manager
                .enqueue(tab_id, reference.clone(), registry_name, auth.clone())
        }) else {
            return;
        };

        // 自动打开下载抽屉以展现分层下载进度
        self.with_container_mut(tab_id, cx, |workspace| {
            workspace.pull_drawer_open = true;
        });

        if should_start {
            self.state.send_command(ContainerCommand::PullImageTask {
                tab_id,
                task_id,
                reference: reference.clone(),
                auth,
            });
            self.show_toast(
                rust_i18n::t!("container.hub.pull_started", reference = reference).to_string(),
                true,
                window,
                cx,
            );
        } else {
            self.show_toast(
                rust_i18n::t!("container.hub.pull_queued", reference = reference).to_string(),
                true,
                window,
                cx,
            );
        }
        cx.notify();
    }
}
