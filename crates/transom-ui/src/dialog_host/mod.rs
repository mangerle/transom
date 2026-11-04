//! 应用侧 Dialog 容器宿主：把原先自绘遮罩的模态弹窗接进组件库的弹窗栈。
//!
//! 设计原理（Why）
//! - 实现初衷：模态原先由 `AppShell::render_overlays` 每帧拼自绘遮罩，容器自带的
//!   焦点捕获、层序、右上角关闭件与取消/确认语义一概拿不到。若把
//!   `open_dialog`/`close_dialog` 散落到二十多个开关点去配对调用，弹窗状态与
//!   容器栈必然漂移。这里改成"期望栈仍从既有状态推导，每帧与实际栈对账"，
//!   各站点继续只写 `self.state.modals.xxx = Some(..)` / `None`。
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
use crate::palette::Palette;
use gpui_kit::base::actions::{Cancel, Confirm};
use gpui_kit::component::WindowExt;
use gpui_kit::component::dialog::DialogEntrance;
use gpui_kit::component::input::{
    Enter as InputEnter, Escape as InputEscape, InputState, TextareaState,
};
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
}

/// 单个弹窗的渲染元数据；宽度与 slug 同一处定义，新增弹窗不再漏改列表。
pub(crate) struct ModalSpec {
    pub(crate) width: Pixels,
    pub(crate) slug: &'static str,
}

