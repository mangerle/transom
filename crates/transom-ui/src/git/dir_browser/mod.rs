//! Git 弹窗里的应用内目录树面板（打开项目 / 克隆共用）。
//!
//! 设计原理（Why）：
//! - 树形而不是「点一下就整屏换一层」：选仓库时人要在层级之间来回看（这个目录、
//!   它的同级、它的父级），一次只显示一层的扁平列表每挪一步都要重新认路，而树把
//!   父链一直留在屏幕上。行首一个三角负责展开/收起，点行本身是选中，两件事分开，
//!   与会话侧栏的折叠箭头同一套手感；
//! - 因此没有 `.. (返回上一级)` 这一行：树里父级就在上面，再放一行「跳到上一层」
//!   等于把导航做反；上一层由表头的上箭头负责（换根，而不是插入一行）；
//! - 子项懒加载并按路径缓存：展开某个目录才读它那一层，收起再展开不重新读盘；
//!   一次只列一层不递归，`target`、`node_modules` 这类目录递归扫会跑几十秒；
//! - 文件也列出来但不可点、不展开：只列目录会让人怀疑「是不是没加载完」，
//!   目录 + 文件混排才像同一个文件面板（形态照 SFTP 本地列）；
//! - 快捷目录（主目录/桌面/下载）复用 SFTP 那份 XDG 解析（`sftp::quick_paths`）：
//!   中文系统的用户目录叫「桌面」「下载」，自己拼 `$HOME/Desktop` 会指错地方；
//!   目录不存在时按钮置灰而不是隐藏，用户才知道这个入口存在、只是本机没有；
//! - 列举丢到后台执行器：网络盘上的 `read_dir` 会挂住，UI 线程一挂整个窗口冻结。
//!   与 SFTP 本地窗格同一口径（见 `sftp/local_async.rs`），但单层列举是短任务，
//!   直接用 gpui 后台执行器，不为它单开一条专用线程；
//! - 每层结果按路径归位：某个目录的列举回来时只写它自己那一格，用户中途收起或
//!   换根都不会把内容塞进别的目录行下面。

mod listing;
mod pane;
mod rows;
mod state;

use listing::{DirListing, check_dir_name, list_entries};
pub(crate) use pane::render_git_browser;
pub(crate) use state::DirPicker;

use gpui_kit::component::input::InputState;
use gpui_kit::*;
pub(crate) use state::{DirDraft, GitDirBrowser};
use std::path::{Path, PathBuf};

/// 目录树面板向上层宿主上报的事件。
pub(crate) enum GitDirBrowserEvent {
    /// 选中目录或换根后通知宿主最新路径。
    PathSelected(PathBuf),
    /// 双击条目后请求宿主收起折叠树（项目弹窗忽略，克隆弹窗收起）。
    CloseTree,
}

impl EventEmitter<GitDirBrowserEvent> for DirPicker {}

impl DirPicker {
    /// 把指定路径写回路径输入框。
    pub(crate) fn set_path_text(&self, path: &Path, window: &mut Window, cx: &mut App) {
        let text = path.display().to_string();
        self.path.update(cx, |state, cx| {
            state.set_value(text, window, cx);
        });
    }

    /// 换根到某个目录：写回路径框并通知宿主。
    pub(crate) fn browse_root(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.browser.re_root(path.clone());
        self.browser.selected = Some(path.clone());
        self.set_path_text(&path, window, cx);
        cx.notify();
        self.spawn_dir_listing(path.clone(), cx);
        cx.emit(GitDirBrowserEvent::PathSelected(path));
    }

    /// 跳到快捷目录；目录不存在（按钮已置灰）时不动。
    pub(crate) fn browse_quick(
        &mut self,
        path: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = path else {
            return;
        };
        self.browse_root(path, window, cx);
    }

    /// 点目录行：选中它（写回路径框 + 通知宿主），不改变展开状态。
    pub(crate) fn select_dir(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.browser.selected = Some(path.clone());
        self.set_path_text(&path, window, cx);
        cx.notify();
        cx.emit(GitDirBrowserEvent::PathSelected(path));
    }

    /// 双击选中目录后通知宿主收起目录树。
    pub(crate) fn close_dir_tree(&mut self, cx: &mut Context<Self>) {
        cx.emit(GitDirBrowserEvent::CloseTree);
    }

    /// 点三角：展开或收起树里某个目录，首次展开时读它那一层。
    pub(crate) fn toggle_tree_dir(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let needs_listing = self.browser.toggle(&path);
        cx.notify();
        if let Some(path) = needs_listing {
            self.spawn_dir_listing(path, cx);
        }
    }

