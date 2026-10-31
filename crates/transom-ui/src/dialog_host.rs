//! 应用侧 Dialog 容器宿主：把原先自绘遮罩的模态弹窗接进组件库的弹窗栈。
//!
//! 设计原理（Why）
//! - 实现初衷：模态原先由 `AppShell::render_overlays` 每帧拼自绘遮罩，容器自带的
//!   焦点捕获、层序、右上角关闭件与取消/确认语义一概拿不到。若把
//!   `open_dialog`/`close_dialog` 散落到二十多个开关点去配对调用，弹窗状态与
//!   容器栈必然漂移。这里改成"期望栈仍从既有状态推导，每帧与实际栈对账"，
//!   各站点继续只写 `self.modals.xxx = Some(..)` / `None`。
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
        ModalKind::SftpSearch,
        ModalKind::SnippetForm,
    ];

    /// 该弹窗此刻是否应当存在。只读状态、不产生副作用：对账每帧都会问一次。
    fn is_open(self, shell: &AppShell) -> bool {
        match self {
            ModalKind::AuthPrompt => shell.modals.auth_prompt.is_some(),
            ModalKind::HostKeyConfirm => shell.modals.host_key_confirm.is_some(),
            ModalKind::ClipboardPrompt => shell.modals.clipboard_request.is_some(),
            ModalKind::ForwardConfirm => shell.modals.confirm_forward_any.is_some(),
            ModalKind::SftpConfirm => shell.sftp_modals.confirm.is_some(),
            ModalKind::SftpInput => shell.sftp_modals.input.is_some(),
            ModalKind::SftpChmod => shell.sftp_modals.chmod.is_some(),
            ModalKind::SftpProperties => shell.sftp_modals.properties.is_some(),
            ModalKind::SftpFilter => shell.sftp_modals.filter.is_some(),
            ModalKind::SftpSearch => shell.sftp_modals.search.is_some(),
            ModalKind::SftpSync => shell.sftp_modals.sync.is_some(),
            ModalKind::EditorGotoLine => shell.sftp_modals.goto_line.is_some(),
            ModalKind::SessionForm => shell.modals.form.is_some(),
            ModalKind::FolderForm => shell.modals.folder_form.is_some(),
            ModalKind::BatchEdit => shell.modals.batch_edit.is_some(),
            ModalKind::SnippetForm => shell.modals.snippet_panel.form.is_some(),
            ModalKind::KnownHosts => shell.modals.known_hosts.open,
            ModalKind::KeyWizard => shell.modals.settings.key_wizard.is_some(),
            ModalKind::BackupPrompt => shell.modals.backup_prompt.is_some(),
            ModalKind::AuditViewer => shell.modals.audit_viewer.is_some(),
            ModalKind::Replay => shell.modals.replay.is_some(),
            ModalKind::Settings => shell.modals.settings.open,
            ModalKind::ShortcutCapture => shell.shortcut_recording.capture.is_some(),
            ModalKind::DiffViewer => shell.diff_viewer.is_some(),
        }
    }

    /// 弹窗宽度：沿用各自迁移前的档位，不用容器默认值，避免大面板被改小。
    ///
    /// 参数是窗口 rem 基准：同步弹窗原先按 `rems(DIALOG_MAX_WIDTH)` 表达，
    /// 需要随密度档位等比缩放，其余档位是固定像素。
    fn width(self, window: &Window) -> Pixels {
        let rem = window.rem_size();
        let px_width = match self {
            ModalKind::AuthPrompt => Some(440.0),
            ModalKind::HostKeyConfirm => Some(440.0),
            ModalKind::ClipboardPrompt => Some(520.0),
            ModalKind::ForwardConfirm => Some(460.0),
            ModalKind::SftpConfirm => Some(460.0),
            ModalKind::SftpInput => Some(420.0),
            ModalKind::SftpChmod => Some(520.0),
            ModalKind::SftpProperties => Some(520.0),
            ModalKind::SftpFilter => Some(560.0),
            ModalKind::SftpSearch => Some(560.0),
            // 同步弹窗与密钥向导用全局弹窗宽度上限档位，随密度缩放。
            ModalKind::SftpSync | ModalKind::KeyWizard => None,
            ModalKind::EditorGotoLine => Some(420.0),
            ModalKind::SessionForm => Some(620.0),
            ModalKind::FolderForm => Some(480.0),
            ModalKind::BatchEdit => Some(560.0),
            ModalKind::SnippetForm => Some(420.0),
            ModalKind::KnownHosts => Some(580.0),
            ModalKind::BackupPrompt => Some(520.0),
            ModalKind::AuditViewer => Some(900.0),
            ModalKind::Replay => Some(560.0),
            // 设置弹窗宽度按密度档位；差异对比按视口比例（迁移前 relative(0.94)）。
            ModalKind::Settings => {
                Some(rem.as_f32() * crate::theme::tokens::size::SETTINGS_DIALOG_WIDTH)
            }
            ModalKind::ShortcutCapture => Some(440.0),
            ModalKind::DiffViewer => return window.viewport_size().width * 0.94,
        };
        px_width.map_or(rem * crate::theme::tokens::size::DIALOG_MAX_WIDTH, px)
    }

    /// 元素标识用，便于无障碍与调试定位。
    fn slug(self) -> &'static str {
        match self {
            ModalKind::AuthPrompt => "auth",
            ModalKind::HostKeyConfirm => "host-key",
            ModalKind::ClipboardPrompt => "clipboard",
            ModalKind::ForwardConfirm => "forward",
            ModalKind::SftpConfirm => "sftp-confirm",
            ModalKind::SftpInput => "sftp-input",
            ModalKind::SftpChmod => "sftp-chmod",
            ModalKind::SftpProperties => "sftp-properties",
            ModalKind::SftpFilter => "sftp-filter",
            ModalKind::SftpSearch => "sftp-search",
            ModalKind::SftpSync => "sftp-sync",
            ModalKind::EditorGotoLine => "editor-goto-line",
            ModalKind::SessionForm => "session-form",
            ModalKind::FolderForm => "folder-form",
            ModalKind::BatchEdit => "batch-edit",
            ModalKind::SnippetForm => "snippet-form",
            ModalKind::KnownHosts => "known-hosts",
            ModalKind::KeyWizard => "key-wizard",
            ModalKind::BackupPrompt => "backup-prompt",
            ModalKind::AuditViewer => "audit-viewer",
            ModalKind::Replay => "replay",
            ModalKind::Settings => "settings",
            ModalKind::ShortcutCapture => "shortcut-capture",
            ModalKind::DiffViewer => "diff-viewer",
        }
    }

    /// 是否存在未提交内容：与影子栈记录的打开时基线逐项比对。
    ///
    /// 为什么用基线快照而不是"用户改过就算脏"的标记位：文本框的值由组件自己
    /// 维护，逐处加脏标记要把每个输入框都接一遍回调，漏一处就出现"改了不拦"；
    /// 快照比对只看结果，新增字段不会漏。代价是每次取消都要读一遍全部文本项，
    /// 量级是二十来个字符串，可忽略。
    fn is_dirty(self, shell: &AppShell, cx: &App) -> bool {
        let current = self.field_values(shell, cx);
        !current.is_empty() && current != shell.open_modals.baseline(self)
    }

    /// 该种类需要跟踪的文本项；返回空表示不参与脏判定（确认与授权类）。
    fn field_values(self, shell: &AppShell, cx: &App) -> Vec<String> {
        match self {
            ModalKind::SessionForm => shell
                .modals
                .form
                .as_ref()
                .map(|form| form.text_values(cx))
                .unwrap_or_default(),
            ModalKind::FolderForm => shell
                .modals
                .folder_form
                .as_ref()
                .map(|form| form.text_values(cx))
                .unwrap_or_default(),
            ModalKind::BatchEdit => shell
                .modals
                .batch_edit
                .as_ref()
                .map(|form| form.text_values(cx))
                .unwrap_or_default(),
            ModalKind::SnippetForm => shell
                .modals
                .snippet_panel
                .form
                .as_ref()
                .map(|form| form.text_values(cx))
                .unwrap_or_default(),
            // 设置页的控件是开关、下拉与滑块，不是文本框，逐控件加脏标记要改
            // 遍所有设置页且新增控件必漏；这里直接比整份设置的序列化快照，
            // 覆盖全字段。序列化失败按"不脏"处理：宁可少拦一次，也不能让
            // Esc 永远关不掉弹窗。
            ModalKind::Settings => serde_json::to_string(&shell.settings)
                .map(|signature| vec![signature])
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    /// 取消：把用户意图回给等待方并清空自身状态，返回是否允许容器关闭该层。
    ///
    /// 授权类必须走各自的 cancel 方法把结果发回 responder：只清状态会让
    /// 等待裁决的在途连接任务永久挂住。
    /// 返回 `true` 表示取消已被执行、这一层允许关闭；脏表单返回 `false` 并写提示。
    fn cancel(self, shell: &mut AppShell, window: &mut Window, cx: &mut Context<AppShell>) -> bool {
        if self.is_dirty(shell, cx) {
            // 拦截必须可见：文案说明为什么按了没反应，行为与提示一致。
            shell.dialog_hint = Some(rust_i18n::t!("dialog.dirty_block_hint").to_string());
            cx.notify();
            return false;
        }
        shell.dialog_hint = None;
        match self {
            ModalKind::SftpConfirm => {
                shell.sftp_modals.confirm = None;
                shell.focus_active_sftp_panel(window, cx);
                true
            }
            ModalKind::HostKeyConfirm => {
                shell.reject_host_key(cx);
                true
            }
            ModalKind::ClipboardPrompt => {
                shell.deny_clipboard_write(cx);
                true
            }
            ModalKind::ForwardConfirm => {
                shell.cancel_forward_confirm(cx);
                true
            }
            ModalKind::AuthPrompt => {
                shell.cancel_auth_prompt();
                true
            }
            ModalKind::SftpInput => shell.close_sftp_modal(|hub| hub.input = None, window, cx),
            ModalKind::SftpChmod => shell.close_sftp_modal(|hub| hub.chmod = None, window, cx),
            ModalKind::SftpProperties => {
                shell.close_sftp_modal(|hub| hub.properties = None, window, cx)
            }
            ModalKind::SftpFilter => shell.close_sftp_modal(|hub| hub.filter = None, window, cx),
            ModalKind::SftpSearch => shell.close_sftp_modal(|hub| hub.search = None, window, cx),
            ModalKind::SftpSync => shell.close_sftp_modal(|hub| hub.sync = None, window, cx),
            ModalKind::EditorGotoLine => {
                shell.cancel_editor_goto_line(window, cx);
                true
            }
            ModalKind::SessionForm => {
                shell.modals.form = None;
                true
            }
            ModalKind::FolderForm => {
                shell.modals.folder_form = None;
                true
            }
            ModalKind::BatchEdit => {
                shell.modals.batch_edit = None;
                true
            }
            ModalKind::SnippetForm => {
                shell.modals.snippet_panel.form = None;
                true
            }
            ModalKind::KnownHosts => {
                shell.modals.known_hosts.open = false;
                true
            }
            ModalKind::KeyWizard => {
                shell.modals.settings.key_wizard = None;
                true
            }
            ModalKind::BackupPrompt => {
                shell.modals.backup_prompt = None;
                true
            }
            ModalKind::AuditViewer => {
                shell.modals.audit_viewer = None;
                true
            }
            ModalKind::Replay => {
                shell.modals.replay = None;
                true
            }
            ModalKind::Settings => {
                shell.cancel_settings(window, cx);
                true
            }
            ModalKind::ShortcutCapture => {
                shell.shortcut_recording.capture = None;
                true
            }
            ModalKind::DiffViewer => {
                shell.close_diff_viewer(cx);
                true
            }
        }
    }

    /// 提交：等价于点主按钮。
    ///
    /// 各领域方法自己负责把状态置回 `None`（提交成功）或保留弹窗并写错误：
    /// 关闭与否一律交给对账器按期望栈收敛，不让容器先弹栈，否则校验失败时
    /// 状态仍在、下一帧又按状态把弹窗重开一次。
    /// 提交：执行主按钮动作，然后按"该弹窗状态是否已消失"决定是否允许关闭。
    ///
    /// 为什么用状态反查而不是让每个领域返回布尔：校验失败、保存失败、需要继续
    /// 填的表单都会把状态留在原处，动作函数本身并不统一返回成败；而"状态还在"
    /// 恰恰就是弹窗该继续存在的唯一事实。
    fn commit(self, shell: &mut AppShell, window: &mut Window, cx: &mut Context<AppShell>) -> bool {
        self.run_commit(shell, window, cx);
        !self.is_open(shell)
    }

    fn run_commit(self, shell: &mut AppShell, window: &mut Window, cx: &mut Context<AppShell>) {
        match self {
            ModalKind::SftpConfirm => {
                shell.sftp_confirm_delete(cx);
                shell.focus_active_sftp_panel(window, cx);
            }
            ModalKind::HostKeyConfirm => shell.accept_host_key(cx),
            ModalKind::ClipboardPrompt => shell.confirm_clipboard_write(cx),
            ModalKind::ForwardConfirm => shell.confirm_forward_start(cx),
            ModalKind::AuthPrompt => shell.submit_auth_prompt(cx),
            ModalKind::SftpInput => {
                shell.commit_sftp_input_modal(window, cx);
                shell.focus_active_sftp_panel(window, cx);
            }
            ModalKind::SftpChmod => shell.save_sftp_chmod_modal(window, cx),
            // 属性与搜索弹窗只有"关闭"这一个主按钮，Enter 与它同义。
            ModalKind::SftpProperties => {
                shell.close_sftp_modal(|hub| hub.properties = None, window, cx);
            }
            ModalKind::SftpSearch => {
                shell.close_sftp_modal(|hub| hub.search = None, window, cx);
            }
            ModalKind::SftpFilter => shell.save_sftp_filter_modal(window, cx),
            ModalKind::SftpSync => shell.start_sftp_sync(window, cx),
            ModalKind::EditorGotoLine => shell.commit_editor_goto_line(window, cx),
            ModalKind::SessionForm => shell.save_session_form(cx),
            ModalKind::FolderForm => shell.save_folder_form(cx),
            ModalKind::BatchEdit => shell.save_batch_edit(cx),
            ModalKind::SnippetForm => shell.save_snippet_form(cx),
            ModalKind::BackupPrompt => shell.submit_backup(window, cx),
            // 查看与管理类弹窗只有一个"关闭"主按钮，Enter 与它同义。
            ModalKind::KnownHosts => shell.modals.known_hosts.open = false,
            ModalKind::KeyWizard => shell.modals.settings.key_wizard = None,
            ModalKind::AuditViewer => shell.modals.audit_viewer = None,
            ModalKind::Replay => shell.modals.replay = None,
            ModalKind::Settings => shell.cancel_settings(window, cx),
            ModalKind::ShortcutCapture => shell.shortcut_recording.capture = None,
            ModalKind::DiffViewer => shell.close_diff_viewer(cx),
        }
    }

    /// 首帧焦点：文本类弹窗把焦点交给自己的输入控件，选择类交给外壳。
    ///
    /// 一律走 `as_ref()` 安全取值：关闭动作可能在对账把弹窗弹出栈之前就把状态
    /// 清空，这一帧仍可能渲染一次；用 `expect` 会把这条正常路径变成 panic。
    fn focus_target(self, shell: &AppShell) -> ModalFocus {
        let input =
            |entity: Option<Entity<InputState>>| entity.map_or(ModalFocus::None, ModalFocus::Input);
        match self {
            ModalKind::SftpInput => input(
                shell
                    .sftp_modals
                    .input
                    .as_ref()
                    .map(|modal| modal.input.clone()),
            ),
            ModalKind::SftpChmod => input(
                shell
                    .sftp_modals
                    .chmod
                    .as_ref()
                    .map(|modal| modal.octal_input.clone()),
            ),
            ModalKind::SftpSearch => input(
                shell
                    .sftp_modals
                    .search
                    .as_ref()
                    .map(|modal| modal.input.clone()),
            ),
            ModalKind::EditorGotoLine => input(
                shell
                    .sftp_modals
                    .goto_line
                    .as_ref()
                    .map(|modal| modal.input.clone()),
            ),
            ModalKind::SessionForm => {
                input(shell.modals.form.as_ref().map(|form| form.name.clone()))
            }
            ModalKind::FolderForm => input(
                shell
                    .modals
                    .folder_form
                    .as_ref()
                    .map(|form| form.name.clone()),
            ),
            ModalKind::SnippetForm => input(
                shell
                    .modals
                    .snippet_panel
                    .form
                    .as_ref()
                    .map(|form| form.name_input()),
            ),
            ModalKind::SftpFilter => shell
                .sftp_modals
                .filter
                .as_ref()
                .map_or(ModalFocus::None, |modal| {
                    ModalFocus::Textarea(modal.textarea.clone())
                }),
            ModalKind::SftpConfirm
            | ModalKind::SftpProperties
            | ModalKind::SftpSync
            | ModalKind::HostKeyConfirm
            | ModalKind::ClipboardPrompt
            | ModalKind::ForwardConfirm
            | ModalKind::AuditViewer
            | ModalKind::Replay
            | ModalKind::BatchEdit
            | ModalKind::KnownHosts
            | ModalKind::KeyWizard
            | ModalKind::BackupPrompt
            | ModalKind::Settings
            | ModalKind::DiffViewer
            | ModalKind::AuthPrompt => ModalFocus::None,
            // 改键录入面板自己持有焦点句柄：录键期间按键要落在这块面板上。
            ModalKind::ShortcutCapture => shell
                .shortcut_recording
                .capture
                .as_ref()
                .map_or(ModalFocus::None, |capture| {
                    ModalFocus::Handle(capture.focus.clone())
                }),
        }
    }

    /// 弹窗体：内容与页脚仍由各领域自己渲染。
    fn render_body(
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
            ModalKind::SessionForm => shell.render_session_form_body(app, window, cx, p),
            ModalKind::FolderForm => shell.render_folder_form_body(app, window, cx, p),
            ModalKind::BatchEdit => shell.render_batch_edit_body(app, window, cx, p),
            ModalKind::SnippetForm => shell.render_snippet_form_body(app, window, cx, p),
            ModalKind::KnownHosts => shell.render_known_hosts_body(app, window, cx, p),
            ModalKind::KeyWizard => shell.render_key_wizard_body(app, window, cx, p),
            ModalKind::BackupPrompt => shell.render_backup_prompt_body(app, window, cx, p),
            ModalKind::AuditViewer => shell.render_audit_viewer_body(app, window, cx, p),
            ModalKind::Replay => shell.render_replay_body(app, window, cx, p),
            ModalKind::Settings => shell.render_settings_body(app, window, cx, p),
            ModalKind::ShortcutCapture => shell.render_shortcut_capture_body(app, window, cx, p),
            ModalKind::DiffViewer => shell.render_diff_viewer_body(app, window, cx, p),
        }
    }
}