impl ModalKind {
    /// 期望栈的层序：越靠后越在上层，沿用迁移前 `render_overlays` 的渲染次序，
    /// 保证遮罩层叠关系不变。
    const ORDER: &'static [ModalKind] = &[
        ModalKind::SessionForm,
        ModalKind::AuditViewer,
        ModalKind::Replay,
        ModalKind::FolderForm,
        ModalKind::AuthPrompt,
        ModalKind::HostKeyConfirm,
        ModalKind::ClipboardPrompt,
        ModalKind::KnownHosts,
        ModalKind::ForwardConfirm,
        ModalKind::Settings,
        ModalKind::ShortcutCapture,
        ModalKind::BatchEdit,
        ModalKind::BackupPrompt,
        ModalKind::KeyWizard,
        ModalKind::SftpConfirm,
        ModalKind::SftpChmod,
        ModalKind::SftpInput,
        ModalKind::SftpProperties,
        ModalKind::SftpFilter,
        ModalKind::SftpSync,
        ModalKind::EditorGotoLine,
        ModalKind::DiffViewer,
        ModalKind::ContainerForm,
        ModalKind::ContainerFolderForm,
        ModalKind::ContainerRunForm,
        ModalKind::ContainerCreate,
        ModalKind::ContainerConfirm,
        ModalKind::ContainerRename,
        ModalKind::ContainerTagImage,
        ModalKind::ContainerCommit,
        ModalKind::ContainerUpdate,
        ModalKind::ContainerNetworkConnect,
        ModalKind::ContainerExportConfig,
        ModalKind::ContainerFileRename,
        ModalKind::ContainerFileNewDir,
        ModalKind::ContainerHub,
        ModalKind::ContainerPush,
        ModalKind::SftpSearch,
        ModalKind::SnippetForm,
    ];

    /// 弹窗元数据：宽度与元素标识。
    ///
    /// 宽度沿用迁移前的档位；除设置、密钥向导、同步弹窗与差异对比外都是固定
    /// 像素。差异对比按视口比例，设置弹窗按密度 rem 缩放，二者需要窗口尺寸，
    /// 因此这里以 `Window` 为参数，而不是把宽度散落回各调用点。
    fn spec(self, window: &Window) -> ModalSpec {
        let rem = window.rem_size();
        let (px_width, slug) = match self {
            ModalKind::AuthPrompt => (Some(crate::theme::tokens::dialog::AUTH), "auth"),
            ModalKind::HostKeyConfirm => (Some(crate::theme::tokens::dialog::AUTH), "host-key"),
            ModalKind::ClipboardPrompt => {
                (Some(crate::theme::tokens::dialog::CLIPBOARD), "clipboard")
            }
            ModalKind::ForwardConfirm => (Some(crate::theme::tokens::dialog::CONFIRM), "forward"),
            ModalKind::SftpConfirm => (Some(crate::theme::tokens::dialog::CONFIRM), "sftp-confirm"),
            ModalKind::SftpInput => (Some(crate::theme::tokens::dialog::INPUT), "sftp-input"),
            ModalKind::SftpChmod => (Some(crate::theme::tokens::dialog::CHMOD), "sftp-chmod"),
            ModalKind::SftpProperties => {
                (Some(crate::theme::tokens::dialog::CHMOD), "sftp-properties")
            }
            ModalKind::SftpFilter => (Some(crate::theme::tokens::dialog::FILTER), "sftp-filter"),
            ModalKind::SftpSearch => (Some(crate::theme::tokens::dialog::SEARCH), "sftp-search"),
            // 同步弹窗与密钥向导用全局弹窗宽度上限档位，随密度缩放。
            ModalKind::SftpSync => (None, "sftp-sync"),
            ModalKind::KeyWizard => (None, "key-wizard"),
            ModalKind::EditorGotoLine => (
                Some(crate::theme::tokens::dialog::EDITOR),
                "editor-goto-line",
            ),
            ModalKind::ContainerForm => {
                (Some(crate::theme::tokens::dialog::FILTER), "container-form")
            }
            ModalKind::ContainerFolderForm => (
                Some(crate::theme::tokens::dialog::INPUT),
                "container-folder-form",
            ),
            ModalKind::ContainerRunForm => {
                (Some(crate::theme::tokens::dialog::FILTER), "container-run")
            }
            ModalKind::ContainerCreate => (
                Some(crate::theme::tokens::dialog::CONFIRM),
                "container-create",
            ),
            ModalKind::ContainerConfirm => {
                (Some(crate::theme::tokens::dialog::CONFIRM), "container-confirm")
            }
            ModalKind::ContainerRename => {
                (Some(crate::theme::tokens::dialog::INPUT), "container-rename")
            }
            ModalKind::ContainerTagImage => {
                (Some(crate::theme::tokens::dialog::INPUT), "container-tag-image")
            }
            ModalKind::ContainerCommit => {
                (Some(crate::theme::tokens::dialog::FILTER), "container-commit")
            }
            ModalKind::ContainerUpdate => {
                (Some(crate::theme::tokens::dialog::INPUT), "container-update")
            }
            ModalKind::ContainerNetworkConnect => (
                Some(crate::theme::tokens::dialog::INPUT),
                "container-network-connect",
            ),
            ModalKind::ContainerExportConfig => (
                Some(crate::theme::tokens::dialog::AUDIT),
                "container-export-config",
            ),
            ModalKind::ContainerFileRename => (
                Some(crate::theme::tokens::dialog::INPUT),
                "container-file-rename",
            ),
            ModalKind::ContainerFileNewDir => (
                Some(crate::theme::tokens::dialog::INPUT),
                "container-file-new-dir",
            ),
            ModalKind::ContainerHub => (
                Some(crate::theme::tokens::dialog::AUDIT),
                "container-hub",
            ),
            ModalKind::ContainerPush => (
                Some(crate::theme::tokens::dialog::FILTER),
                "container-push",
            ),
            ModalKind::SessionForm => (
                Some(crate::theme::tokens::dialog::SESSION_FORM),
                "session-form",
            ),
            ModalKind::FolderForm => (
                Some(crate::theme::tokens::dialog::FOLDER_FORM),
                "folder-form",
            ),
            ModalKind::BatchEdit => (Some(crate::theme::tokens::dialog::BATCH_EDIT), "batch-edit"),
            ModalKind::SnippetForm => (
                Some(crate::theme::tokens::dialog::SNIPPET_FORM),
                "snippet-form",
            ),
            ModalKind::KnownHosts => (
                Some(crate::theme::tokens::dialog::KNOWN_HOSTS),
                "known-hosts",
            ),
            ModalKind::BackupPrompt => {
                (Some(crate::theme::tokens::dialog::BACKUP), "backup-prompt")
            }
            ModalKind::AuditViewer => (Some(crate::theme::tokens::dialog::AUDIT), "audit-viewer"),
            ModalKind::Replay => (Some(crate::theme::tokens::dialog::REPLAY), "replay"),
            // 设置弹窗宽度按密度档位；差异对比按视口比例（迁移前 relative(0.94)）。
            ModalKind::Settings => (
                Some(crate::theme::tokens::size::SETTINGS_DIALOG_WIDTH),
                "settings",
            ),
            ModalKind::ShortcutCapture => {
                (Some(crate::theme::tokens::dialog::AUTH), "shortcut-capture")
            }
            ModalKind::DiffViewer => {
                return ModalSpec {
                    width: window.viewport_size().width * 0.94,
                    slug: "diff-viewer",
                };
            }
        };
        ModalSpec {
            width: px_width.map_or(
                rem * crate::theme::tokens::size::DIALOG_MAX_WIDTH,
                |width| rem * width,
            ),
            slug,
        }
    }
}

mod body;
mod container;
mod stack;
mod state;

pub(crate) use body::{dialog_bindings, on_event};
pub(crate) use stack::OpenModals;

