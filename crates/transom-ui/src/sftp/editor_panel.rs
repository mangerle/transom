//! SFTP 内置编辑器的面板状态、窗体动作与几何/语言换算。
//!
//! 生命周期（拉取、保存、回传结果）在 `editor_ops.rs`，渲染在 `editor_view.rs`。
//!
//! 设计原理（Why）：
//! - 容器是可拖拽、可折叠、可最大化的非阻塞面板，而不是模态弹窗。编辑远程配置的
//!   真实动作是"一边看目录或日志一边改这几行"，模态遮罩会挡掉窗格、终端与文件列表，
//!   固定尺寸的弹窗也只能在几百像素里滚动长文件。面板自绘绝对定位，只对自身 `occlude`，
//!   面板之外的一切照常可点可聚焦。
//! - 折叠（只留标题栏）对应"最小化"：面板要常驻但不能一直占着画面，关闭又会丢掉
//!   正在编辑的内容，折叠是这两者之间唯一合理的中间态。
//! - 高亮 token 直接取文件扩展名交给 syntect 解析（`find_syntax_by_token` 先按扩展名
//!   再按语法名匹配），只在扩展名确实无法命中的情况下重定向，其余一律原样透传，
//!   避免维护一张会随语法集版本过期的语言别名表。

use crate::app::AppShell;
use gpui_kit::component::input::EditorState;
use gpui_kit::*;
use transom_core::{TabId, TransferId};

/// 面板默认尺寸：足够容纳一个两百行的配置文件，又不至于盖住整个工作区。
pub(crate) const PANEL_WIDTH: f32 = 880.0;
pub(crate) const PANEL_HEIGHT: f32 = 540.0;
/// 折叠为仅标题栏后的高度。
const PANEL_COLLAPSED_HEIGHT: f32 = 40.0;
/// 拖拽与落位时允许贴近视口边缘的最小间距。
const PANEL_EDGE_MARGIN: f32 = 8.0;
/// 拖拽时始终留在视口内的最小像素数，保证标题栏与控制按钮够得着。
pub(crate) const PANEL_KEEP_VISIBLE: f32 = 160.0;
/// 连续打开多个文件时的级联偏移，避免完全重叠导致只剩最上面一个可见。
const PANEL_CASCADE_STEP: f32 = 26.0;

/// 面板与远端之间的读写阶段。用枚举而非布尔位表达，杜绝"正在上传且已失败"这类非法组合。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EditorPhase {
    /// 正在读取远端内容，编辑器还是空的，主体区显示加载占位。
    Reading,
    /// 没有进行中的读写。
    Idle,
    /// 正在回传，此时禁止重复保存。
    Writing,
    /// 远端已确认写入成功，缓冲区成为新的脏基准。
    Written,
    /// 读取或回传失败；错误就地显示，内容保留可原地重试。
    Failed,
}

