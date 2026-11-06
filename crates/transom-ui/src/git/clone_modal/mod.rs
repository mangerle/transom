//! 「克隆仓库」弹窗：从远端拉取一个还不存在的仓库。
//!
//! 设计原理（Why）：
//! - 弹窗只负责填参数：点开始立即关闭，进度与取消在右下角进度卡；成功后自动
//!   打开仓库，失败在卡里显示原因，可关掉重开弹窗改参数。原先的标签页要用户
//!   自己再关一次页，弹窗等进度又把人锁在窗口上，两种都不合适。
//! - 目录沿用应用内目录树（与「打开本机仓库」同一套列举/缓存/草稿行）：
//!   文件夹按钮展开它，不弹系统文件对话框，交互与另一个添加入口一致。
//! - 分组保留：克隆成功后直接归入用户选的分组。
//! - 浅克隆深度显式给出：`--depth` 是用户能感知的历史裁剪，不能只藏在后端。
//! - URL 原样交给系统 git：`ssh://`、`git@host:path` 与 `http(s)://` 都由 git
//!   自己解析，界面不拼接、不改写协议或端口；只做首尾空白裁剪。
//! - 输入 URL 时自动把仓库名追加到目标目录：用户在目录框里选的是父目录，
//!   最终落盘位置是「父目录/仓库名」；用户手改过目录后不再覆盖。
//! - 成功后延后一帧再开仓库标签：完成事件落地时没有 Window，而开标签、激活
//!   分组都需要它；`cx.defer_in` 把动作排到绘制之后，避免渲染中改交互状态。
//!
//! 代价与局限：
//! - 左侧没有 GitHub / GitHub Enterprise 账号栏：本应用尚无账号授权能力，
//!   只保留 URL 克隆；接账号是另一条独立的功能线。
//! - 输出按行累积显示，不做"猜原因"的加工：git 的 fatal 文案就是答案。

pub(crate) mod calc;
pub(crate) mod task;
pub(crate) mod view;

pub(crate) use calc::*;
pub(crate) use task::*;

use super::dir_browser::{DirPicker, GitHost};
use crate::app::AppShell;
use crate::keys::expand_home;
use gpui_kit::component::input::InputState;
use gpui_kit::*;
use transom_core::{GitCommand, GitRequestId, SessionColor, SessionIcon};

/// 图标行与分组字段共用的宽度：10 个 small 图标按钮加间距的实测宽度。
///
/// 分组的输入下拉不能比这一行更长，否则左右边界会和上方图标错开。
pub(crate) const CONTROL_COLUMN_WIDTH: f32 = 240.0;

/// 克隆弹窗的全部状态；随弹窗关闭一起释放。
pub(crate) struct GitCloneModal {
    /// 目标目录选择面板：路径框 + 目录树 + 一句错误。
    pub picker: DirPicker,
    /// 远端地址。
    pub url: Entity<InputState>,
    /// 克隆完登记到哪个分组（可输入新组名，也可从既有分组里选）。
    pub folder: super::form::GitFolderField,
    /// 登记时的专属图标。
    pub icon: SessionIcon,
    /// 登记时的环境色。
    pub color: SessionColor,
    /// 是否做浅克隆。
    pub depth_enabled: bool,
    /// 浅克隆深度输入框，默认 1。
    pub depth: Entity<InputState>,
    /// 目录树是否展开。
    pub tree_open: bool,
    /// 校验失败原因（字段未填、深度非法）；克隆过程不在这里显示。
    pub error: Option<String>,
    /// 上一次渲染看到的 URL：用于在地址变化时自动补目录名。
    pub(crate) last_url: String,
    /// 上一次自动追加到目录末尾的仓库名：地址再次变化时先摘掉它再加新的。
    pub(crate) auto_repo: Option<String>,
}

