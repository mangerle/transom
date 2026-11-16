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
//! - 弹窗视图作为独立 GPUI 实体管理内部表单状态与目录树交互，通过 `EventEmitter`
//!   向上抛出克隆启动与关闭意图。
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

use super::dir_browser::{DirPicker, GitDirBrowserEvent};
use super::form::GitFolderField;
use crate::keys::expand_home;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;
use std::path::PathBuf;
use transom_core::{SessionColor, SessionIcon};

/// 图标行与分组字段共用的宽度：10 个 small 图标按钮加间距的实测宽度。
///
/// 分组的输入下拉不能比这一行更长，否则左右边界会和上方图标错开。
pub(crate) const CONTROL_COLUMN_WIDTH: f32 = 240.0;

/// 克隆弹窗对外发出的事件。
#[derive(Debug, Clone)]
pub(crate) enum GitCloneModalEvent {
    /// 参数校验通过，请求启动后台克隆任务。
    Start {
        url: String,
        dest: PathBuf,
        depth: Option<u32>,
        repo_name: String,
        folder_name: String,
        icon: SessionIcon,
        color: SessionColor,
    },
    /// 取消或关闭弹窗。
    Close,
}

/// 克隆弹窗独立实体；随弹窗关闭一起释放。
pub(crate) struct GitCloneModalView {
    /// 目标目录选择面板：路径框 + 目录树 + 一句错误。
    pub picker: Entity<DirPicker>,
    /// 远端地址。
    pub url: Entity<InputState>,
    /// 克隆完登记到哪个分组（可输入新组名，也可从既有分组里选）。
    pub folder: GitFolderField,
    /// 既有分组名候选列表。
    pub folders: Vec<String>,
    /// 登记时的专属图标。
    pub icon: SessionIcon,
    /// 登记时的环境色。
    pub color: SessionColor,
    /// 是否做浅克隆。
    pub depth_enabled: bool,
    /// 浅克隆深度输入（默认 `"1"`）。
    pub depth: Entity<InputState>,
    /// 目录树展开开关：默认折叠，点文件夹图标切换。
    pub tree_open: bool,
    /// 上一次参与推导的 URL，用于识别变化并避免重复覆写。
    pub last_url: String,
    /// 上一次自动追加在目录尾部的仓库名；目录不再以它结尾说明用户手动改过。
    pub auto_repo: Option<String>,
    /// 表单校验错误或冲突提示。
    pub error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<GitCloneModalEvent> for GitCloneModalView {}

impl GitCloneModalView {
    /// 创建克隆弹窗实体并绑定 URL 与目录选择器的联动订阅。
    pub(crate) fn new(folders: Vec<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let picker = cx.new(|cx| DirPicker::new(window, cx, ""));
        let url_placeholder = rust_i18n::t!("git.clone.url_placeholder").to_string();
        let folder_placeholder = rust_i18n::t!("git.project.folder_placeholder").to_string();
        let url = cx.new(|cx| InputState::new(window, cx).placeholder(&url_placeholder));
        let depth = cx.new(|cx| InputState::new(window, cx).default_value("1"));
        let folder = GitFolderField::new(window, cx, &folder_placeholder);

        let url_sub = cx.subscribe_in(&url, window, |this, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::Change) {
                this.sync_auto_dest(window, cx);
            }
        });
        let picker_sub = cx.subscribe_in(
            &picker,
            window,
            |this, _, event: &GitDirBrowserEvent, window, cx| match event {
                GitDirBrowserEvent::PathSelected(path) => {
                    this.apply_selected_parent(path.clone(), window, cx);
                }
                GitDirBrowserEvent::CloseTree => {
                    this.tree_open = false;
                    cx.notify();
                }
            },
        );
        let picker_obs = cx.observe(&picker, |_, _, cx| cx.notify());

        Self {
            picker,
            url,
            folder,
            folders,
            icon: SessionIcon::default(),
            color: SessionColor::default(),
            depth_enabled: false,
            depth,
            tree_open: false,
            last_url: String::new(),
            auto_repo: None,
            error: None,
            _subscriptions: vec![url_sub, picker_sub, picker_obs],
        }
    }

    /// 供焦点管理获取 URL 输入框焦点句柄。
    pub(crate) fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.url.read(cx).focus_handle(cx)
    }

    /// 设置校验或冲突提示文案。
    pub(crate) fn set_error(&mut self, message: String, cx: &mut Context<Self>) {
        self.error = Some(message);
        cx.notify();
    }

    /// 当 URL 变更时自动推导目标目录并写入路径输入框。
    fn sync_auto_dest(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let url_value = self.url.read(cx).value().trim().to_string();
        if url_value == self.last_url {
            return;
        }
        let Some(repo) = calc::repo_name_from_url(&url_value) else {
            self.last_url = url_value;
            return;
        };
        let (current, fallback) = {
            let picker = self.picker.read(cx);
            (
                picker.path.read(cx).value().trim().to_string(),
                picker.browser.root.display().to_string(),
            )
        };
        let next = calc::clone_destination(&current, self.auto_repo.as_deref(), &repo, &fallback);
        self.last_url = url_value;
        self.auto_repo = Some(repo);
        if next != current {
            let path_input = self.picker.read(cx).path.clone();
            path_input.update(cx, |input, cx| input.set_value(next, window, cx));
        }
    }

    /// 目录树里换父目录后，把当前 URL 的仓库名继续补在后面。
    fn apply_selected_parent(
        &mut self,
        parent: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let url = self.url.read(cx).value().trim().to_string();
        let Some(repo) = repo_name_from_url(&url) else {
            return;
        };
        self.auto_repo = Some(repo.clone());
        let joined = parent.join(repo);
        self.picker
            .update(cx, |p, cx| p.set_path_text(&joined, window, cx));
    }

    /// 校验表单输入并在通过后发出启动克隆事件。
    pub(crate) fn submit(&mut self, cx: &mut Context<Self>) {
        let url = self.url.read(cx).value().trim().to_string();
        let raw_dest = self
            .picker
            .read(cx)
            .path
            .read(cx)
            .value()
            .trim()
            .to_string();
        if url.is_empty() || raw_dest.is_empty() {
            self.error = Some(rust_i18n::t!("git.clone.need_fields").to_string());
            cx.notify();
            return;
        }
        let depth = if self.depth_enabled {
            match self.depth.read(cx).value().trim().parse::<u32>() {
                Ok(value) if value > 0 => Some(value),
                _ => {
                    self.error = Some(rust_i18n::t!("git.clone.depth_invalid").to_string());
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
        let folder_name = self.folder.name.read(cx).value().trim().to_string();
        cx.emit(GitCloneModalEvent::Start {
            url,
            dest,
            depth,
            repo_name,
            folder_name,
            icon: self.icon,
            color: self.color,
        });
    }
}