/// 弹窗打开后要把焦点交给谁。
///
/// 为什么需要：`open_dialog` 会把焦点移到弹窗容器上，站点原先在打开时写的
/// `input.focus(..)` 会被这一步覆盖，表现为一打开就打字没反应。这里由弹窗体
/// 在首帧按种类取回焦点，和容器抢焦点的顺序也就此固定下来。
pub(crate) enum ModalFocus {
    Input(Entity<InputState>),
    Textarea(Entity<TextareaState>),
    /// 面板自己持有的焦点句柄（如改键录入面板）。
    Handle(FocusHandle),
    /// 没有文本输入控件：焦点留在弹窗外壳，交给 Tab 键遍历内部控件。
    None,
}

/// 弹窗体实体：随外壳状态自绘。
///
/// 为什么必须是独立实体：弹窗层由组件库内部状态驱动重建，宿主 `cx.notify()`
/// 只会重绘外壳子树、不会重跑 `open_dialog` 的 builder；实体自己脏了才会重绘。
/// 实测依据见 `tests/dialog_key_probe.rs::宿主变脏时弹窗体独立重绘而弹窗层不重建`。
struct ModalBody {
    kind: ModalKind,
    app: Entity<AppShell>,
    /// 首帧需要把焦点交给输入控件，之后不再干预用户的焦点移动。
    pending_focus: bool,
    /// 保留订阅：外壳变更后让本实体脏掉重绘；丢弃即失联。
    _observe: Subscription,
}

