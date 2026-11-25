//! 应用侧 Dialog 容器宿主：把原先自绘遮罩的模态弹窗接进组件库的弹窗栈。
//!
//! 设计原理（Why）
//! - 实现初衷：模态原先由 `AppShell::render_overlays` 每帧拼自绘遮罩，容器自带的
//!   焦点捕获、层序、右上角关闭件与取消/确认语义一概拿不到。若把
//!   `open_dialog`/`close_dialog` 散落到二十多个开关点去配对调用，弹窗状态与
//!   容器栈必然漂移。这里改成"期望栈仍从既有状态推导，每帧与实际栈对账"，
//!   各站点只写自己的弹窗状态（或实体打开标记），不再成对调用容器开关。
//! - 核心优势：
//!   1. 站点不需要成对的开关调用，新增入口不会漏掉容器操作；
//!   2. 按键语义集中在本文件：取消、提交、脏拦截一处声明，不会出现某个弹窗
//!      单独忘记接 Escape；
//!   3. 弹窗体是独立实体，随外壳状态自绘，绕开"弹窗层只由组件库内部状态驱动
//!      重建、宿主没有公开入口触发它"的限制（`WindowState` 在组件库内是
//!      `pub(crate)`）。
//! - 代价与局限：
//!   1. 对账排在绘制之后（`window.defer`），弹窗比原先晚一帧出现；
//!   2. 栈中间层被移除时只能整栈关闭再按期望顺序重开，表现为闪一下；
//!   3. 容器的 `escape → Cancel`、`enter → Confirm` 只在焦点不被输入框抢占时
//!      生效，实测见 `tests/dialog_key_probe.rs`，因此取消与提交同时接
//!      容器动作与输入框动作两路。

use crate::app::{AppShell, AppState};
use crate::container::ContainerFormFolders;
use crate::palette::Palette;
use gpui_kit::base::actions::{Cancel, Confirm};
use gpui_kit::component::input::{
    Enter as InputEnter, Escape as InputEscape, InputState, TextareaState,
};
use gpui_kit::component::{WindowExt, dialog::DialogEntrance};
use gpui_kit::*;

