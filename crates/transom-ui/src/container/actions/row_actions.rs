//! 容器列表行操作：在工作区实体内执行本域动作，仅跨实体/窗口动作上交外壳。

use super::super::*;

/// 需要应用外壳协同完成的跨实体行动作。
pub(crate) enum ShellRowAction {
    RunImage(String),
    CloneContainer { id: String, name: String },
}

impl ContainerWorkspace {
    /// 列表行动作分发：行内快捷按钮与右键菜单共用这一套语义，避免出现两处行为。
    ///
    /// 复制类动作不改选中项：用户常常要复制一个不是当前选中项的 ID，改选中会
    /// 顺手清掉右侧正在看的内容。
    pub(crate) fn apply_row_action(
        &mut self,
        target: RowTarget,
        action: RowAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<ShellRowAction> {
        if matches!(action, RowAction::CopyId | RowAction::CopyName) {
            self.copy_row_text(&target, action, cx);
            return None;
        }
        self.select_row(&target, cx);
        let container = match &target {
            RowTarget::Container { name, .. } => Some(name.clone()),
            _ => None,
        };
        match action {
            RowAction::Overview => self.set_detail_tab(ContainerDetailTab::Overview, cx),
            RowAction::Logs => self.set_detail_tab(ContainerDetailTab::Logs, cx),
            RowAction::Terminal => self.set_detail_tab(ContainerDetailTab::Terminal, cx),
            RowAction::Stats => self.set_detail_tab(ContainerDetailTab::Stats, cx),
            RowAction::Files => self.set_detail_tab(ContainerDetailTab::Files, cx),
            RowAction::Start
            | RowAction::Stop
            | RowAction::Restart
            | RowAction::Pause
            | RowAction::Unpause
            | RowAction::Kill => {
                let name = container?;
                let lifecycle = match action {
                    RowAction::Start => ContainerLifecycleAction::Start,
                    RowAction::Stop => ContainerLifecycleAction::Stop,
                    RowAction::Restart => ContainerLifecycleAction::Restart,
                    RowAction::Pause => ContainerLifecycleAction::Pause,
                    RowAction::Unpause => ContainerLifecycleAction::Unpause,
                    RowAction::Kill => ContainerLifecycleAction::Kill { signal: None },
                    _ => ContainerLifecycleAction::Restart,
                };
                self.container_lifecycle(name, lifecycle, cx);
            }
            RowAction::Remove => {
                let confirm = match target {
                    RowTarget::Container { name, .. } => Some(ContainerConfirm::Container(name)),
                    RowTarget::Image(id) => Some(ContainerConfirm::Image(id)),
                    RowTarget::Volume(name) => Some(ContainerConfirm::Volume(name)),
                    RowTarget::Network(id) => Some(ContainerConfirm::Network(id)),
                    RowTarget::Compose(_) => None,
                };
                if let Some(confirm) = confirm {
                    self.state.modals.confirm = Some(confirm);
                    cx.notify();
                }
            }
            RowAction::Run => {
                let RowTarget::Image(id) = &target else {
                    return None;
                };
                let image = self
                    .state
                    .images
                    .iter()
                    .find(|image| &image.id == id)
                    .map(|image| image.display_name())
                    .unwrap_or_else(|| transom_core::short_id(id));
                return Some(ShellRowAction::RunImage(image));
            }
            RowAction::Rename => {
                let RowTarget::Container { id, name } = target else {
                    return None;
                };
                self.open_rename(id, name, window, cx);
            }
            RowAction::ExportConfig => {
                let RowTarget::Container { name, .. } = target else {
                    return None;
                };
                self.open_container_export(name, cx);
            }
            RowAction::Commit => {
                let RowTarget::Container { name, .. } = target else {
                    return None;
                };
                self.open_container_commit(name, window, cx);
            }
            RowAction::Clone => {
                let RowTarget::Container { id, name } = target else {
                    return None;
                };
                return Some(ShellRowAction::CloneContainer { id, name });
            }
            RowAction::Tag => {
                let RowTarget::Image(id) = target else {
                    return None;
                };
                self.open_image_tag(id, window, cx);
            }
            RowAction::Push => {
                let RowTarget::Image(id) = target else {
                    return None;
                };
                let reference = self
                    .state
                    .images
                    .iter()
                    .find(|item| item.id == id)
                    .map(|item| item.display_name())
                    .unwrap_or_else(|| id.clone());
                self.open_push_dialog(id, reference, window, cx);
            }
            RowAction::Save => {
                let RowTarget::Image(id) = target else {
                    return None;
                };
                self.save_image(id, cx);
            }
            RowAction::History => {
                let RowTarget::Image(id) = target else {
                    return None;
                };
                self.select_image_item(id);
                self.state.detail_tab = ContainerDetailTab::History;
                cx.notify();
            }
            RowAction::EditComposeYaml => {
                let RowTarget::Compose(name) = target else {
                    return None;
                };
                self.edit_compose_yaml(&name, cx);
            }
            RowAction::BackupVolume => {
                let RowTarget::Volume(name) = target else {
                    return None;
                };
                self.backup_volume(name, cx);
            }
            RowAction::RestoreVolume => {
                let RowTarget::Volume(name) = target else {
                    return None;
                };
                self.restore_volume(Some(name), cx);
            }
            RowAction::CopyId | RowAction::CopyName => {}
        }
        None
    }
}
