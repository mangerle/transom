//! 容器侧栏事件的外壳处理：目录写入、窗口动作与一次性提示。
//!
//! 设计原理（Why）：侧栏实体不持有外壳句柄，也不读会话目录；所有需要目录
//! 写入、窗口或跨标签协调的动作都以 [`ContainerSidebarEvent`] 上交。事件在
//! 实体更新结束后统一派发，因此这里可以安全地回写侧栏实体（如表单错误）。

use super::*;
use crate::container::{
    ContainerSidebarDrag, ContainerSidebarEvent, ContainerSidebarTarget, ContainerSidebarView,
};
use transom_core::CatalogError;

impl AppShell {
    /// 处理一条侧栏动作意图。
    pub(crate) fn on_container_sidebar_event(
        &mut self,
        event: &ContainerSidebarEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let sidebar = self.state.containers.sidebar.clone();
        match event {
            ContainerSidebarEvent::OpenEngine(profile) => {
                self.open_engine_workspace((**profile).clone(), window, cx);
            }
            ContainerSidebarEvent::OpenEngineForm(profile) => {
                let profile = profile.as_ref().map(|profile| (**profile).clone());
                self.open_container_form(profile, None, window, cx);
            }
            ContainerSidebarEvent::OpenNewEngineForm(folder) => {
                self.open_new_container_in_folder(*folder, window, cx);
            }
            ContainerSidebarEvent::DeleteEngine(id) => {
                self.delete_container_profile(*id, window, cx);
            }
            ContainerSidebarEvent::MoveEngineToRoot(id) => {
                match self.state.catalog.move_engine(*id, None) {
                    Ok(()) => {
                        self.state.persist_catalog();
                    }
                    Err(error) => self.state.set_error(error.to_string()),
                }
                cx.notify();
            }
            ContainerSidebarEvent::ToggleFolder(folder_id) => {
                self.toggle_engine_folder(*folder_id, cx);
            }
            ContainerSidebarEvent::SubmitFolderForm {
                parent,
                editing,
                name,
            } => {
                self.submit_engine_folder_form(*parent, *editing, name.clone(), &sidebar, cx);
            }
            ContainerSidebarEvent::DeleteFolder(folder_id) => {
                self.delete_engine_folder(*folder_id, window, cx);
            }
            ContainerSidebarEvent::Drop { drag, target } => {
                self.handle_container_sidebar_drop(*drag, *target, cx);
            }
            ContainerSidebarEvent::DiscoverEngines => {
                self.discover_engines(&sidebar, cx);
            }
            ContainerSidebarEvent::AddDiscoveredEngine(profile) => {
                self.add_discovered_engine((**profile).clone(), &sidebar, window, cx);
            }
            ContainerSidebarEvent::OpenSessionWorkspace(session_id) => {
                self.open_container_workspace_for_session(*session_id, window, cx);
            }
            ContainerSidebarEvent::ShowToast { message, success } => {
                self.show_toast(message.clone(), *success, window, cx);
            }
        }
    }

    /// 折叠/展开引擎分组并持久化。
    fn toggle_engine_folder(&mut self, folder_id: transom_core::FolderId, cx: &mut Context<Self>) {
        let collapsed = self
            .state
            .catalog
            .engine_folder(folder_id)
            .map(|folder| !folder.collapsed);
        log::debug!("切换引擎分组折叠，分组: {folder_id}，折叠后: {collapsed:?}");
        if let Some(collapsed) = collapsed {
            let _ = self
                .state
                .catalog
                .set_engine_folder_collapsed(folder_id, collapsed);
            self.state.persist_catalog();
        }
        cx.notify();
    }

    /// 提交分组名称：新建或重命名，错误就地回写侧栏表单。
    fn submit_engine_folder_form(
        &mut self,
        parent: Option<transom_core::FolderId>,
        editing: Option<transom_core::FolderId>,
        name: String,
        sidebar: &Entity<ContainerSidebarView>,
        cx: &mut Context<Self>,
    ) {
        log::debug!("提交引擎分组表单，编辑: {editing:?}，名称: {name}");
        let result = match editing {
            Some(id) => self.state.catalog.rename_engine_folder(id, &name),
            None => self
                .state
                .catalog
                .add_engine_folder(&name, parent)
                .map(|_| ()),
        };
        match result {
            Ok(()) => {
                sidebar.update(cx, |sidebar, cx| sidebar.finish_folder_form(cx));
                self.state.persist_catalog();
            }
            Err(error) => {
                sidebar.update(cx, |sidebar, cx| {
                    sidebar.set_folder_form_error(error.to_string(), cx)
                });
            }
        }
    }

    /// 删除分组；其中的连接与子分组提升到父层级。
    fn delete_engine_folder(
        &mut self,
        folder_id: transom_core::FolderId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        log::info!("删除引擎分组，分组: {folder_id}");
        match self.state.catalog.delete_engine_folder(folder_id) {
            Ok(name) => {
                self.state.persist_catalog();
                self.show_toast(
                    rust_i18n::t!("container.sidebar.folder_deleted", name = name).to_string(),
                    true,
                    window,
                    cx,
                );
            }
            Err(error) => self.state.set_error(error.to_string()),
        }
        cx.notify();
    }

