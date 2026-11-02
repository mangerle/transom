//! 引擎分组的折叠、改名、删除与移动。

use super::*;

impl AppShell {
    /// 折叠/展开引擎分组并持久化。
    pub(crate) fn toggle_engine_folder(&mut self, folder_id: FolderId, cx: &mut Context<Self>) {
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

    /// 打开引擎分组的新建/重命名弹窗。
    pub(crate) fn open_engine_folder_form(
        &mut self,
        parent: Option<FolderId>,
        editing: Option<FolderId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        log::debug!("打开引擎分组表单，父级: {parent:?}，编辑: {editing:?}");
        let name = editing
            .and_then(|id| self.state.catalog.engine_folder(id))
            .map(|folder| folder.name.clone())
            .unwrap_or_default();
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(name)
                .placeholder(rust_i18n::t!("container.sidebar.folder_name").to_string())
        });
        input.update(cx, |state, cx| state.focus(window, cx));
        self.state.containers.engine_folder_form = Some(EngineFolderForm {
            parent,
            editing,
            input,
            error: None,
        });
        cx.notify();
    }

    pub(crate) fn dismiss_engine_folder_form(&mut self, cx: &mut Context<Self>) {
        self.state.containers.engine_folder_form = None;
        cx.notify();
    }

    /// 提交分组名称：新建或重命名，错误就地显示。
    pub(crate) fn submit_engine_folder_form(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(form) = self.state.containers.engine_folder_form.as_ref() else {
            return;
        };
        let name = form.input.read(cx).value().trim().to_string();
        let parent = form.parent;
        let editing = form.editing;
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
                self.state.containers.engine_folder_form = None;
                self.state.persist_catalog();
            }
            Err(error) => {
                if let Some(form) = self.state.containers.engine_folder_form.as_mut() {
                    form.error = Some(error.to_string());
                }
            }
        }
        cx.notify();
    }

    /// 删除分组；其中的连接与子分组提升到父层级。
    pub(crate) fn delete_engine_folder(
        &mut self,
        folder_id: FolderId,
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

    /// 把引擎连接移入某个分组（`None` 为根层级）。
    pub(crate) fn move_engine_to_folder(
        &mut self,
        profile_id: EngineProfileId,
        folder: Option<FolderId>,
        cx: &mut Context<Self>,
    ) {
        match self.state.catalog.move_engine(profile_id, folder) {
            Ok(()) => {
                self.state.persist_catalog();
            }
            Err(error) => self.state.set_error(error.to_string()),
        }
        cx.notify();
    }
}