impl Render for ModalBody {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::from(cx);
        let app = self.app.clone();
        let kind = self.kind;
        if self.pending_focus {
            self.pending_focus = false;
            match kind.focus_target(app.read(cx)) {
                ModalFocus::Input(input) => {
                    input.update(cx, |state, cx| state.focus(window, cx));
                }
                ModalFocus::Textarea(textarea) => {
                    textarea.update(cx, |state, cx| state.focus(window, cx));
                }
                ModalFocus::Handle(handle) => handle.focus(window, cx),
                ModalFocus::None => {}
            }
        }
        let body = kind.render_body(app.read(cx), &app, window, cx, &p);
        let hint = app.read(cx).dialog_hint.clone();
        div()
            .id(ElementId::Name(
                format!("modal-body-{}", kind.slug()).into(),
            ))
            // 两路并存：焦点在输入框里时按键归输入框（容器那条键位不生效），
            // 焦点在弹窗外壳或按钮上时按键归容器动作；四种落点都指向同一条
            // 取消/提交逻辑。截住传播，避免焦点位于弹窗内部时容器同名动作
            // 再被执行一次。
            .on_action({
                let app = app.clone();
                move |_: &InputEscape, window, cx| {
                    if cancel_modal(&app, kind, window, cx) {
                        close_layer(&app, kind, cx);
                        window.close_dialog(cx);
                    }
                    cx.stop_propagation();
                }
            })
            .on_action({
                let app = app.clone();
                move |_: &InputEnter, window, cx| {
                    if commit_modal(&app, kind, window, cx) {
                        close_layer(&app, kind, cx);
                        window.close_dialog(cx);
                    }
                    cx.stop_propagation();
                }
            })
            .on_action({
                // 焦点落在弹窗内部控件上时，容器自己的 Cancel 处理还在更外层，
                // 这里先执行并截住，避免两条取消链各跑一次。
                let app = app.clone();
                move |_: &Cancel, window, cx| {
                    if cancel_modal(&app, kind, window, cx) {
                        close_layer(&app, kind, cx);
                        window.close_dialog(cx);
                    }
                    cx.stop_propagation();
                }
            })
            .on_action({
                let app = app.clone();
                move |_: &Confirm, window, cx| {
                    if commit_modal(&app, kind, window, cx) {
                        close_layer(&app, kind, cx);
                        window.close_dialog(cx);
                    }
                    cx.stop_propagation();
                }
            })
            .child(body)
            .children(hint.map(|text| {
                div()
                    .text_xs()
                    .text_color(Palette::from(cx).danger)
                    .child(text)
            }))
    }
}

