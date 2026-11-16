//! Git 工作区实体：一个 Git 标签页的状态、身份与命令投递端。
//!
//! 设计原理（Why）：Git 工作区的生命周期就是标签页的生命周期，状态必须随标签页
//! 一起创建与销毁，动作也只能落在它自己的状态上。把状态收进
//! `Entity<GitWorkspace>` 后，外壳只持有句柄，读写都经 `read` / `update`：同一
//! 标签页不会出现两份状态，也不会再出现“借用标签页状态的同时又调用外壳方法”的
//! 借用冲突。
//!
//! 代价与局限：读写都必须携带 GPUI 上下文；渲染期只允许 `read`，写动作必须在
//! `update` 闭包内完成，且闭包里不能再反查同一实体（会触发双重租借崩溃）。

use std::collections::HashMap;

use super::GitNamePurpose;
use super::types::GitWorkspaceState;
use gpui_kit::{ClipboardItem, Context, EventEmitter, IntoElement, Render, Task, Window, div};
use transom_core::{AppCommand, GitCommand, GitProject, GitRequestId, TabId};

/// Git 工作区实体向应用外壳上报的跨域意图事件。
///
/// 设计原理（Why）：工作区实体只负责自身标签页内的状态流转与命令投递，
/// 不持有 `WeakEntity<AppShell>`；需要弹窗、终端标签、设置页或状态栏提示等
/// 跨标签/窗口级协作时，通过事件上报由外壳统一消费。
#[derive(Clone)]
pub(crate) enum GitWorkspaceEvent {
    /// 底栏一次性状态提示。
    TransientStatus(String),
    /// 应用级错误提示。
    Error(String),
    /// 请求打开推送弹窗。
    OpenPushModal {
        project: GitProject,
        target: Option<String>,
    },
    /// 请求打开命名弹窗（新建分支/重命名分支/打标签）。
    OpenNameModal {
        project: GitProject,
        purpose: GitNamePurpose,
        initial: String,
    },
    /// 请求打开新建工作树弹窗。
    OpenWorktreeModal { project: GitProject, start: String },
    /// 请求打开交互式变基弹窗。
    OpenRebaseModal {
        project: GitProject,
        upstream: String,
    },
    /// 请求打开冲突解决弹窗。
    OpenConflictModal(GitProject),
    /// 请求在独立标签页或侧栏打开工作树路径。
    OpenWorktreePath(String),
    /// 触发项目初始化并让外壳登记全局 `pending_project_init`。
    ProjectInitStarted {
        request_id: GitRequestId,
        project: GitProject,
    },
    /// 请求打开设置弹窗并定位到 Git 页。
    OpenSettings,
    /// 请求持久化差异显示等设置快照。
    PersistSettings(Box<transom_core::AppSettings>),
}

impl EventEmitter<GitWorkspaceEvent> for GitWorkspace {}

/// 日志单页条数：一屏约 40 行，5 页滚动内不必再发请求。
const LOG_PAGE_SIZE: usize = 200;

/// 一个 Git 标签页的实体。
pub(crate) struct GitWorkspace {
    /// 标签页的全部界面状态。
    pub(crate) state: GitWorkspaceState,
    /// 所属标签页 id：事件回传与外壳定位都用它，不必再按项目反查。
    pub(crate) tab_id: TabId,
    /// 控制器命令投递端：实体自己就能取数据，不需要外壳转手。
    pub(crate) command_tx: flume::Sender<AppCommand>,
    /// 后台差异准备任务容器：按请求 ID 纳管，替换或关闭标签时自动取消。
    pub(crate) diff_tasks: HashMap<GitRequestId, Task<()>>,
}