impl ModalKind {
    /// 弹窗体：内容与页脚仍由各领域自己渲染。
    /// 设置弹窗需要先拿实体句柄再进入 `update`，其它弹窗直接读外壳。
    ///
    /// 分开两条路径是为了避免“`app.read(cx)` 的只读借用一直活到设置实体的
    /// 可变 `update`”这类借用冲突：设置分支先克隆句柄、立即结束外壳借用。
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
        let shell = app.read(cx);
        self.render_shell_body(shell, app, window, cx, p)
    }

    /// 非设置弹窗的渲染分发：只读外壳状态。
    fn render_shell_body(
        self,
        shell: &AppShell,
        app: &Entity<AppShell>,
        window: &mut Window,
        cx: &App,
        p: &Palette,
    ) -> AnyElement {
        match self {
            ModalKind::SftpConfirm => shell.render_sftp_confirm_body(app, cx, p),
            ModalKind::HostKeyConfirm => shell.render_host_key_confirm_body(app, cx, p),
            ModalKind::ClipboardPrompt => shell.render_clipboard_prompt_body(app, cx, p),
            ModalKind::ForwardConfirm => shell.render_forward_confirm_body(app, cx, p),
            ModalKind::AuthPrompt => shell.render_auth_prompt_body(app, cx, p),
            ModalKind::SftpInput => shell.render_sftp_input_body(app, cx, p),
            ModalKind::SftpChmod => shell.render_sftp_chmod_body(app, cx, p),
            ModalKind::SftpProperties => shell.render_sftp_properties_body(app, cx, p),
            ModalKind::SftpFilter => shell.render_sftp_filter_body(app, cx, p),
            ModalKind::SftpSearch => shell.render_sftp_search_body(app, cx, p),
            ModalKind::SftpSync => shell.render_sftp_sync_body(app, cx, p),
            ModalKind::EditorGotoLine => shell.render_editor_goto_line_body(app, cx, p),
            ModalKind::ContainerForm => shell.render_container_form_body(app, window, cx, p),
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
            ModalKind::ContainerHub => {
                if let Some((tab_id, _)) =
                    shell.find_container_modal_tab(|ws| ws.hub_dialog.is_some())
                {
                    shell.render_container_hub_dialog_body(tab_id, app, window, cx, p)
                } else {
                    div().into_any_element()
                }
            }
            ModalKind::ContainerPush => {
                if let Some((tab_id, _)) =
                    shell.find_container_modal_tab(|ws| ws.push_dialog.is_some())
                {
                    shell.render_container_push_dialog_body(tab_id, app, window, cx, p)
                } else {
                    div().into_any_element()
                }
            }
            ModalKind::SessionForm => shell.render_session_form_body(app, window, cx, p),
            ModalKind::FolderForm => shell.render_folder_form_body(app, window, cx, p),
            ModalKind::BatchEdit => shell.render_batch_edit_body(app, window, cx, p),
            ModalKind::SnippetForm => shell.render_snippet_form_body(app, window, cx, p),
            ModalKind::KnownHosts => shell.render_known_hosts_body(app, window, cx, p),
            ModalKind::KeyWizard => shell.render_key_wizard_body(app, window, cx, p),
            ModalKind::BackupPrompt => shell.render_backup_prompt_body(app, window, cx, p),
            ModalKind::AuditViewer => shell.render_audit_viewer_body(app, window, cx, p),
            ModalKind::Replay => shell.render_replay_body(app, window, cx, p),
            ModalKind::Settings => {
                // 由 [`Self::render_body`] 的专用分支处理，这里不会走到。
                div().into_any_element()
            }
            ModalKind::ShortcutCapture => shell.render_shortcut_capture_body(app, window, cx, p),
            ModalKind::DiffViewer => shell.render_diff_viewer_body(app, window, cx, p),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ModalKind;

    /// 全部弹窗种类；新增变体时必须同步本表，顺序测试会校验 `ORDER` 不漏不重。
    const ALL: &[ModalKind] = &[
        ModalKind::AuthPrompt,
        ModalKind::HostKeyConfirm,
        ModalKind::ClipboardPrompt,
        ModalKind::ForwardConfirm,
        ModalKind::SftpConfirm,
        ModalKind::SftpInput,
        ModalKind::SftpChmod,
        ModalKind::SftpProperties,
        ModalKind::SftpFilter,
        ModalKind::SftpSearch,
        ModalKind::SftpSync,
        ModalKind::EditorGotoLine,
        ModalKind::ContainerForm,
        ModalKind::ContainerFolderForm,
        ModalKind::ContainerRunForm,
        ModalKind::ContainerCreate,
        ModalKind::ContainerConfirm,
        ModalKind::ContainerRename,
        ModalKind::ContainerTagImage,
        ModalKind::ContainerCommit,
        ModalKind::ContainerUpdate,
        ModalKind::ContainerNetworkConnect,
        ModalKind::ContainerExportConfig,
        ModalKind::ContainerFileRename,
        ModalKind::ContainerFileNewDir,
        ModalKind::ContainerHub,
        ModalKind::ContainerPush,
        ModalKind::SessionForm,
        ModalKind::FolderForm,
        ModalKind::BatchEdit,
        ModalKind::SnippetForm,
        ModalKind::KnownHosts,
        ModalKind::KeyWizard,
        ModalKind::BackupPrompt,
        ModalKind::AuditViewer,
        ModalKind::Replay,
        ModalKind::Settings,
        ModalKind::ShortcutCapture,
        ModalKind::DiffViewer,
    ];

    #[test]
    fn order_covers_every_kind_exactly_once() {
        assert_eq!(
            ALL.len(),
            ModalKind::ORDER.len(),
            "ORDER 漏掉或多出弹窗种类"
        );
        for kind in ALL {
            let count = ModalKind::ORDER.iter().filter(|item| *item == kind).count();
            assert_eq!(count, 1, "弹窗种类 {:?} 在 ORDER 中必须恰好出现一次", kind);
        }
    }
}
