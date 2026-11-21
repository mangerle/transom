//! 面板发给外壳的意图事件。
//!
//! 面板不持有外壳句柄：需要外壳做事（开弹窗、投递命令、改布局）时发事件，
//! 外壳在创建面板时订阅一次。事件必须自带全部数据，外壳的处理函数不得回读面板，
//! 否则会与正在派发事件的实体形成双重租借。

use gpui_kit::Entity;
use gpui_kit::component::input::{InputState, TextareaState};
use std::path::PathBuf;
use transom_core::{ConnectionId, RemoteDeleteTarget, SftpCommand, TabId};

/// SFTP 目录快捷动作的目标路径。
///
/// 本地路径保留 `PathBuf` 以支持平台原生路径；远端路径保持协议层给出的字符串，
/// 两者都不在这里转换成展示文本，避免在菜单构建阶段丢失非 UTF-8 信息。
#[derive(Clone, Debug)]
pub(crate) enum SftpTerminalDirectoryPath {
    Local(PathBuf),
    Remote(String),
}

/// SFTP 面板向外壳发出的意图。
#[derive(Clone)]
pub(crate) enum SftpPanelEvent {
    /// 请求向后台投递一条 SFTP 命令；命令已构造完整，外壳直接转发。
    SendCommand(SftpCommand),
    /// 请求打开 Chmod 弹窗；输入框已由面板建好并聚焦。
    OpenChmodModal {
        tab_id: TabId,
        connection_id: Option<ConnectionId>,
        remote_path: String,
        file_name: String,
        permissions: u32,
        octal_input: Entity<InputState>,
    },
    /// 请求打开远端检索弹窗；输入框已由面板建好并聚焦。
    OpenSearchModal(Box<crate::sftp::SftpSearchModalState>),
    /// 请求打开目录同步弹窗；扫描状态已由面板构造。
    OpenSyncModal(Box<crate::sftp::SftpSyncModalState>),
    /// 请求打开传输排除规则弹窗；文本框已由面板建好并聚焦。
    OpenFilterModal { textarea: Entity<TextareaState> },
    /// 请求打开远端编辑面板；需要窗口，由外壳在渲染期统一执行。
    OpenEditor(crate::editor::RemoteEditorOpen),
    /// 请求打开输入弹窗；需要窗口，由外壳在渲染期统一执行。
    OpenInputModal(crate::sftp::SftpInputModalParams),
    /// 请求把面板加宽；需要窗口，由外壳在渲染期统一执行。
    GrowPanelWidth(f32),
    /// 请求把指定已有终端切换到面板目录；不打开新会话。
    ChangeTerminalDirectory {
        connection_id: ConnectionId,
        path: SftpTerminalDirectoryPath,
    },
    /// 请求关闭指定窗格的面板；句柄由外壳回收。
    ClosePanel { tab_id: TabId },
    /// 请求打开本地对比面板；目标已由面板校验完成。
    OpenLocalDiff {
        tab_id: TabId,
        target: crate::diff_viewer::LocalDiffTarget,
    },
    /// 请求把一条提示写进状态栏。
    SetStatus(String),
    /// 请求展示属性弹窗；属性内容已由面板读取完成。
    ShowProperties {
        scope: String,
        properties: crate::sftp::FileProperties,
    },
    /// 请求持久化设置并回写状态栏；面板改完 Global 设置后发出。
    PersistSettings { status: String },
    /// 请求弹出本地删除确认。
    ConfirmLocalDelete { tab_id: TabId, paths: Vec<PathBuf> },
    /// 请求弹出远端删除确认；`connection_id` 为 `None` 表示主远端列。
    ConfirmRemoteDelete {
        tab_id: TabId,
        connection_id: Option<ConnectionId>,
        targets: Vec<RemoteDeleteTarget>,
    },
    /// 拖拽上传浮层可见性或展开状态变更，通知外壳刷新终端窗格上的浮层。
    OverlayChanged,
}