impl AppShell {
    /// 打开「克隆仓库」弹窗。
    pub(crate) fn open_git_clone_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dest_placeholder = rust_i18n::t!("git.clone.dest_placeholder").to_string();
        let folder_placeholder = rust_i18n::t!("git.project.folder_placeholder").to_string();
        let depth = cx.new(|cx| InputState::new(window, cx).placeholder("1"));
        depth.update(cx, |state, cx| state.set_value("1", window, cx));
        let url = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("git.clone.url_placeholder").to_string())
        });
        cx.subscribe_in(
            &url,
            window,
            |this, _, event: &gpui_kit::component::input::InputEvent, window, cx| {
                if matches!(event, gpui_kit::component::input::InputEvent::Change) {
                    this.sync_clone_auto_dest(window, cx);
                }
            },
        )
        .detach();
        self.state.modals.git.clone = Some(GitCloneModal {
            picker: DirPicker::new(window, cx, &dest_placeholder),
            url,
            folder: super::form::GitFolderField::new(window, cx, &folder_placeholder),
            icon: SessionIcon::default(),
            color: SessionColor::default(),
            depth_enabled: false,
            depth,
            tree_open: false,
            error: None,
            last_url: String::new(),
            auto_repo: None,
        });
        // 开窗先把路径框落到主目录：用户不用先点一下才知道起点在哪。
        if let Some(root) = self
            .state
            .modals
            .git
            .clone
            .as_ref()
            .map(|modal| modal.picker.browser.root.clone())
        {
            self.git_browse_root(GitHost::Clone, root, window, cx);
        }
        cx.notify();
    }

    /// 当 URL 变更时自动推导目标目录并写入路径输入框。
    pub(crate) fn sync_clone_auto_dest(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(modal) = self.state.modals.git.clone.as_mut() else {
            return;
        };
        let url_value = modal.url.read(cx).value().trim().to_string();
        if url_value == modal.last_url {
            return;
        }
        let Some(repo) = calc::repo_name_from_url(&url_value) else {
            modal.last_url = url_value;
            return;
        };
        let current = modal.picker.path.read(cx).value().trim().to_string();
        let fallback = modal.picker.browser.root.display().to_string();
        let next = calc::clone_destination(&current, modal.auto_repo.as_deref(), &repo, &fallback);
        modal.last_url = url_value;
        modal.auto_repo = Some(repo);
        if next != current {
            modal
                .picker
                .path
                .update(cx, |input, cx| input.set_value(next, window, cx));
        }
    }

    /// 关闭弹窗。克隆一旦开始就由进度卡接管，这里不取消在途任务。
    pub(crate) fn close_git_clone_modal(&mut self, cx: &mut Context<Self>) {
        self.state.modals.git.clone = None;
        cx.notify();
    }

    /// 开始克隆：校验输入后关弹窗，任务交给右下角进度卡。
    pub(crate) fn start_git_clone(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        // 一次只跑一个克隆：覆盖任务表会让旧子进程失去取消入口。
        if self.state.git.clone_task.is_some() {
            if let Some(modal) = self.state.modals.git.clone.as_mut() {
                modal.error = Some(rust_i18n::t!("git.clone.busy").to_string());
            }
            cx.notify();
            return;
        }
        let Some(modal) = self.state.modals.git.clone.as_ref() else {
            return;
        };
        let url = modal.url.read(cx).value().trim().to_string();
        let raw_dest = modal.picker.path.read(cx).value().trim().to_string();
        if url.is_empty() || raw_dest.is_empty() {
            if let Some(modal) = self.state.modals.git.clone.as_mut() {
                modal.error = Some(rust_i18n::t!("git.clone.need_fields").to_string());
            }
            cx.notify();
            return;
        }
        let depth = if modal.depth_enabled {
            match modal.depth.read(cx).value().trim().parse::<u32>() {
                Ok(value) if value > 0 => Some(value),
                _ => {
                    if let Some(modal) = self.state.modals.git.clone.as_mut() {
                        modal.error = Some(rust_i18n::t!("git.clone.depth_invalid").to_string());
                    }
                    cx.notify();
                    return;
                }
            }
        } else {
            None
        };
        let dest = expand_home(&raw_dest);
        let repo_name = repo_name_from_url(&url).unwrap_or_else(|| {
            dest.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| rust_i18n::t!("git.clone.title").to_string())
        });
        let folder_name = modal.folder.name.read(cx).value().trim().to_string();
        let (icon, color) = (modal.icon, modal.color);
        let request_id = GitRequestId::new();
        self.state.git.clone_task = Some(GitCloneTask {
            request_id,
            repo_name,
            folder_name,
            icon,
            color,
            last_line: String::new(),
            percent: None,
            running: true,
            error: None,
            done: None,
        });
        // 弹窗立即关闭：进度、取消都在右下角卡片上，用户可以去干别的。
        self.state.modals.git.clone = None;
        // URL 原样交给系统 git：ssh://、git@host:path 与 http(s):// 都由 git
        // 自己解析；这里只裁剪首尾空白，不拼接、不改写协议或端口。
        self.send_command(GitCommand::Clone {
            request_id,
            url,
            dest,
            depth,
        });
        cx.notify();
    }

    /// 目录树里换父目录后，把当前 URL 的仓库名继续补在后面。
    ///
    /// 树里的选中高亮仍落在用户点的目录上；写回路径框的是「父目录/仓库名」，
    /// 这样切换父目录不会丢掉仓库名。URL 没有仓库名时原样返回。
    pub(crate) fn git_clone_selected_input(
        &mut self,
        path: std::path::PathBuf,
        cx: &Context<Self>,
    ) -> std::path::PathBuf {
        let Some(modal) = self.state.modals.git.clone.as_mut() else {
            return path;
        };
        let url = modal.url.read(cx).value().trim().to_string();
        let Some(repo) = repo_name_from_url(&url) else {
            return path;
        };
        modal.auto_repo = Some(repo.clone());
        path.join(repo)
    }
}
