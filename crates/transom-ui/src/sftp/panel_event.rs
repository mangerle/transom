//! 面板发给外壳的意图事件。
//!
//! 面板不持有外壳句柄：需要外壳做事（开弹窗、投递命令、改布局）时发事件，
//! 外壳在创建面板时订阅一次。事件必须自带全部数据，外壳的处理函数不得回读面板，
//! 否则会与正在派发事件的实体形成双重租借。

use gpui_kit::Entity;
use gpui_kit::component::input::{InputState, TextareaState};
use std::path::PathBuf;
use transom_core::{ConnectionId, RemoteDeleteTarget, SftpCommand, TabId};

/// SFTP 面板向外壳发出的意图。
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
}