    /// 重新列举：根与所有展开中的目录都清缓存重读。
    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        self.relist_browser(cx);
    }

    /// 切换是否显示隐藏条目：口径变了，所有缓存都要作废。
    pub(crate) fn toggle_hidden(&mut self, cx: &mut Context<Self>) {
        self.browser.show_hidden = !self.browser.show_hidden;
        self.relist_browser(cx);
    }

    fn relist_browser(&mut self, cx: &mut Context<Self>) {
        let paths = self.browser.relist_all();
        cx.notify();
        for path in paths {
            self.spawn_dir_listing(path, cx);
        }
    }

    /// 点「新建目录」：在目标父级末尾长出一行处于重命名态的条目。
    pub(crate) fn start_dir_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let parent = self
            .browser
            .selected
            .clone()
            .unwrap_or_else(|| self.browser.root.clone());
        self.browser.expand(&parent);
        let needs_listing = !self.browser.is_listed(&parent);
        if needs_listing {
            self.browser.queue(&parent);
        }
        self.error = None;
        let input = cx.new(|cx| InputState::new(window, cx));
        self.browser.draft = Some(DirDraft {
            parent: parent.clone(),
            input,
        });
        cx.notify();
        if needs_listing {
            self.spawn_dir_listing(parent, cx);
        }
        let exec = cx.background_executor().clone();
        cx.spawn_in(window, async move |this, cx| {
            exec.timer(std::time::Duration::from_millis(30)).await;
            let _ = this.update_in(cx, |picker, window, cx| {
                picker.focus_draft_input(window, cx);
            });
        })
        .detach();
    }

    fn focus_draft_input(&self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(handle) = self
            .browser
            .draft
            .as_ref()
            .map(|draft| draft.input.read(cx).focus_handle(cx))
        else {
            return;
        };
        window.focus(&handle, cx);
    }

    /// 取消草稿行：只收掉这一行，不动已选中的目录。
    pub(crate) fn cancel_dir_draft(&mut self, cx: &mut Context<Self>) {
        self.browser.draft = None;
        cx.notify();
    }

    /// 确认草稿行：校验名字后建目录，建好即选中并通知宿主。
    pub(crate) fn commit_dir_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (parent, raw) = match self.browser.draft.as_ref() {
            Some(draft) => (
                draft.parent.clone(),
                draft.input.read(cx).value().trim().to_string(),
            ),
            None => return,
        };
        if let Some(key) = check_dir_name(&raw) {
            self.error = Some(rust_i18n::t!(key).to_string());
            cx.notify();
            return;
        }
        let created_path = parent.join(&raw);
        self.error = None;
        let executor = cx.background_executor().clone();
        cx.spawn_in(window, async move |this, cx| {
            let created = executor
                .spawn(async move { std::fs::create_dir_all(&created_path).map(|()| created_path) })
                .await;
            let _ = this.update_in(cx, |picker, window, cx| match created {
                Ok(path) => {
                    log::info!("已在目录树里新建目录: {}", path.display());
                    picker.after_mkdir(&path, window, cx);
                }
                Err(error) => {
                    log::warn!("新建目录失败，原因: {error}");
                    picker.error = Some(format!(
                        "{}: {error}",
                        rust_i18n::t!("git.browser.mkdir_failed")
                    ));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn after_mkdir(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let owned = path.to_path_buf();
        let mut pending: Vec<PathBuf> = Vec::with_capacity(2);
        if let Some(parent) = path.parent() {
            self.browser.expand(parent);
            let parent = parent.to_path_buf();
            pending.push(self.browser.refresh_one(&parent));
        }
        self.browser.queue(&owned);
        pending.push(owned.clone());
        self.browser.selected = Some(owned.clone());
        self.browser.draft = None;
        self.set_path_text(&owned, window, cx);
        cx.notify();
        for path in pending {
            self.spawn_dir_listing(path, cx);
        }
        cx.emit(GitDirBrowserEvent::PathSelected(owned));
    }

    fn spawn_dir_listing(&self, path: PathBuf, cx: &mut Context<Self>) {
        let executor = cx.background_executor().clone();
        let show_hidden = self.browser.show_hidden;
        let job_path = path.clone();
        cx.spawn(async move |this, cx| {
            let (entries, truncated, error) = executor
                .spawn(async move { list_entries(&job_path, show_hidden) })
                .await;
            let _ = this.update(cx, |picker, cx| {
                let recorded = picker.browser.record(
                    &path,
                    DirListing {
                        entries,
                        truncated,
                        error,
                        loading: false,
                    },
                );
                if recorded {
                    cx.notify();
                }
            });
        })
        .detach();
    }
}