/// 取消一路：执行领域取消逻辑，返回是否允许关闭这一层。
fn cancel_modal(
    app: &Entity<AppShell>,
    kind: ModalKind,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    app.update(cx, |shell, cx| kind.cancel(shell, window, cx))
}

/// 提交一路：执行领域提交逻辑，返回是否允许关闭这一层。
fn commit_modal(
    app: &Entity<AppShell>,
    kind: ModalKind,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    app.update(cx, |shell, cx| kind.commit(shell, window, cx))
}

/// 立即关掉一层弹窗。
///
/// 为什么不等下一帧对账：等一帧会让"按下 Esc"多一次外壳重绘，观感上就是
/// 先卡一下再闪没。这里同步摘掉影子栈条目，容器与影子保持一致，
/// 下一帧对账不会重复弹栈。
fn close_layer(app: &Entity<AppShell>, kind: ModalKind, cx: &mut App) {
    app.update(cx, |shell, cx| {
        shell.open_modals.drop(kind);
        // 压暗层的显隐由影子栈决定，必须随本层一起落定：少这一次通知，
        // 弹窗没了而整条压暗还挂在屏幕上。
        shell.dialog_hint = None;
        cx.notify();
    });
}

/// 已打开弹窗的影子栈：与容器实际栈同序，逐帧对账的依据。
#[derive(Default)]
pub(crate) struct OpenModals {
    /// 每层记录三样：种类、弹窗体实体、打开时的文本基线。
    layers: Vec<ModalLayer>,
}