    /// 处理引擎树拖放：换分组或同级排序。
    fn handle_container_sidebar_drop(
        &mut self,
        drag: ContainerSidebarDrag,
        target: ContainerSidebarTarget,
        cx: &mut Context<Self>,
    ) {
        log::info!("引擎侧栏拖放，载荷: {:?}，落点: {:?}", drag, target);
        match self.apply_container_sidebar_drop(drag, target) {
            Ok(landed) => {
                // 目标分组被折叠时自动展开，保证拖进去的条目可见。
                if let Some(folder_id) = landed
                    && let Some(folder) = self
                        .state
                        .catalog
                        .engine_folders
                        .iter_mut()
                        .find(|folder| folder.id == folder_id)
                {
                    folder.collapsed = false;
                }
                self.state.persist_catalog();
            }
            Err(error) => self.state.set_error(error.to_string()),
        }
        cx.notify();
    }

    /// 把拖放映射为目录写入；返回落点分组，供调用方展开。
    fn apply_container_sidebar_drop(
        &mut self,
        drag: ContainerSidebarDrag,
        target: ContainerSidebarTarget,
    ) -> Result<Option<transom_core::FolderId>, CatalogError> {
        match (drag, target) {
            (ContainerSidebarDrag::Engine(profile_id), ContainerSidebarTarget::Root) => {
                self.state.catalog.move_engine(profile_id, None)?;
                Ok(None)
            }
            (ContainerSidebarDrag::Folder(folder_id), ContainerSidebarTarget::Root) => {
                self.state.catalog.move_engine_folder(folder_id, None)?;
                Ok(None)
            }
            (
                ContainerSidebarDrag::Engine(profile_id),
                ContainerSidebarTarget::Folder(folder_id),
            ) => {
                self.state
                    .catalog
                    .move_engine(profile_id, Some(folder_id))?;
                Ok(Some(folder_id))
            }
            (ContainerSidebarDrag::Folder(moved), ContainerSidebarTarget::Folder(parent)) => {
                if moved == parent {
                    return Ok(Some(parent));
                }
                self.state.catalog.move_engine_folder(moved, Some(parent))?;
                Ok(Some(parent))
            }
            (drag, ContainerSidebarTarget::Relative { anchor, side }) => match (drag, anchor) {
                (
                    ContainerSidebarDrag::Engine(profile_id),
                    ContainerSidebarDrag::Engine(anchor_id),
                ) => {
                    self.state
                        .catalog
                        .reorder_engine(profile_id, anchor_id, side)?;
                    Ok(self
                        .state
                        .catalog
                        .engines
                        .iter()
                        .find(|profile| profile.id == anchor_id)
                        .and_then(|profile| profile.folder_id))
                }
                (
                    ContainerSidebarDrag::Folder(folder_id),
                    ContainerSidebarDrag::Folder(anchor_id),
                ) => {
                    self.state
                        .catalog
                        .reorder_engine_folder(folder_id, anchor_id, side)?;
                    Ok(self
                        .state
                        .catalog
                        .engine_folder(folder_id)
                        .and_then(|folder| folder.parent))
                }
                (
                    ContainerSidebarDrag::Engine(profile_id),
                    ContainerSidebarDrag::Folder(anchor_id),
                ) => {
                    self.state
                        .catalog
                        .move_engine(profile_id, Some(anchor_id))?;
                    Ok(Some(anchor_id))
                }
                (
                    ContainerSidebarDrag::Folder(folder_id),
                    ContainerSidebarDrag::Engine(anchor_id),
                ) => {
                    let parent = self
                        .state
                        .catalog
                        .engines
                        .iter()
                        .find(|profile| profile.id == anchor_id)
                        .and_then(|profile| profile.folder_id);
                    self.state.catalog.move_engine_folder(folder_id, parent)?;
                    Ok(parent)
                }
            },
        }
    }

    /// 发起一次本机引擎发现。
    fn discover_engines(&mut self, sidebar: &Entity<ContainerSidebarView>, cx: &mut Context<Self>) {
        if sidebar.read(cx).state.discovering {
            return;
        }
        sidebar.update(cx, |sidebar, cx| sidebar.begin_discovery(cx));
        self.state.send_command(ContainerCommand::Discover {
            request_id: ContainerRequestId::new(),
        });
        cx.notify();
    }

    /// 添加一个发现的引擎：同种类同端点已保存时不再重复入库，直接打开工作区。
    fn add_discovered_engine(
        &mut self,
        profile: transom_core::ContainerEngineProfile,
        sidebar: &Entity<ContainerSidebarView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let duplicated = self.state.catalog.engines.iter().any(|saved| {
            saved.engine.kind == profile.engine.kind
                && saved.engine.endpoint.describe() == profile.engine.endpoint.describe()
        });
        if !duplicated {
            self.state.catalog.engines.push(profile.clone());
            self.state.persist_catalog();
        }
        let endpoint = profile.engine.endpoint.describe();
        sidebar.update(cx, |sidebar, cx| {
            sidebar.remove_discovery_candidate(&endpoint, cx)
        });
        self.open_engine_workspace(profile, window, cx);
    }
}