impl GitWorkspace {
    /// 应用仓库补初始化结果。
    ///
    /// 失败时把原因同时落到状态栏与日志区，并结束加载态；成功时清掉错误与
    /// 未初始化标记，交由调用方继续触发刷新。这里只改本实体状态，不读外壳。
    pub(crate) fn apply_project_init_result(&mut self, error: Option<&str>) {
        let state = &mut self.state;
        state.init_running = false;
        state.not_repo = error.is_some();
        state.status_error = error.map(str::to_string);
        state.log.loading = false;
        state.log.error = error.map(str::to_string);
        if error.is_some() {
            state.status_loading = false;
        }
    }
    /// 组装一个 Git 工作区实体。
    pub(crate) fn new(
        state: GitWorkspaceState,
        tab_id: TabId,
        command_tx: flume::Sender<AppCommand>,
    ) -> Self {
        Self {
            state,
            tab_id,
            command_tx,
            diff_tasks: HashMap::new(),
        }
    }

    /// 向控制器投递命令：非阻塞 `try_send`，失败只记 warn 日志并返回 `false`。
    ///
    /// 设计原理（Why）：命令队列满时快速失败，不阻塞 UI 线程，也不 panic。
    /// 这里刻意不写应用级错误队列——命令失败的结果由事件层统一回传，实体再报
    /// 一次会让同一故障出现两条通知。
    pub(crate) fn send(&self, command: impl Into<AppCommand>) -> bool {
        match self.command_tx.try_send(command.into()) {
            Ok(()) => true,
            Err(error) => {
                log::warn!(
                    "Git 工作区投递命令失败，标签: {}，原因: {error}",
                    self.tab_id
                );
                false
            }
        }
    }

    /// 分页加载提交历史；`skip` 为 0 时整页替换，否则追加（下拉加载）。
    ///
    /// 设计原理（Why）：请求登记、加载态与查询快照是一组必须原子的状态改动，
    /// 放在实体上后调用方只需给出页码，不必再重复一遍“先置 loading 再发命令”
    /// 的顺序；命令走实体自己的 `command_tx`，不依赖外壳。
    ///
    /// 返回 `false` 表示命令队列已满或控制平面已退出，调用方负责把失败
    /// 转成应用级提示；实体自身只留日志，避免同一故障出现两条通知。
    pub(crate) fn load_log(&mut self, skip: usize) -> bool {
        self.state.log.loading = true;
        self.state.log.error = None;
        let request_id = GitRequestId::new();
        self.state.pending.push(request_id);
        self.state.log.request = Some((request_id, skip > 0));
        let mut query = self.state.query.clone();
        query.skip = skip;
        query.limit = LOG_PAGE_SIZE;
        self.send(GitCommand::LoadLog {
            project: self.state.project.clone(),
            request_id,
            query,
        })
    }

    /// 刷新当前仓库的工作区状态与引用列表。
    pub(crate) fn refresh(&mut self) {
        self.state.status_loading = true;
        self.state.status_error = None;
        let project = self.state.project.clone();
        self.send(GitCommand::Refresh {
            project: project.clone(),
        });
        self.send(GitCommand::LoadRefs { project });
    }

    /// 取消当前标签页登记的所有在途 Git 请求。
    pub(crate) fn cancel_pending(&self) {
        for &request_id in &self.state.pending {
            self.send(GitCommand::Cancel { request_id });
        }
    }

    /// 消费提交成功后的延迟清空标志（需要在持有 `Window` 的渲染帧入口调用）。
    pub(crate) fn clear_pending_commit_message(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.state.commit_clear_pending {
            return;
        }
        self.state.commit_clear_pending = false;
        let commit_message = self.state.commit_message.clone();
        commit_message.update(cx, |input, cx| input.set_value("", window, cx));
    }

    /// 切换引用栏展开状态。
    pub(crate) fn toggle_refs_panel(&mut self, cx: &mut Context<Self>) {
        self.state.refs_panel = !self.state.refs_panel;
        cx.notify();
    }

    /// 为尚未初始化的目录执行 `git init`。
    pub(crate) fn init_project(&mut self, cx: &mut Context<Self>) {
        if self.state.init_running {
            return;
        }
        let request_id = GitRequestId::new();
        self.state.init_running = true;
        self.state.status_error = None;
        let project = self.state.project.clone();
        if self.send(GitCommand::Init {
            request_id,
            path: project.path.clone(),
        }) {
            cx.emit(GitWorkspaceEvent::ProjectInitStarted {
                request_id,
                project,
            });
        } else {
            self.state.init_running = false;
            cx.emit(GitWorkspaceEvent::Error(
                rust_i18n::t!("error.command_queue_full").to_string(),
            ));
        }
        cx.notify();
    }