/// 影子栈的一层。
pub(crate) struct ModalLayer {
    kind: ModalKind,
    body: Entity<ModalBody>,
    /// 打开那一刻的文本项快照，脏判定与它比对。
    baseline: Vec<String>,
}

impl OpenModals {
    /// 摘掉某一层影子记录（容器已自行弹栈、或即将由调用方弹栈时调用）。
    fn drop(&mut self, kind: ModalKind) {
        if let Some(index) = self.layers.iter().position(|layer| layer.kind == kind) {
            self.layers.remove(index);
        }
    }

    /// 取某一层的文本基线；该层不存在时返回空切片（此时脏判定恒为假）。
    fn baseline(&self, kind: ModalKind) -> &[String] {
        self.layers
            .iter()
            .find(|layer| layer.kind == kind)
            .map_or(&[][..], |layer| layer.baseline.as_slice())
    }
}

impl AppShell {
    /// 当前应当打开的弹窗栈，按层序由下到上。
    fn desired_modal_stack(&self) -> Vec<ModalKind> {
        ModalKind::ORDER
            .iter()
            .copied()
            .filter(|kind| kind.is_open(self))
            .collect()
    }

    /// 把容器弹窗栈对账到期望栈。
    ///
    /// 为什么放在渲染里而不是各个开关点：状态变更入口分散在菜单、快捷键、
    /// 拖放与异步回调等二十多处，逐个补开关必然漏；渲染是唯一的汇聚点。
    /// 为什么要 `defer`：`open_dialog` 会移交焦点并通知组件层，在渲染过程中
    /// 改这些状态会踩借用与重入，延到绘制之后再执行。
    pub(crate) fn reconcile_dialogs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let desired = self.desired_modal_stack();
        let actual: Vec<ModalKind> = self
            .open_modals
            .layers
            .iter()
            .map(|layer| layer.kind)
            .collect();
        if desired == actual {
            return;
        }