/// 一个已打开的远端文本文件对应的悬浮面板状态。
pub(crate) struct EditorPanelState {
    /// 面板所属标签页；关闭标签时用于连带回收面板。
    pub(crate) tab_id: TabId,
    pub(crate) remote_path: String,
    pub(crate) file_name: String,
    /// 传给编辑器的高亮 token（扩展名或语法名），未识别时为 `text`。
    pub(crate) language: String,
    pub(crate) editor: Entity<EditorState>,
    /// 最近一次与远端一致的内容，用于脏检查；回传成功后整体替换。
    pub(crate) baseline: SharedString,
    /// 由编辑器 `Change` 事件置位，避免每帧比较整份文本。
    pub(crate) dirty: bool,
    /// 是否已成功装载过远端内容。
    ///
    /// 读取失败时缓冲区是空的：若允许在空缓冲区上编辑并回传，会把远端文件覆盖成
    /// 用户随手打的几个字。因此它是"能否写回"的硬前提，与 `dirty` 分开表达——
    /// 后者只描述"内容与基准不同"，前者描述"基准是否来自远端"。
    pub(crate) loaded: bool,
    /// 缓存的光标行列（1 基），随编辑器通知更新。
    ///
    /// 状态栏不能直接在渲染时读 `cursor_position()`：光标移动只重绘编辑器实体，
    /// 不会重建 `AppShell`，读到的永远是上一次重建时的旧值。改为监听编辑器通知、
    /// 只在行列真的变了时刷新这份缓存并重绘，既拿到了实时值，也避免了
    /// 「面板重绘 → 编辑器通知 → 面板重绘」的自激循环。
    pub(crate) caret: (u32, u32),
    pub(crate) phase: EditorPhase,
    /// 最近一次回传成功后远端确认的字节数；再次改动即清空。
    ///
    /// 状态栏把"已回传（1.2 KB）"直接写出来，是给用户看的一次写入凭证：
    /// 数字来自写完后对远端文件的 stat，与提交字节数比对过才显示。
    pub(crate) written_size: Option<u64>,
    /// 在途上传任务 ID；传输完成/失败事件按它回填结果。
    pub(crate) pending_op: Option<TransferId>,
    pub(crate) error: Option<String>,
    /// 关闭时存在未保存修改，等待用户在面板内二次确认。
    pub(crate) confirm_discard: bool,
    /// 折叠为仅标题栏：保留面板但让出画面，对应"最小化"的诉求。
    pub(crate) collapsed: bool,
    pub(crate) maximized: bool,
    pub(crate) left: f32,
    pub(crate) top: f32,
    pub(crate) width: f32,
    pub(crate) height: f32,
    /// 拖拽锚点：按下点相对面板左上角的偏移，`None` 表示当前没在拖。
    pub(crate) drag_grab: Option<(f32, f32)>,
    /// 订阅句柄必须持有，析构即退订，`Change` 事件将无法置脏。
    pub(crate) _change_sub: Subscription,
    /// 编辑器通知观察句柄：光标移动只 notify 不 emit 事件，靠它刷新行列。
    pub(crate) _caret_sub: Subscription,
}

impl EditorPanelState {
    /// 当前实际占用的垂直高度：折叠态只占标题栏。
    pub(crate) fn current_height(&self) -> f32 {
        if self.collapsed {
            PANEL_COLLAPSED_HEIGHT
        } else {
            self.height
        }
    }

    /// 面板身份：同一标签内同一路径视为同一个编辑会话。
    pub(crate) fn same_file(&self, tab_id: TabId, remote_path: &str) -> bool {
        self.tab_id == tab_id && self.remote_path == remote_path
    }
}

impl AppShell {
    pub(crate) fn toggle_editor_panel_collapsed(
        &mut self,
        editor: &Entity<EditorState>,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = self.editor_panel_mut(editor) else {
            return;
        };
        item.collapsed = !item.collapsed;
        // 折叠与最大化互斥：折叠时先退出全屏，否则只有标题栏的一条横幅铺满整屏。
        if item.collapsed {
            item.maximized = false;
        }
        cx.notify();
    }

    pub(crate) fn toggle_editor_panel_maximized(
        &mut self,
        editor: &Entity<EditorState>,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = self.editor_panel_mut(editor) else {
            return;
        };
        item.maximized = !item.maximized;
        item.collapsed = false;
        cx.notify();
    }

    /// 把面板挪到绘制序列末尾，即抬到所有悬浮面板之上。
    pub(crate) fn raise_editor_panel(
        &mut self,
        editor: &Entity<EditorState>,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self
            .editor_panels
            .iter()
            .position(|item| item.editor == *editor)
        else {
            return;
        };
        if index + 1 == self.editor_panels.len() {
            return;
        }
        let item = self.editor_panels.remove(index);
        self.editor_panels.push(item);
        cx.notify();
    }

    /// 标题栏按下：记录抓取偏移并置顶，真正的位移由视口级 mouse_move 驱动。
    pub(crate) fn begin_editor_panel_drag(
        &mut self,
        editor: &Entity<EditorState>,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.raise_editor_panel(editor, cx);
        let Some(item) = self.editor_panel_mut(editor) else {
            return;
        };
        if item.maximized {
            return;
        }
        let (x, y): (f32, f32) = (position.x.into(), position.y.into());
        item.drag_grab = Some((x - item.left, y - item.top));
    }

