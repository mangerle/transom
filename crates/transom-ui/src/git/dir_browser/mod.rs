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

pub(crate) use crate::git::form::GitHost;
use listing::{DirListing, check_dir_name, list_entries};
pub(crate) use pane::render_git_browser;
pub(crate) use state::DirPicker;

use crate::app::AppShell;
use gpui_kit::component::input::InputState;
use gpui_kit::*;
pub(crate) use state::{DirDraft, GitDirBrowser};
use std::path::{Path, PathBuf};

impl AppShell {
    /// 按宿主取到那棵树的只读视图。
    fn picker(&self, target: GitHost) -> Option<&DirPicker> {
        match target {
            GitHost::Modal => self
                .state
                .modals
                .git
                .project
                .as_ref()
                .map(|modal| &modal.picker),
            GitHost::Clone => self
                .state
                .modals
                .git
                .clone
                .as_ref()
                .map(|modal| &modal.picker),
        }
    }

    /// 按宿主取到那棵树的可写视图。
    fn picker_mut(&mut self, target: GitHost) -> Option<&mut DirPicker> {
        match target {
            GitHost::Modal => self
                .state
                .modals
                .git
                .project
                .as_mut()
                .map(|modal| &mut modal.picker),
            GitHost::Clone => self
                .state
                .modals
                .git
                .clone
                .as_mut()
                .map(|modal| &mut modal.picker),
        }
    }
}

impl AppShell {
    /// 换根到某个目录：写回路径框并立刻探测是不是仓库。
    pub(crate) fn git_browse_root(
        &mut self,
        target: GitHost,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 克隆弹窗里用户点的是父目录，路径框要保留「父目录/仓库名」；
        // 项目弹窗点哪个目录就是哪个目录。
        let input_path = if matches!(target, GitHost::Clone) {
            self.git_clone_selected_input(path.clone(), cx)
        } else {
            path.clone()
        };
        let Some(picker) = self.picker_mut(target) else {
            return;
        };
        picker.browser.re_root(path.clone());
        picker.browser.selected = Some(path.clone());
        sync_path_inputs(picker, &input_path, window, cx);
        cx.notify();
        self.spawn_dir_listing(target, path, cx);
        self.probe_if_modal(target, cx);
    }

    /// 跳到快捷目录；目录不存在（按钮已置灰）时不动。
    pub(crate) fn git_browse_quick(
        &mut self,
        target: GitHost,
        path: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = path else {
            return;
        };
        self.git_browse_root(target, path, window, cx);
    }

    /// 点目录行：选中它（写回路径框 + 探测），不改变展开状态。
    ///
    /// 为什么选中与展开分开：一路点行进到深层，等于把「选中的东西」和「看的东西」
    /// 绑死，想选父级就得换根；三角管看，行管选，两件事各一次点击。
    pub(crate) fn git_select_dir(
        &mut self,
        target: GitHost,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input_path = if matches!(target, GitHost::Clone) {
            self.git_clone_selected_input(path.clone(), cx)
        } else {
            path.clone()
        };
        let Some(picker) = self.picker_mut(target) else {
            return;
        };
        picker.browser.selected = Some(path);
        sync_path_inputs(picker, &input_path, window, cx);
        cx.notify();
        self.probe_if_modal(target, cx);
    }

    /// 双击选中目录后收起克隆弹窗的目录树；项目弹窗的树是主内容，不收起。
    pub(crate) fn git_close_dir_tree(&mut self, target: GitHost, cx: &mut Context<Self>) {
        if !matches!(target, GitHost::Clone) {
            return;
        }
        let Some(modal) = self.state.modals.git.clone.as_mut() else {
            return;
        };
        if modal.tree_open {
            modal.tree_open = false;
            cx.notify();
        }
    }

    /// 点三角：展开或收起树里某个目录，首次展开时读它那一层。
    pub(crate) fn git_toggle_tree_dir(
        &mut self,
        target: GitHost,
        path: PathBuf,
        cx: &mut Context<Self>,
    ) {
        let Some(picker) = self.picker_mut(target) else {
            return;
        };
        let needs_listing = picker.browser.toggle(&path);
        cx.notify();
        if let Some(path) = needs_listing {
            self.spawn_dir_listing(target, path, cx);
        }
    }