        // 公共前缀原地保留；一旦中间层被移除或需要在中间插入，只能整栈
        // 关掉再按期望顺序重开——这是容器栈的真实代价。
        let common = desired
            .iter()
            .zip(actual.iter())
            .take_while(|(want, have)| want == have)
            .count();
        let extra = actual.len() - common;
        let missing = desired[common..].to_vec();

        let app = cx.entity().clone();
        let deferred_app = app.clone();
        let deferred_missing = missing.clone();
        window.defer(cx, move |window, cx| {
            for _ in 0..extra {
                window.close_dialog(cx);
            }
            for kind in deferred_missing {
                deferred_app.update(cx, |shell, cx| shell.push_dialog(kind, window, cx));
            }
        });
        // 影子栈先按期望落定：延后执行的那批开关完成后即与之一致。
        self.open_modals.layers.truncate(common);
        for kind in missing {
            // 基线在开弹窗这一刻取：此后任何文本变化都算未提交内容。
            let baseline = kind.field_values(self, cx);
            let body_app = app.clone();
            let body = cx.new(move |cx| {
                let observe = cx.observe(&body_app, move |_, _, cx| cx.notify());
                ModalBody {
                    kind,
                    app: body_app,
                    pending_focus: true,
                    _observe: observe,
                }
            });
            self.open_modals.layers.push(ModalLayer {
                kind,
                body,
                baseline,
            });
        }
    }

    /// 压入一层弹窗：外壳只负责容器属性，内容交给弹窗体。
    fn push_dialog(&mut self, kind: ModalKind, window: &mut Window, cx: &mut Context<Self>) {
        let Some(body) = self
            .open_modals
            .layers
            .iter()
            .find(|layer| layer.kind == kind)
            .map(|layer| layer.body.clone())
        else {
            log::error!("弹窗对账失序：种类 {:?} 缺少弹窗体实体，跳过这一层", kind);
            return;
        };
        let app = cx.entity().clone();
        let width = kind.width(window);
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let cancel_app = app.clone();
            let commit_app = app.clone();

            dialog
                .w(width)
                .max_w(width)
                // 右上角关闭件保留：用户要一个看得见的出口。遮罩点击关闭与
                // 入场动画按拍板关掉——对本应用的高危确认框，点外面就关是误操作源。
                .close_button(true)
                // 容器遮罩按 `window_paddings` 内缩定位：客户端装饰下实测内缩
                // 20px，会把压暗层上沿留在自绘顶栏中部。压暗改由宿主画满窗面，
                // 见 `AppShell::dialog_dim`。
                .overlay(false)
                .overlay_closable(false)
                .entrance(DialogEntrance::None)
                .child(body.clone())
                .on_cancel(move |_, window, cx| {
                    let closed = cancel_modal(&cancel_app, kind, window, cx);
                    if closed {
                        close_layer(&cancel_app, kind, cx);
                    }
                    closed
                })
                .on_ok(move |_, window, cx| {
                    let closed = commit_modal(&commit_app, kind, window, cx);
                    if closed {
                        close_layer(&commit_app, kind, cx);
                    }
                    closed
                })
        });
    }
}

