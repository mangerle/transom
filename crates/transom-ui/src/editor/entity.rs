//! 远端文本编辑面板实体：拥有面板集合状态与全部渲染，跨域动作以事件上报外壳。
//!
//! # 设计原理（Why）
//! - 编辑面板是窗口级浮层，生命周期与窗口一致，因此独立为
//!   `Entity<EditorPanels>`。折叠、最大化、拖拽与读写阶段变化只通知本实体，
//!   外壳不再因为一次光标移动或一次错误横幅刷新而重建整棵树；
//! - 需要发送 SFTP 命令、打开口令弹窗、关闭标签组、跳转到行等动作时，面板
//!   以 [`EditorPanelEvent`] 上报意图，事件自带数据；外壳在创建时订阅一次，
//!   处理阶段不回读本实体。
//!
//! # 调用约束
//! 实体回调执行期间本实体已被 GPUI 租借。外壳处理事件时**不得**再
//! `read`/`update` 本实体，否则触发 “cannot update while it is already being
//! updated” 崩溃；需要窗口的动作排进 `pending_editor_requests`，由渲染期统一
//! 执行，状态变更则留在本实体内先完成。

use super::close::PendingEditorClose;
use super::manager::EditorManager;
use super::panel::{EditorPanelState, EditorPhase};
use crate::app::AppShell;
use crate::app::app_settings;
use gpui_kit::component::input::EditorState;
use gpui_kit::*;
use transom_core::{RemoteTextMeta, TabId};

/// 编辑面板向外壳发出的意图。
///
/// 面板不持有外壳句柄后，需要窗口的动作与跨域动作都从这里交给外壳；
/// 事件自带数据，外壳处理时不得回读面板。
#[derive(Clone)]
pub(crate) enum EditorPanelEvent {
    /// 请打开"跳转到行"输入框。
    OpenGotoLine(Entity<EditorState>),
    /// 请把焦点还给 SFTP 面板。
    FocusSftpPanel,
    /// 请在下一帧续行挂起的关闭。
    ResumePendingClose,
    /// 请用编辑器当前基线与本地文件比对。
    OpenLocalDiff(EditorDiffSource),
    /// 编辑器列表显隐变化；外壳工具栏选中态需要重绘。
    ListOpenChanged,
}

/// 外壳发起"与本地对比"时需要的远端快照；由实体只读产出。
#[derive(Clone)]
pub(crate) struct EditorDiffSource {
    pub(crate) tab_id: TabId,
    pub(crate) remote_path: String,
    pub(crate) file_name: String,
    pub(crate) remote_text: String,
    pub(crate) remote_meta: Option<RemoteTextMeta>,
}

/// 外壳尚未绑定时读取设置的兜底值；正常路径永远从设置快照取值。
const FALLBACK_EDITOR_FONT_SIZE: f32 = 14.0;
const FALLBACK_EDIT_MAX_BYTES: u64 = 8 * 1024 * 1024;

/// 远端文本编辑面板的窗口级实体。
pub(crate) struct EditorPanels {
    /// 面板集合、待关闭意图与列表浮层显隐等纯状态。
    manager: EditorManager,
    /// 命令通道：面板自己投递读写命令，保留同步返回值语义。
    command_tx: Option<flume::Sender<transom_core::AppCommand>>,
}

impl EventEmitter<EditorPanelEvent> for EditorPanels {}

impl Default for EditorPanels {
    fn default() -> Self {
        Self::new()
    }
}

impl EditorPanels {
    pub(crate) fn new() -> Self {
        Self {
            manager: EditorManager::new(),
            command_tx: None,
        }
    }

    /// 命令通道句柄。
    pub(super) fn command_tx(&self) -> Option<&flume::Sender<transom_core::AppCommand>> {
        self.command_tx.as_ref()
    }

    /// 注入命令通道；创建后调用一次。
    pub(crate) fn set_command_tx(&mut self, tx: flume::Sender<transom_core::AppCommand>) {
        self.command_tx = Some(tx);
    }

    /// 已打开面板数量。
    pub(crate) fn len(&self) -> usize {
        self.manager.len()
    }

    /// 面板管理器只读访问：限于编辑器模块内部使用。
    pub(super) fn manager(&self) -> &EditorManager {
        &self.manager
    }

    /// 面板管理器可变访问：限于编辑器模块内部使用。
    pub(super) fn manager_mut(&mut self) -> &mut EditorManager {
        &mut self.manager
    }

    /// 列表浮层是否展开。
    pub(crate) fn list_open(&self) -> bool {
        self.manager.list_open
    }

    /// 写入列表浮层显隐；外壳工具栏的选中态也要跟着重绘。
    pub(crate) fn set_list_open(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.manager.list_open == open {
            return;
        }
        self.manager.list_open = open;
        cx.notify();
        cx.emit(EditorPanelEvent::ListOpenChanged);
    }

    /// 挂起的关闭意图快照。
    pub(crate) fn pending_close(&self) -> Option<PendingEditorClose> {
        self.manager.pending_close.clone()
    }