    /// 视口级拖拽推进：只有确实有面板在被拖动时才改写状态并重绘。
    pub(crate) fn drag_editor_panels(
        &mut self,
        position: Point<Pixels>,
        viewport: Size<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let (x, y): (f32, f32) = (position.x.into(), position.y.into());
        let (viewport_w, viewport_h): (f32, f32) = (viewport.width.into(), viewport.height.into());
        let mut dragging = false;
        for item in self.editor_panels.iter_mut() {
            let Some((grab_x, grab_y)) = item.drag_grab else {
                continue;
            };
            dragging = true;
            item.left = clamp_axis(x - grab_x, item.width, viewport_w, PANEL_KEEP_VISIBLE);
            item.top = clamp_axis(
                y - grab_y,
                item.current_height(),
                viewport_h,
                PANEL_KEEP_VISIBLE,
            );
        }
        if dragging {
            cx.notify();
        }
    }

    pub(crate) fn end_editor_panel_drag(&mut self, cx: &mut Context<Self>) {
        if !self
            .editor_panels
            .iter()
            .any(|item| item.drag_grab.is_some())
        {
            return;
        }
        for item in self.editor_panels.iter_mut() {
            item.drag_grab = None;
        }
        cx.notify();
    }

    /// 关闭请求：脏内容先在面板内确认，干净内容直接回收。
    pub(crate) fn request_close_editor_panel(
        &mut self,
        editor: &Entity<EditorState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dirty = self
            .editor_panels
            .iter()
            .find(|item| item.editor == *editor)
            .is_some_and(|item| item.dirty);
        if dirty {
            if let Some(item) = self.editor_panel_mut(editor) {
                item.confirm_discard = true;
            }
            cx.notify();
            return;
        }
        self.drop_editor_panel(editor, window, cx);
    }

    /// 确认丢弃：放弃未保存改动并关闭面板。
    pub(crate) fn discard_editor_panel(
        &mut self,
        editor: &Entity<EditorState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(item) = self.editor_panel_mut(editor) {
            item.confirm_discard = false;
            item.dirty = false;
        }
        self.drop_editor_panel(editor, window, cx);
    }

    /// 留在面板：撤销二次确认并把焦点交还编辑器。
    pub(crate) fn keep_editor_panel(
        &mut self,
        editor: &Entity<EditorState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(item) = self.editor_panel_mut(editor) {
            item.confirm_discard = false;
        }
        editor.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }
}

/// 读取光标行列并换算成 1 基展示值。
///
/// 行列换算集中在这一处，状态栏与观察回调才不会各写一遍而算出两套编号。
pub(crate) fn caret_of(editor: &Entity<EditorState>, cx: &App) -> (u32, u32) {
    let position = editor.read(cx).cursor_position();
    (
        position.line.saturating_add(1),
        position.character.saturating_add(1),
    )
}

/// 沿单轴夹取面板左上角。
///
/// 面板放得下时完全容纳在视口内；放不下时保证至少 `min_visible` 像素留在视口里，
/// 避免被拖到够不着的地方——标题栏回不来是浮动窗口最常见的误操作。
/// 视口比 `min_visible` 还小时贴边，用 `max` 兜住下界，杜绝反向区间。
pub(crate) fn clamp_axis(origin: f32, panel_len: f32, viewport_len: f32, min_visible: f32) -> f32 {
    let upper = if panel_len <= viewport_len {
        viewport_len - panel_len
    } else {
        viewport_len - min_visible
    }
    .max(PANEL_EDGE_MARGIN);
    origin.clamp(PANEL_EDGE_MARGIN, upper)
}

/// 级联落位：第 n 个面板相对前一个向右下偏移。
pub(crate) fn cascade_origin(index: usize, viewport: Size<Pixels>) -> (f32, f32) {
    let (viewport_w, viewport_h): (f32, f32) = (viewport.width.into(), viewport.height.into());
    let step = (index as f32) * PANEL_CASCADE_STEP;
    (
        clamp_axis(
            PANEL_EDGE_MARGIN + step,
            PANEL_WIDTH,
            viewport_w,
            PANEL_KEEP_VISIBLE,
        ),
        clamp_axis(
            PANEL_EDGE_MARGIN + step,
            PANEL_HEIGHT,
            viewport_h,
            PANEL_KEEP_VISIBLE,
        ),
    )
}