impl AppShell {
    /// 弹窗栈的压暗层：只负责视觉压暗与命中阻断，不承载弹窗本体。
    ///
    /// 为什么要在宿主这一侧画：弹窗层由组件库在窗口根上渲染，其遮罩按窗口
    /// 内缩定位，客户端装饰下实测四周各留 20px；而应用顶栏从窗面 y=0 起画，
    /// 于是压暗层上沿正好卡在顶栏中间。宿主元素铺满整个窗面，层序在编辑面板
    /// 之上、弹窗层之下，与迁移前的自绘遮罩一致。
    pub(crate) fn dialog_dim(&self, p: &Palette) -> Option<AnyElement> {
        if self.open_modals.layers.is_empty() {
            return None;
        }
        Some(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .bg(p.overlay)
                // 与迁移前一致：阻断下层命中测试，否则点弹窗会同时触发终端与
                // 文件列表的鼠标处理并抢走焦点。点遮罩本身不做关闭动作（已拍板）。
                .occlude()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                .into_any_element(),
        )
    }
}

impl AppShell {
    /// 关闭一个 SFTP 弹窗：清状态并把焦点还给活动面板。
    ///
    /// 七个 SFTP 弹窗的取消动作只差"清哪个字段"，用闭包参数而不是七个
    /// 同形方法，避免复制七遍同样的焦点与重绘逻辑。
    pub(crate) fn close_sftp_modal<F>(
        &mut self,
        clear: F,
        window: &mut Window,
        cx: &mut Context<AppShell>,
    ) -> bool
    where
        F: FnOnce(&mut crate::sftp::SftpModals),
    {
        clear(&mut self.sftp_modals);
        self.focus_active_sftp_panel(window, cx);
        cx.notify();
        true
    }
}

