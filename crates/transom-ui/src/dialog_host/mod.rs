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

use crate::app::AppShell;
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
}

mod body;
mod container;
mod git;
mod registry;
mod stack;
mod state;

pub(crate) use body::{dialog_bindings, on_event};
pub(crate) use stack::OpenModals;

impl ModalKind {
    /// 弹窗体：内容与页脚仍由各领域自己渲染。
    /// 设置、已知主机、备份、审计查看器、密钥向导、会话回放、会话表单与连接
    /// 确认弹窗的状态归实体，先克隆句柄再进入 `update`，其它弹窗直接读外壳。
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
            let settings = app.read(cx).state.modals.settings.clone();
            return settings.update(cx, |view, cx| view.render_body(&settings, window, cx, p));
        }
        if self == ModalKind::KnownHosts {
            let view = app.read(cx).state.modals.connection.known_hosts.clone();
            return view.update(cx, |view, cx| view.render_body(cx, p));
        }
        if self == ModalKind::BackupPrompt {
            let view = app.read(cx).state.modals.tools.backup_prompt.clone();
            return view.update(cx, |view, cx| view.render_body(cx, p));
        }
        if self == ModalKind::AuditViewer {
            let view = app.read(cx).state.modals.tools.audit_viewer.clone();
            return view.update(cx, |view, cx| view.render_body(cx, p));
        }
        if self == ModalKind::KeyWizard {
            let (view, targets) = {
                let shell = app.read(cx);
                (
                    shell.state.modals.tools.key_wizard.clone(),
                    shell.state.key_targets(),
                )
            };
            return view.update(cx, |view, cx| view.render_body(&targets, cx, p));
        }
        if self == ModalKind::Replay {
            let view = app.read(cx).state.modals.tools.replay.clone();
            return view.update(cx, |view, cx| view.render_body(cx, p));
        }
        if self == ModalKind::SessionForm {
            let view = app.read(cx).state.modals.session.form.clone();
            return view.update(cx, |view, cx| view.render_body(window, cx, p));
        }
        if self == ModalKind::ContainerForm {
            // 分组候选由外壳在读锁内快照后注入，实体不反查目录；快照读完即
            // 结束外壳借用，随后的实体 update 才不会撞上双重租借。
            let (view, folders) = {
                let shell = app.read(cx);
                let options = shell
                    .state
                    .catalog
                    .engine_folders
                    .iter()
                    .map(|folder| (folder.id, folder.name.clone()))
                    .collect();
                (
                    shell.state.modals.tools.container_form.clone(),
                    ContainerFormFolders { options },
                )
            };
            return view.update(cx, |view, cx| view.render_body(&folders, p, cx));
        }
        if matches!(
            self,
            ModalKind::AuthPrompt | ModalKind::HostKeyConfirm | ModalKind::ClipboardPrompt
        ) {
            let view = app.read(cx).state.modals.connection.prompts.clone();
            return view.update(cx, |view, cx| view.render_body(self, cx, p));
        }
        let shell = app.read(cx);
        self.render_shell_body(shell, app, window, cx, p)
    }

    /// 非实体弹窗的渲染分发：只读外壳状态。
    fn render_shell_body(
        self,
        shell: &AppShell,
        app: &Entity<AppShell>,
        window: &mut Window,
        cx: &App,
        p: &Palette,
    ) -> AnyElement {
        if let Some(element) = self.render_git_dialog_body(shell, app, window, cx, p) {
            return element;
        }
        match self {
            ModalKind::SftpConfirm => shell.render_sftp_confirm_body(app, cx, p),
            // 由 [`Self::render_body`] 的实体分支处理，这里不会走到。
            ModalKind::AuthPrompt | ModalKind::HostKeyConfirm | ModalKind::ClipboardPrompt => {
                div().into_any_element()
            }
            ModalKind::ForwardConfirm => shell.render_forward_confirm_body(app, cx, p),
            ModalKind::SftpInput => shell.render_sftp_input_body(app, cx, p),
            ModalKind::SftpChmod => shell.render_sftp_chmod_body(app, cx, p),
            ModalKind::SftpProperties => shell.render_sftp_properties_body(app, cx, p),
            ModalKind::SftpFilter => shell.render_sftp_filter_body(app, cx, p),
            ModalKind::SftpSearch => shell.render_sftp_search_body(app, cx, p),
            ModalKind::SftpSync => shell.render_sftp_sync_body(app, cx, p),
            ModalKind::EditorGotoLine => shell.render_editor_goto_line_body(app, cx, p),
            // 由 [`Self::render_body`] 的实体分支处理，这里不会走到。
            ModalKind::ContainerForm => div().into_any_element(),
            ModalKind::ContainerFolderForm => shell.render_engine_folder_form_body(app, cx, p),
            ModalKind::ContainerRunForm => shell.render_container_run_form_body(app, window, cx, p),
            ModalKind::ContainerCreate => {
                shell.render_container_create_dialog_body(app, window, cx, p)
            }
            ModalKind::ContainerConfirm => shell.render_container_confirm_dialog_body(app, cx, p),
            ModalKind::ContainerRename => shell.render_container_rename_dialog_body(app, cx, p),
            ModalKind::ContainerTagImage => shell.render_container_tag_dialog_body(app, cx, p),
            ModalKind::ContainerCommit => shell.render_container_commit_dialog_body(app, cx, p),
            ModalKind::ContainerUpdate => shell.render_container_update_dialog_body(app, cx, p),
            ModalKind::ContainerNetworkConnect => {
                shell.render_container_network_connect_dialog_body(app, cx, p)
            }
            ModalKind::ContainerExportConfig => {
                shell.render_container_export_config_dialog_body(app, cx, p)
            }
            ModalKind::ContainerFileRename => {
                shell.render_container_file_rename_dialog_body(app, cx, p)
            }
            ModalKind::ContainerFileNewDir => {
                shell.render_container_file_new_dir_dialog_body(app, cx, p)
            }
            ModalKind::ContainerHub => shell
                .find_container_modal_tab(cx, |ws| ws.hub_dialog.is_some())
                .map_or_else(
                    || div().into_any_element(),
                    |(tab_id, _)| {
                        shell.render_container_hub_dialog_body(tab_id, app, window, cx, p)
                    },
                ),
            ModalKind::ContainerPush => shell
                .find_container_modal_tab(cx, |ws| ws.push_dialog.is_some())
                .map_or_else(
                    || div().into_any_element(),
                    |(tab_id, _)| {
                        shell.render_container_push_dialog_body(tab_id, app, window, cx, p)
                    },
                ),
            ModalKind::ContainerSystemDf => shell
                .find_container_modal_tab(cx, |ws| ws.system_df_dialog_open)
                .map_or_else(
                    || div().into_any_element(),
                    |(tab_id, _)| {
                        shell.render_container_system_df_dialog_body(tab_id, app, window, cx, p)
                    },
                ),
            ModalKind::ContainerBuild => shell
                .find_container_modal_tab(cx, |ws| ws.build_dialog.is_some())
                .map_or_else(
                    || div().into_any_element(),
                    |(tab_id, _)| {
                        shell.render_container_build_dialog_body(tab_id, app, window, cx, p)
                    },
                ),
            ModalKind::ContainerForward => shell
                .find_container_modal_tab(cx, |ws| ws.forward_dialog.is_some())
                .map_or_else(
                    || div().into_any_element(),
                    |(tab_id, _)| {
                        shell.render_container_forward_dialog_body(tab_id, app, window, cx, p)
                    },
                ),
            ModalKind::ContainerPods => shell.render_container_pods_dialog_body(app, cx, p),
            ModalKind::ContainerQuadlet => shell.render_container_quadlet_dialog_body(app, cx, p),
            // 由 [`Self::render_body`] 的实体分支处理，这里不会走到。
            ModalKind::SessionForm => div().into_any_element(),
            ModalKind::FolderForm => shell.render_folder_form_body(app, window, cx, p),
            ModalKind::BatchEdit => shell.render_batch_edit_body(app, window, cx, p),
            ModalKind::SnippetForm => shell.render_snippet_form_body(app, window, cx, p),
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
            ModalKind::ShortcutCapture => shell.render_shortcut_capture_body(app, window, cx, p),
            ModalKind::DiffViewer => shell.render_diff_viewer_body(app, window, cx, p),
            _ => div().into_any_element(),
        }
    }
}

#[cfg(test)]
mod tests;