    /// 重新列举：根与所有展开中的目录都清缓存重读，外部改了文件系统不必重开弹窗。
    pub(crate) fn git_refresh_browser(&mut self, target: GitHost, cx: &mut Context<Self>) {
        self.relist_browser(target, cx);
    }

    /// 切换是否显示隐藏条目：口径变了，所有缓存都要作废。
    pub(crate) fn git_toggle_hidden(&mut self, target: GitHost, cx: &mut Context<Self>) {
        let Some(picker) = self.picker_mut(target) else {
            return;
        };
        picker.browser.show_hidden = !picker.browser.show_hidden;
        self.relist_browser(target, cx);
    }

    /// 作废并重列根与展开中的目录。
    fn relist_browser(&mut self, target: GitHost, cx: &mut Context<Self>) {
        let Some(picker) = self.picker_mut(target) else {
            return;
        };
        let paths = picker.browser.relist_all();
        cx.notify();
        for path in paths {
            self.spawn_dir_listing(target, path, cx);
        }
    }

    /// 点「新建目录」：在目标父级末尾长出一行处于重命名态的条目。
    ///
    /// 目标父级取「选中项，没选中就是根」：用户点了哪个目录就往哪个目录里建，
    /// 与文件管理器的直觉一致；父级处于收起态时先展开，否则新行看不见。
    pub(crate) fn git_start_dir_draft(
        &mut self,
        target: GitHost,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(picker) = self.picker_mut(target) else {
            return;
        };
        let parent = picker
            .browser
            .selected
            .clone()
            .unwrap_or_else(|| picker.browser.root.clone());
        picker.browser.expand(&parent);
        let needs_listing = !picker.browser.is_listed(&parent);
        if needs_listing {
            // 收起的父级还没读过内容，不补一次列举就会出现"有输入框却没有列表"。
            picker.browser.queue(&parent);
        }
        picker.error = None;
        let input = cx.new(|cx| InputState::new(window, cx));
        picker.browser.draft = Some(DirDraft {
            parent: parent.clone(),
            input: input.clone(),
        });
        cx.notify();
        // 借用到此结束：下面要动 self 派发列举，也要动 window 投焦点。
        if needs_listing {
            self.spawn_dir_listing(target, parent, cx);
        }
        // 焦点延后一帧再投：草稿行是这一帧才建出来的，元素还没挂上，此时 focus 会被
        // 平台退回原来的焦点（活动标签页里的终端），表现就是"名字打进终端了"。
        let entity = cx.entity().clone();
        let exec = cx.background_executor().clone();
        window
            .spawn(cx, async move |cx| {
                exec.timer(std::time::Duration::from_millis(30)).await;
                let _ = cx.update(|window, app| {
                    entity.update(app, |this, cx| {
                        this.focus_git_draft_input(target, window, cx);
                    });
                });
            })
            .detach();
    }

    /// 把焦点投给草稿行的输入框；草稿已提交或取消时什么都不做。
    fn focus_git_draft_input(
        &mut self,
        target: GitHost,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(handle) = self
            .picker(target)
            .and_then(|picker| picker.browser.draft.as_ref())
            .map(|draft| draft.input.read(cx).focus_handle(cx))
        else {
            return;
        };
        window.focus(&handle, cx);
    }

    /// 取消草稿行：只收掉这一行，不动已选中的目录。
    pub(crate) fn git_cancel_dir_draft(&mut self, target: GitHost, cx: &mut Context<Self>) {
        if let Some(picker) = self.picker_mut(target) {
            picker.browser.draft = None;
        }
        cx.notify();
    }