    /// 清除挂起的关闭意图。
    pub(crate) fn clear_pending_close(&mut self) {
        self.manager.pending_close = None;
    }

    /// 全部脏面板所属标签快照，供外壳判定挂起关闭是否可续行。
    pub(crate) fn dirty_tabs(&self) -> Vec<TabId> {
        self.manager
            .panels
            .iter()
            .filter(|item| item.dirty)
            .map(|item| item.tab_id)
            .collect()
    }

    /// 取编辑器面板中可参与"与本地对比"的远端快照；未装载、新建文件或读写中
    /// 返回 `None`。
    pub(crate) fn diff_source(&self, editor: &Entity<EditorState>) -> Option<EditorDiffSource> {
        self.manager
            .panels
            .iter()
            .find(|item| item.editor == *editor)
            .filter(|item| item.loaded && !item.new_file && !Self::phase_busy(item.phase))
            .map(|item| EditorDiffSource {
                tab_id: item.tab_id,
                remote_path: item.remote_path.clone(),
                file_name: item.file_name.clone(),
                remote_text: item.baseline.to_string(),
                remote_meta: item.remote_meta,
            })
    }

    /// 清理已关闭标签对应的面板；订阅句柄随面板析构退订。
    pub(crate) fn prune_dead_tabs(&mut self, active_tabs: &[TabId]) {
        self.manager.prune_dead_tabs(active_tabs);
    }

    /// 按编辑视图找面板下标。
    pub(super) fn index_of(&self, editor: &Entity<EditorState>) -> Option<usize> {
        self.manager
            .panels
            .iter()
            .position(|item| item.editor == *editor)
    }

    /// 按编辑视图就地修改面板；返回是否找到。
    pub(super) fn update_panel(
        &mut self,
        editor: &Entity<EditorState>,
        action: impl FnOnce(&mut EditorPanelState),
    ) -> bool {
        let Some(index) = self.index_of(editor) else {
            return false;
        };
        action(&mut self.manager.panels[index]);
        true
    }

    /// 与 [`Self::update_panel`] 同形，但把闭包结果带出借用作用域。
    pub(super) fn map_panel<R>(
        &mut self,
        editor: &Entity<EditorState>,
        action: impl FnOnce(&mut EditorPanelState) -> R,
    ) -> Option<R> {
        let index = self.index_of(editor)?;
        Some(action(&mut self.manager.panels[index]))
    }

    /// 按编辑视图移除面板；订阅句柄随面板析构退订。
    pub(super) fn remove_panel(&mut self, editor: &Entity<EditorState>) {
        self.manager.panels.retain(|item| item.editor != *editor);
    }

    /// 当前应接收“保存”快捷键的面板：优先编辑器持有焦点的那个，
    /// 没有焦点信息时退到最上层，避免快捷键静默失效。
    pub(super) fn focused_editor(&self, window: &Window, cx: &App) -> Option<Entity<EditorState>> {
        let panels = &self.manager.panels;
        if panels.is_empty() {
            return None;
        }
        panels
            .iter()
            .position(|item| {
                item.editor
                    .read(cx)
                    .focus_handle(cx)
                    .contains_focused(window, cx)
            })
            .or(Some(panels.len() - 1))
            .and_then(|index| panels.get(index))
            .map(|item| item.editor.clone())
    }

    /// 面板是否处于拖拽中；窗口级监听以此作廉价闸门。
    pub(crate) fn drag_in_progress(&self) -> bool {
        self.manager.drag_in_progress()
    }

    /// 代码字号独立于界面密度；越界值按边界处理，避免极端值撑坏布局。
    pub(super) fn editor_font_size(&self, cx: &App) -> f32 {
        let configured = app_settings(cx).sftp.editor_font_size;
        let size = if configured > 0.0 {
            configured
        } else {
            FALLBACK_EDITOR_FONT_SIZE
        };
        size.clamp(9.0, 40.0)
    }

    /// 当前配置的编辑读取上限，实体在投递读取命令前读取。
    pub(super) fn edit_max_bytes(&self, cx: &App) -> u64 {
        let configured = app_settings(cx).sftp.edit_max_bytes;
        if configured > 0 {
            configured
        } else {
            FALLBACK_EDIT_MAX_BYTES
        }
    }

    /// 保存中/读取中的面板不允许再次提交；
    pub(super) fn phase_busy(phase: EditorPhase) -> bool {
        matches!(phase, EditorPhase::Reading | EditorPhase::Writing)
    }
}

impl AppShell {
    /// 处理编辑面板发来的意图。
    ///
    /// 需要窗口的动作排到渲染期执行：事件派发阶段没有 `Window`。
    pub(crate) fn on_editor_panel_event(
        &mut self,
        event: &EditorPanelEvent,
        cx: &mut Context<Self>,
    ) {
        self.state.pending_editor_requests.push(event.clone());
        cx.notify();
    }
}