/// 进入组件库弹窗栈的模态种类。
///
/// 为什么要显式枚举：对账要把"当前应当开着哪些弹窗"映射成栈里的层，
/// 必须能回答"这一层是谁"。用状态字段本身当身份，等于把字段判断顺序
/// 复制两份，顺序一改就会出现关掉下层、留下上层的层序错误。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ModalKind {
    /// SSH 鉴权口令与键盘交互问答。
    AuthPrompt,
    /// 首连主机指纹确认（TOFU）。
    HostKeyConfirm,
    /// 远端 OSC 52 写剪贴板授权。
    ClipboardPrompt,
    /// 端口转发冲突确认。
    ForwardConfirm,
    /// SFTP 危险操作确认（删除等）。
    SftpConfirm,
    /// SFTP 输入类弹窗（新建目录、重命名、软链接、归档、提权）。
    SftpInput,
    /// SFTP 权限位修改。
    SftpChmod,
    /// SFTP 文件属性查看。
    SftpProperties,
    /// SFTP 过滤规则编辑（多行）。
    SftpFilter,
    /// SFTP 全盘搜索。
    SftpSearch,
    /// SFTP 目录同步。
    SftpSync,
    /// 编辑器跳转到行。
    EditorGotoLine,
    /// 容器引擎连接表单。
    ContainerForm,
    /// 引擎分组新建/重命名表单。
    ContainerFolderForm,
    /// 运行容器表单。
    ContainerRunForm,
    /// 拉取镜像 / 新建卷与网络的弹窗。
    ContainerCreate,
    /// 容器/镜像/卷/网络/Compose/文件删除确认。
    ContainerConfirm,
    /// 容器重命名。
    ContainerRename,
    /// 镜像打标签。
    ContainerTagImage,
    /// 容器提交为镜像。
    ContainerCommit,
    /// 容器资源限额修改。
    ContainerUpdate,
    /// 网络连接容器。
    ContainerNetworkConnect,
    /// 容器反向导出配置查看。
    ContainerExportConfig,
    /// 容器文件重命名。
    ContainerFileRename,
    /// 容器新建目录。
    ContainerFileNewDir,
    /// 远端镜像市场探索与搜索。
    ContainerHub,
    /// 镜像推送到远端仓库。
    ContainerPush,
    /// 全局系统磁盘用量看板与空间体检。
    ContainerSystemDf,
    /// 从 Dockerfile 构建镜像。
    ContainerBuild,
    /// 容器内端口一键建立 SSH 本地端口转发。
    ContainerForward,
    /// Podman Pod 列表与启停。
    ContainerPods,
    /// Podman Quadlet 单元列表与启停。
    ContainerQuadlet,
    /// 会话与文件夹新建/编辑表单。
    SessionForm,
    /// 文件夹新建/编辑表单。
    FolderForm,
    /// 多会话批量修改表单。
    BatchEdit,
    /// 快捷命令片段表单。
    SnippetForm,
    /// 已知主机管理。
    KnownHosts,
    /// 密钥部署向导。
    KeyWizard,
    /// 备份口令提示。
    BackupPrompt,
    /// 审计记录查看器。
    AuditViewer,
    /// 会话录制回放。
    Replay,
    /// 偏好设置。
    Settings,
    /// 应用更新：版本摘要、Markdown 更新日志与安装闭环。
    Update,
    /// 改键录入。
    ShortcutCapture,
    /// 分栏差异对比。
    DiffViewer,
    /// Git 项目管理与设置。
    GitProject,
    /// Git 仓库克隆向导。
    GitClone,
    /// Git 工作树管理。
    GitWorktree,
    /// Git 命名输入（新建分支/标签/重命名等）。
    GitName,
    /// Git 变基操作确认。
    GitRebase,
    /// Git 推送提交确认。
    GitPush,
    /// Git 冲突解决器。
    GitConflict,
    /// API 新建与导入请求弹窗。
    ApiNewRequest,
    /// API 环境与全局配置弹窗。
    ApiConfig,
}

/// 单个弹窗的渲染元数据；宽度与 slug 同一处定义，新增弹窗不再漏改列表。
pub(crate) struct ModalSpec {
    pub(crate) width: Pixels,
    pub(crate) slug: &'static str,
}

impl ModalKind {
    /// 弹窗元数据：宽度与元素标识。
    fn spec(self, window: &Window) -> ModalSpec {
        registry::spec(self, window)
    }

    /// 该弹窗是否承载需要明确取舍的危险或授权决策。
    ///
    /// AlertDialog 默认不显示右上角关闭件，也不接受遮罩点击关闭；用它承载
    /// 主机指纹、剪贴板授权和删除确认，可以让用户必须显式选择确认或取消，
    /// 避免普通表单弹窗的关闭路径绕过业务决策。
    fn uses_alert_dialog(self) -> bool {
        matches!(
            self,
            Self::HostKeyConfirm
                | Self::ClipboardPrompt
                | Self::ForwardConfirm
                | Self::SftpConfirm
                | Self::ContainerConfirm
        )
    }
}

mod body;
mod container;
mod git;
mod registry;
mod stack;
pub(crate) use stack::{dialog_dim, reconcile_dialogs};
mod state;

pub(crate) use body::{dialog_bindings, on_event};
pub(crate) use stack::OpenModals;