    /// 确认草稿行：校验名字后建目录，建好即选中并探测。
    pub(crate) fn git_commit_dir_draft(
        &mut self,
        host: GitHost,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(picker) = self.picker_mut(host) else {
            return;
        };
        let (parent, raw) = match picker.browser.draft.as_ref() {
            Some(draft) => (
                draft.parent.clone(),
                draft.input.read(cx).value().trim().to_string(),
            ),
            None => return,
        };
        if let Some(key) = check_dir_name(&raw) {
            picker.error = Some(rust_i18n::t!(key).to_string());
            cx.notify();
            return;
        }
        let created_path = parent.join(&raw);
        picker.error = None;
        let entity = cx.entity().clone();
        let executor = cx.background_executor().clone();
        // 用 `window.spawn` 而不是 `cx.spawn`：建好目录后要写回路径框，
        // 而 `InputState::set_value` 只有拿到 Window 才调得动。
        window
            .spawn(cx, async move |cx| {
                let created =
                    executor
                        .spawn(async move {
                            std::fs::create_dir_all(&created_path).map(|()| created_path)
                        })
                        .await;
                let _ = cx.update(|window, app| match created {
                    Ok(path) => {
                        log::info!("已在目录树里新建目录: {}", path.display());
                        entity.update(app, |this, cx| {
                            this.git_after_mkdir(host, &path, window, cx);
                        });
                    }
                    Err(error) => {
                        log::warn!("新建目录失败，原因: {error}");
                        entity.update(app, |this, cx| {
                            if let Some(picker) = this.picker_mut(host) {
                                picker.error = Some(format!(
                                    "{}: {error}",
                                    rust_i18n::t!("git.browser.mkdir_failed")
                                ));
                            }
                            cx.notify();
                        });
                    }
                });
            })
            .detach();
    }

    /// 新建目录成功后的收尾：展开父级、补列没读过的层、把新目录设为选中项。
    fn git_after_mkdir(
        &mut self,
        target: GitHost,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 新建的是父目录，克隆弹窗的路径框仍要带仓库名；树里选中的是新建目录本身。
        let input_path = if matches!(target, GitHost::Clone) {
            self.git_clone_selected_input(path.to_path_buf(), cx)
        } else {
            path.to_path_buf()
        };
        let Some(picker) = self.picker_mut(target) else {
            return;
        };
        let owned = path.to_path_buf();
        let mut pending: Vec<PathBuf> = Vec::with_capacity(2);
        if let Some(parent) = path.parent() {
            picker.browser.expand(parent);
            let parent = parent.to_path_buf();
            // 父级可能早已列过（展开时读过、当时还没有这个目录），只判断"没列过"
            // 会把新条目挡在缓存之外，表现为新建成功但树里看不到。这里强制重读。
            pending.push(picker.browser.refresh_one(&parent));
        }
        picker.browser.queue(&owned);
        pending.push(owned.clone());
        picker.browser.selected = Some(owned);
        picker.browser.draft = None;
        sync_path_inputs(picker, &input_path, window, cx);
        cx.notify();
        for path in pending {
            self.spawn_dir_listing(target, path, cx);
        }
        self.probe_if_modal(target, cx);
    }

    /// 派发一次后台列举，结果按路径归位到那一格。
    fn spawn_dir_listing(&mut self, target: GitHost, path: PathBuf, cx: &mut Context<Self>) {
        let executor = cx.background_executor().clone();
        let show_hidden = self
            .picker(target)
            .is_some_and(|picker| picker.browser.show_hidden);
        let entity = cx.entity().clone();
        // 路径既要交给后台任务，又要用来定位结果那一格，先复制一份。
        let job_path = path.clone();
        cx.spawn(async move |_, cx| {
            let (entries, truncated, error) = executor
                .spawn(async move { list_entries(&job_path, show_hidden) })
                .await;
            entity.update(cx, |this, cx| {
                let Some(picker) = this.picker_mut(target) else {
                    return;
                };
                // 结果只写回它自己那一格：期间换根或收起都不影响别的目录。
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

    /// 只有「打开本机仓库」的宿主需要探测：克隆标签页的目标目录还不存在，
    /// 对它跑 `rev-parse` 只会得到一句"不是仓库"并污染状态行。
    fn probe_if_modal(&mut self, target: GitHost, cx: &mut Context<Self>) {
        if matches!(target, GitHost::Modal) {
            self.probe_git_project_dir(cx);
        }
    }
}

/// 浏览结果写回路径框：路径框是唯一真相，底部按钮的语义读的就是它。
fn sync_path_inputs(
    picker: &mut DirPicker,
    path: &Path,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let text = path.display().to_string();
    picker.path.update(cx, |state, cx| {
        state.set_value(text, window, cx);
    });
}