/// 推断语法高亮 token：优先扩展名，其次整文件名，都不认识时退回 `text`。
///
/// syntect 先按扩展名再按语法名匹配，`text` 在库内被显式判定为不高亮，
/// 因此未知类型自然降级为纯文本，不需要额外分支。
pub(crate) fn editor_language_token(remote_path: &str) -> String {
    let base = remote_path
        .rsplit('/')
        .next()
        .unwrap_or(remote_path)
        .to_lowercase();
    if base.is_empty() {
        return "text".to_string();
    }
    if let Some(dotfile) = base.strip_prefix('.') {
        // 无扩展名的点文件（.bashrc/.profile）按 shell 处理，扩展名表里没有这些条目。
        return match dotfile {
            "bashrc" | "bash_profile" | "bash_login" | "zshrc" | "zprofile" | "zshenv"
            | "profile" => "sh".to_string(),
            other => extension_token(other),
        };
    }
    match base.rsplit_once('.') {
        Some((_, ext)) if !ext.is_empty() => extension_token(ext),
        // 无扩展名：Makefile/Dockerfile 这类以语法名注册的文件可以直接按名字命中。
        _ => base,
    }
}

/// 扩展名到高亮 token 的唯一一层重定向，只处理扩展名本身无法命中的情况。
fn extension_token(ext: &str) -> String {
    match ext {
        // systemd 单元文件是 ini 结构，语法集里没有独立 grammar；.cnf 同理。
        "service" | "socket" | "timer" | "mount" | "automount" | "swap" | "target" | "path"
        | "unit" | "cnf" => "ini".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{PANEL_EDGE_MARGIN, clamp_axis, editor_language_token, extension_token};

    #[test]
    fn test_language_token_uses_extension() {
        assert_eq!(editor_language_token("/etc/nginx/nginx.conf"), "conf");
        assert_eq!(editor_language_token("/app/config.yaml"), "yaml");
        // 别名（yml→yaml、sh→bash 等）由组件库的 language_name 归一化，本层只透传，
        // 这样就不必复刻一张会随语法集版本过期的别名表。
        assert_eq!(editor_language_token("/app/config.yml"), "yml");
        assert_eq!(editor_language_token("/srv/app/.env"), "env");
    }

    #[test]
    fn test_language_token_redirects_unresolvable_extensions() {
        assert_eq!(
            editor_language_token("/etc/systemd/journald.service"),
            "ini"
        );
        assert_eq!(editor_language_token("/etc/my.cnf"), "ini");
        assert_eq!(extension_token("toml"), "toml");
    }

    #[test]
    fn test_language_token_handles_dotfile_and_bare_name() {
        assert_eq!(editor_language_token("/root/.bashrc"), "sh");
        assert_eq!(editor_language_token("/build/Makefile"), "makefile");
        assert_eq!(editor_language_token("/build/Dockerfile"), "dockerfile");
        assert_eq!(editor_language_token(""), "text");
    }

    #[test]
    fn test_clamp_axis_keeps_panel_reachable() {
        // 视口足够大：面板整体容纳，右上角极限是 1920-880。
        assert_eq!(clamp_axis(5000.0, 880.0, 1920.0, 160.0), 1040.0);
        assert_eq!(clamp_axis(-500.0, 880.0, 1920.0, 160.0), PANEL_EDGE_MARGIN);
        // 视口比面板窄：仍保留 160 像素可见，而不是把标题栏推出屏幕。
        assert_eq!(clamp_axis(5000.0, 880.0, 500.0, 160.0), 340.0);
        // 视口极小：贴边，不产生反向区间。
        assert_eq!(clamp_axis(50.0, 880.0, 100.0, 160.0), PANEL_EDGE_MARGIN);
    }
}