impl ModalKind {
    /// 弹窗体：内容与页脚仍由各领域自己渲染。
    /// 设置、已知主机、备份、审计查看器、密钥向导、会话回放、会话表单、
    /// 连接确认与运行容器表单的状态归实体，先克隆句柄再进入 `update`，
    /// 其它弹窗直接读外壳。
    ///
    /// 分开两条路径是为了避免“`app.read(cx)` 的只读借用一直活到实体的可变
    /// `update`”这类借用冲突：实体分支先克隆句柄、立即结束外壳借用。
    fn render_body(
        self,
        app: &Entity<AppShell>,
        window: &mut Window,
        cx: &mut App,
        p: &Palette,
    ) -> AnyElement {
        if self == ModalKind::Settings {
            let settings = app.read(cx).state.ui.modals.settings.clone();
            return settings.update(cx, |view, cx| view.render_body(&settings, window, cx, p));
        }
        if self == ModalKind::Update {
            let update = app.read(cx).state.features.update.clone();
            return update.update(cx, |view, _cx| view.render_body(&update, p));
        }
        if self == ModalKind::KnownHosts {
            let view = app.read(cx).state.ui.modals.connection.known_hosts.clone();
            return view.update(cx, |view, cx| view.render_body(cx, p));
        }
        if self == ModalKind::BackupPrompt {
            let view = app.read(cx).state.ui.modals.tools.backup_prompt.clone();
            return view.update(cx, |view, cx| view.render_body(cx, p));
        }
        if self == ModalKind::AuditViewer {
            let view = app.read(cx).state.ui.modals.tools.audit_viewer.clone();
            return view.update(cx, |view, cx| view.render_body(cx, p));
        }
        if self == ModalKind::KeyWizard {
            let (view, targets) = {
                let app_state = &app.read(cx).state;
                (
                    app_state.ui.modals.tools.key_wizard.clone(),
                    app_state.key_targets(),
                )
            };
            return view.update(cx, |view, cx| view.render_body(&targets, cx, p));
        }
        if self == ModalKind::Replay {
            let view = app.read(cx).state.ui.modals.tools.replay.clone();
            return view.update(cx, |view, cx| view.render_body(cx, p));
        }
        if self == ModalKind::SessionForm {
            let view = app.read(cx).state.ui.modals.session.form.clone();
            return view.update(cx, |view, cx| view.render_body(window, cx, p));
        }
        if self == ModalKind::ContainerForm {
            // 分组候选由外壳在读锁内快照后注入，实体不反查目录；快照读完即
            // 结束外壳借用，随后的实体 update 才不会撞上双重租借。
            let (view, folders) = {
                let app_state = &app.read(cx).state;
                let options = app_state
                    .session
                    .catalog
                    .engine_folders
                    .iter()
                    .map(|folder| (folder.id, folder.name.clone()))
                    .collect();
                (
                    app_state.ui.modals.tools.container_form.clone(),
                    ContainerFormFolders { options },
                )
            };
            return view.update(cx, |view, cx| view.render_body(&folders, p, cx));
        }
        if self == ModalKind::ContainerRunForm {
            // 镜像摘要由外壳在读锁内快照后注入，实体不反查工作区；快照读完即
            // 结束外壳借用，随后的实体 update 才不会撞上双重租借。
            let (view, image_meta) = {
                let app_state = &app.read(cx).state;
                let view = app_state.ui.modals.tools.run_form.clone();
                let image_meta = view.read(cx).state().and_then(|form| {
                    let image_raw = form.image.read(cx).value().trim().to_string();
                    app_state
                        .container_workspace(form.tab_id)
                        .and_then(|workspace| {
                            workspace
                                .read(cx)
                                .state
                                .images
                                .iter()
                                .find(|image| {
                                    image.tags.contains(&image_raw)
                                        || image.id.starts_with(&image_raw)
                                })
                                .cloned()
                        })
                });
                (view, image_meta)
            };
            return view.update(cx, |view, cx| view.render_body(image_meta.as_ref(), p, cx));
        }
        if self == ModalKind::ContainerCreate {
            // 主输入仍归目标工作区：弹窗宿主在只读外壳里取出句柄后注入实体，
            // 实体不反查工作区；句柄读完即结束外壳借用，随后的实体 update
            // 才不会撞上双重租借。
            let (view, create_input) = {
                let app_state = &app.read(cx).state;
                let view = app_state.ui.modals.tools.create_dialog.clone();
                let create_input = view.read(cx).state().and_then(|dialog| {
                    app_state
                        .container_workspace(dialog.tab_id)
                        .map(|workspace| workspace.read(cx).state.create_input.clone())
                });
                (view, create_input)
            };
            let Some(create_input) = create_input else {
                return div().into_any_element();
            };
            return view.update(cx, |view, cx| view.render_body(create_input, p, cx));
        }
        if let Some(element) = {
            let app_state = &app.read(cx).state;
            self.container_workspace_body(app_state, cx, p)
        } {
            return element;
        }
        if matches!(
            self,
            ModalKind::AuthPrompt | ModalKind::HostKeyConfirm | ModalKind::ClipboardPrompt
        ) {
            let view = app.read(cx).state.ui.modals.connection.prompts.clone();
            return view.update(cx, |view, cx| view.render_body(self, cx, p));
        }
        let app_state = &app.read(cx).state;
        match self {
            ModalKind::SftpConfirm => app_state
                .ui
                .modals
                .sftp
                .confirm
                .as_ref()
                .map(|m| m.clone().into_any_element())
                .unwrap_or_else(|| div().into_any_element()),
            ModalKind::SftpInput => app_state
                .ui
                .modals
                .sftp
                .input
                .as_ref()
                .map(|m| m.clone().into_any_element())
                .unwrap_or_else(|| div().into_any_element()),
            ModalKind::SftpChmod => app_state
                .ui
                .modals
                .sftp
                .chmod
                .as_ref()
                .map(|m| m.clone().into_any_element())
                .unwrap_or_else(|| div().into_any_element()),
            ModalKind::SftpProperties => app_state
                .ui
                .modals
                .sftp
                .properties
                .as_ref()
                .map(|m| m.clone().into_any_element())
                .unwrap_or_else(|| div().into_any_element()),
            ModalKind::SftpFilter => app_state
                .ui
                .modals
                .sftp
                .filter
                .as_ref()
                .map(|m| m.clone().into_any_element())
                .unwrap_or_else(|| div().into_any_element()),
            ModalKind::SftpSearch => app_state
                .ui
                .modals
                .sftp
                .search
                .as_ref()
                .map(|m| m.clone().into_any_element())
                .unwrap_or_else(|| div().into_any_element()),
            ModalKind::SftpSync => app_state
                .ui
                .modals
                .sftp
                .sync
                .as_ref()
                .map(|m| m.clone().into_any_element())
                .unwrap_or_else(|| div().into_any_element()),
            _ => self.render_shell_body(app_state, app, window, cx, p),
        }
    }