    /// 勾选/取消「修正」：勾上时回填目标提交信息并读取其文件清单。
    pub(crate) fn toggle_commit_amend(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let next = !self.state.commit_amend;
        self.state.commit_amend = next;
        if !next {
            self.state.amend_files_loading = false;
            cx.notify();
            return;
        }
        let entry = self
            .state
            .amend_target
            .as_ref()
            .and_then(|oid| self.state.log.entries.iter().find(|e| &e.oid == oid))
            .or_else(|| self.state.log.entries.first())
            .cloned();
        let Some(entry) = entry else {
            cx.notify();
            return;
        };
        let mut message = entry.subject.clone();
        let body = entry.body.trim();
        if !body.is_empty() {
            message.push_str("\n\n");
            message.push_str(body);
        }
        let request_id = GitRequestId::new();
        self.state.amend_request = Some(request_id);
        self.state.amend_files_loading = true;
        self.state.amend_files.clear();
        self.state.pending.push(request_id);
        let commit_message = self.state.commit_message.clone();
        let project = self.state.project.clone();
        commit_message.update(cx, |input, cx| input.set_value(message, window, cx));
        self.send(GitCommand::LoadCommitFiles {
            project,
            request_id,
            oid: entry.oid,
        });
        cx.notify();
    }

    /// 切换底部 Git 提交记录抽屉面板的展开/收起状态。
    pub(crate) fn toggle_bottom_log(&mut self, cx: &mut Context<Self>) {
        self.state.bottom_log_open = !self.state.bottom_log_open;
        if self.state.bottom_log_open {
            self.refresh();
            self.load_log(0);
        }
        cx.notify();
    }

    /// 提交当前输入的消息（支持 Amend 与提交后自动弹起推送弹窗）。
    pub(crate) fn submit_commit(&mut self, push_after: bool, cx: &mut Context<Self>) {
        let message = self.state.commit_message.read(cx).value().to_string();
        let amend = self.state.commit_amend;
        if message.trim().is_empty() && !amend {
            cx.emit(GitWorkspaceEvent::Error(
                rust_i18n::t!("git.commit.empty_message").to_string(),
            ));
            return;
        }
        let fixup_target = if amend {
            self.state.amend_target.as_ref().and_then(|target| {
                let head_oid = self.state.log.entries.first().map(|e| &e.oid);
                (head_oid != Some(target)).then(|| target.clone())
            })
        } else {
            None
        };
        self.state.commit_submitting = true;
        self.state.checked.clear();
        if self.send(GitCommand::Commit {
            project: self.state.project.clone(),
            message,
            amend,
            fixup_target,
        }) {
            if push_after {
                self.state.push_after_commit = true;
            }
        } else {
            self.state.commit_submitting = false;
            cx.emit(GitWorkspaceEvent::Error(
                rust_i18n::t!("error.command_queue_full").to_string(),
            ));
        }
        cx.notify();
    }

    /// 复制文本到系统剪贴板并给出底栏提示。
    pub(crate) fn copy_to_clipboard(&mut self, text: &str, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
        cx.emit(GitWorkspaceEvent::TransientStatus(
            rust_i18n::t!("git.menu.copied").to_string(),
        ));
        cx.notify();
    }

    /// 丢弃或删除单个路径的工作区/未跟踪改动。
    pub(crate) fn rollback_single_path(&mut self, path: &str, cx: &mut Context<Self>) {
        let project = self.state.project.clone();
        self.send(GitCommand::Discard {
            project,
            paths: vec![path.to_string()],
        });
        cx.notify();
    }

    /// 请求打开 Git 设置页。
    pub(crate) fn open_settings(&mut self, cx: &mut Context<Self>) {
        cx.emit(GitWorkspaceEvent::OpenSettings);
    }
}

impl Render for GitWorkspace {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}