/// 弹窗体内的事件回调适配器。
///
/// 为什么需要：弹窗体渲染时手上只有 `Entity<AppShell>` 与 `&mut App`，
/// 拿不到 `Context<AppShell>`，原先的 `cx.listener` 写法无处可用。事件回调
/// 发生在渲染之外，用实体 `update` 取回同样的借用对，即可原样复用各领域
/// 已有的状态变更方法，不必为弹窗另写一套逻辑。
pub(crate) fn on_event<A, F>(
    app: &Entity<AppShell>,
    handler: F,
) -> impl Fn(&A, &mut Window, &mut App) + 'static
where
    A: 'static,
    F: Fn(&mut AppShell, &A, &mut Window, &mut Context<AppShell>) + 'static,
{
    let app = app.clone();
    move |event, window, cx| {
        app.update(cx, |shell, cx| handler(shell, event, window, cx));
    }
}

/// 多行输入框的提交键位绑定：容器只绑了裸 `enter`。
///
/// 绑在 `"Dialog"` 上下文里，只在弹窗打开时生效，不会抢走终端与编辑器
/// 面板的 Ctrl+字母控制码；与 `keymap::bindings()` 一起注册。
pub(crate) const DIALOG_CONTEXT: &str = "Dialog";

pub(crate) fn dialog_bindings() -> Vec<KeyBinding> {
    vec![KeyBinding::new(
        "ctrl-enter",
        Confirm { secondary: true },
        Some(DIALOG_CONTEXT),
    )]
}