    /// 非实体弹窗的渲染分发：只读外壳状态。
    fn render_shell_body(
        self,
        app_state: &AppState,
        app: &Entity<AppShell>,
        window: &mut Window,
        cx: &App,
        p: &Palette,
    ) -> AnyElement {
        if let Some(element) = self.render_git_dialog_body(app_state, app, window, cx, p) {
            return element;
        }
        match self {
            ModalKind::ForwardConfirm => {
                crate::forward::render_forward_confirm_body(app_state, app, cx, p)
            }
            ModalKind::EditorGotoLine => {
                crate::editor::render_editor_goto_line_body(app_state, app, cx, p)
            }
            // 由 [`Self::render_body`] 的实体分支处理，这里不会走到。
            ModalKind::ContainerForm => div().into_any_element(),
            ModalKind::ContainerFolderForm => {
                let sidebar = app_state.features.containers.sidebar.clone();
                let state = sidebar.read(cx);
                crate::container::render_engine_folder_form_body(
                    &sidebar,
                    &state.state,
                    &app_state.session.catalog,
                    p,
                )
            }
            // 由 [`Self::render_body`] 的实体分支处理，这里不会走到。
            ModalKind::ContainerRunForm => div().into_any_element(),
            // 由 [`Self::render_body`] 的实体分支处理，这里不会走到。
            ModalKind::ContainerCreate => div().into_any_element(),
            // 由 [`Self::render_body`] 的实体分支处理，这里不会走到。
            // 以下容器弹窗由 [`Self::render_body`] 的实体分支处理，这里不会走到。
            ModalKind::ContainerConfirm
            | ModalKind::ContainerRename
            | ModalKind::ContainerTagImage
            | ModalKind::ContainerCommit
            | ModalKind::ContainerUpdate
            | ModalKind::ContainerNetworkConnect
            | ModalKind::ContainerExportConfig
            | ModalKind::ContainerFileRename
            | ModalKind::ContainerFileNewDir => div().into_any_element(),
            // 由 [`Self::render_body`] 的实体分支处理，这里不会走到。
            ModalKind::ContainerHub => div().into_any_element(),
            // 由 [`Self::render_body`] 的实体分支处理，这里不会走到。
            ModalKind::ContainerPush => div().into_any_element(),
            // 由 [`Self::render_body`] 的实体分支处理，这里不会走到。
            ModalKind::ContainerSystemDf => div().into_any_element(),
            // 由 [`Self::render_body`] 的实体分支处理，这里不会走到。
            ModalKind::ContainerBuild => div().into_any_element(),
            // 由 [`Self::render_body`] 的实体分支处理，这里不会走到。
            ModalKind::ContainerForward => div().into_any_element(),
            // 由 [`Self::render_body`] 的实体分支处理，这里不会走到。
            ModalKind::ContainerPods | ModalKind::ContainerQuadlet => div().into_any_element(),
            // 由 [`Self::render_body`] 的实体分支处理，这里不会走到。
            ModalKind::SessionForm => div().into_any_element(),
            ModalKind::FolderForm => {
                crate::sidebar::render_folder_form_body(app_state, app, window, cx, p)
            }
            ModalKind::BatchEdit => {
                crate::batch::render_batch_edit_body(app_state, app, window, cx, p)
            }
            ModalKind::SnippetForm => {
                crate::snippets::render_snippet_form_body(app_state, app, window, cx, p)
            }
            // 由 [`Self::render_body`] 的实体分支处理，这里不会走到。
            ModalKind::KnownHosts => div().into_any_element(),
            // 由 [`Self::render_body`] 的实体分支处理，这里不会走到。
            ModalKind::KeyWizard => div().into_any_element(),
            // 由 [`Self::render_body`] 的实体分支处理，这里不会走到。
            ModalKind::BackupPrompt => div().into_any_element(),
            // 由 [`Self::render_body`] 的实体分支处理，这里不会走到。
            ModalKind::AuditViewer => div().into_any_element(),
            // 由 [`Self::render_body`] 的实体分支处理，这里不会走到。
            ModalKind::Replay => div().into_any_element(),
            ModalKind::Settings => {
                // 由 [`Self::render_body`] 的专用分支处理，这里不会走到。
                div().into_any_element()
            }
            ModalKind::ShortcutCapture => {
                crate::shortcuts::render_shortcut_capture_body(app_state, app, window, cx, p)
            }
            ModalKind::DiffViewer => crate::diff_viewer::render_diff_viewer_body(
                app_state.features.diff_viewer.as_ref(),
                app,
                window,
                cx,
                p,
            ),
            ModalKind::ApiNewRequest => {
                let Some(workspace) = app_state
                    .workspace
                    .active_tab
                    .and_then(|tab_id| app_state.workspace.tab(tab_id))
                    .and_then(|tab| tab.api_workspace())
                else {
                    return div().into_any_element();
                };
                crate::api_client::render_api_new_request_dialog_body(
                    &workspace,
                    &app_state.features.api_sidebar,
                    window,
                    cx,
                    p,
                )
            }
            ModalKind::ApiConfig => {
                let Some(workspace) = app_state
                    .workspace
                    .active_tab
                    .and_then(|tab_id| app_state.workspace.tab(tab_id))
                    .and_then(|tab| tab.api_workspace())
                else {
                    return div().into_any_element();
                };
                crate::api_client::render_api_config_dialog_body(
                    &workspace,
                    &app_state.features.api_sidebar,
                    window,
                    cx,
                    p,
                )
            }
            _ => div().into_any_element(),
        }
    }
}

#[cfg(test)]
mod tests;
