//! 云端同步界面：设置表单、活动状态、历史、侧边栏、上传下载与合并回填。
//!
//! 分层说明（Why）：
//! - 端点拓扑进设置文件，WebDAV/S3/Git 凭据与同步口令只进系统钥匙串；
//! - 网络动作交给控制器单飞执行，结果携带触发来源与耗时回流 UI；
//! - 运行状态与历史属于应用生命周期，集中放在 `CloudSyncViewState`，侧边栏
//!   只读取快照，不反向持有外壳；
//! - 常规上传回填合并目录，显式冲突处理才允许强制本地或强制远端；下拉合并
//!   前始终保存本机安全副本。

mod actions;
mod auto_sync;
mod common;
mod events;
mod form;
mod history;
mod pull;
mod scheduler;
mod sidebar;
mod state;

pub(crate) use actions::{
    dispatch_cloud_sync_check, dispatch_cloud_sync_pull, dispatch_cloud_sync_push,
    dispatch_cloud_sync_test, finish_sync_failure, record_sync_history, start_cloud_sync_check,
    start_cloud_sync_force_pull, start_cloud_sync_force_push, start_cloud_sync_pull,
    start_cloud_sync_push, start_cloud_sync_test, start_git_sync, start_git_sync_scheduled,
};
pub(crate) use auto_sync::tick_cloud_sync_auto;
pub(crate) use events::handle_sync_app_event;
pub(crate) use form::{DEFAULT_SYNC_DIRECTORY, SyncFormInputs};
pub(crate) use history::{SyncHistoryWriter, load_history, sync_history_path};
pub(crate) use pull::finish_cloud_sync_pull;
pub(crate) use scheduler::GitSyncScheduler;
pub(crate) use sidebar::{render_cloud_sync_sidebar, toggle_cloud_sync_sidebar};
pub(crate) use state::CloudSyncViewState;
