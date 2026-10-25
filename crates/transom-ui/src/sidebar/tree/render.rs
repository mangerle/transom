//! 会话树行的渲染：文件夹行、会话行、拖拽预览与排序落点。
//!
//! 行模型、落点语义与拖放处理留在父模块 `tree`，本文件只负责把状态拼成 GPUI 元素，
//! 拆开的直接原因是合在一起后生产代码行数超过单文件阈值。

use super::{
    BAND_HEIGHT, FolderRow, ROW_CHEVRON_SIZE, ROW_HEIGHT, ROW_ICON_SIZE, SessionRow, SidebarDrag,
    SidebarTarget, TINT_FOLDER_IDLE, TINT_FOLDER_INTO, TINT_IDLE, TINT_SELECTED, indent_padding,
};
use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use transom_core::{Connection, DropSide, HostAction, SessionId};

/// 行内动作按钮的命中边长（px）。
const ROW_ACTION_SIZE: Pixels = px(20.0);
/// 行内动作按钮的图标边长（px），与折叠箭头同级，避免抢过会话协议图标。
const ROW_ACTION_ICON_SIZE: Pixels = px(14.0);

/// 行内图标按钮：常驻但弱色，悬停时才加深并垫一层底色。
///
/// 设计原理（Why）：
/// - 实现初衷：入口要挂在每一行上才谈得上"快捷"，做成悬停才浮现等于把入口藏起来，
///   用户反而不知道该点哪里，所以图标必须始终可见。
/// - 核心优势：用 `muted_foreground` 弱色与 14px 小尺寸，让它明显轻于行首的
///   18px 协议图标，几百行的列表不会被图标压成噪声；hover 态再补足可读性。
///   悬浮样式与 `id` 落在同一个元素上，靠按钮自己的 hover 表达状态，不去读父行
///   （父行的 hover 只会改底色，拿不到布尔值）。
/// - 常态色由调用方给出：活动会话是实心强调底，弱色图标在上面会糊成一片，
///   与行首协议图标共用同一套 `is_active` 判断才不各自漂移。
/// - 代价与局限：图标占据固定布局宽度（不是绝对定位叠加），行名可用宽度比没有
///   按钮时少 20px；不这么做的话悬停时行名会跟着抖动，比少 20px 更难接受。
fn row_action_button(
    element_id: String,
    icon: IconName,
    tooltip: String,
    idle_color: Hsla,
    p: &Palette,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(ElementId::Name(element_id.into()))
        .flex_none()
        .size(ROW_ACTION_SIZE)
        .flex()
        .items_center()
        .justify_center()
        .rounded(p.radius.md)
        .cursor_pointer()
        .text_color(idle_color)
        .hover(|style| style.bg(p.foreground.opacity(0.10)))
        .active(|style| style.bg(p.border))
        .tooltip(move |_window, cx| Tooltip::new(tooltip.clone()).build(_window, cx))
        .on_click(on_click)
        .child(Icon::new(icon).with_size(ROW_ACTION_ICON_SIZE))
        .into_any_element()
}

/// 拖动时跟随鼠标的轻量预览卡片。
pub(crate) struct SidebarDragPreview {
    label: String,
    icon: IconName,
}

impl SidebarDragPreview {
    fn new(label: String, icon: IconName) -> Self {
        Self { label, icon }
    }
}

impl Render for SidebarDragPreview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::from(cx);
        h_flex()
            .px_3()
            .py_1p5()
            .gap_2()
            .items_center()
            .rounded(p.radius.md)
            .bg(p.popover)
            .border_1()
            .border_color(p.border)
            .shadow_md()
            .text_sm()
            .text_color(p.popover_foreground)
            .child(Icon::new(self.icon).xsmall().text_color(p.muted_foreground))
            .child(self.label.clone())
    }
}

impl AppShell {
    /// 覆盖整行的排序落点：把行按份数等分，用户不需要瞄准行边缘。
    ///
    /// 会话行二分：上半插到该行之前、下半之后。文件夹行三分并留出中间一块，
    /// 中间块不注册任何监听，因此不产生命中框，拖放会继续落到行本身，
    /// 即“放入该文件夹”。等分用 `flex_1` 而非百分比宽度，gpui 没有宽度百分比。
    fn reorder_overlay(
        app: &Entity<AppShell>,
        anchor: SidebarDrag,
        keep_middle: bool,
        p: &Palette,
    ) -> AnyElement {
        let overlay = div()
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            .child(Self::reorder_half(app, anchor, DropSide::Before, true, p));
        let overlay = if keep_middle {
            overlay.child(div().flex_1())
        } else {
            overlay
        };
        overlay
            .child(Self::reorder_half(app, anchor, DropSide::After, false, p))
            .into_any_element()
    }

    /// 半区落点本身：`top` 决定插入线画在上沿还是下沿。
    ///
    /// 插入线用 accent_foreground：accent 已经是行底色，同色描边看不出。
    fn reorder_half(
        app: &Entity<AppShell>,
        anchor: SidebarDrag,
        side: DropSide,
        top: bool,
        p: &Palette,
    ) -> AnyElement {
        let target = SidebarTarget::Relative { anchor, side };
        // `drag_over` 的闭包要求 `'static`，颜色先取成局部值再捕获。
        let line_color = p.sidebar_accent_foreground;
        let zone = div().flex_1().border_color(line_color);
        let zone = if top {
            zone.drag_over::<SidebarDrag>(move |style, _, _, _| {
                style.border_t_2().border_color(line_color)
            })
        } else {
            zone.drag_over::<SidebarDrag>(move |style, _, _, _| {
                style.border_b_2().border_color(line_color)
            })
        };
        let app = app.clone();
        zone.on_drop(move |drag: &SidebarDrag, window, cx| {
            let drag = *drag;
            app.update(cx, |this, cx| {
                this.handle_sidebar_drop(drag, target, cx);
                window.refresh();
            });
        })
        .into_any_element()
    }

    pub(crate) fn render_folder_row(
        &mut self,
        row: &FolderRow,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let folder_id = row.id;
        let app_entity = cx.entity().clone();
        let label = row.name.clone();
        let chevron = if row.collapsed {
            IconName::ChevronRight
        } else {
            IconName::ChevronDown
        };

        let element = div()
            .id(ElementId::Name(format!("folder-{folder_id}").into()))
            .w_full()
            .h(ROW_HEIGHT)
            .pl(indent_padding(row.depth))
            .pr(px(4.0))
            .flex()
            .items_center()
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.toggle_folder_collapsed(folder_id, cx);
            }))
            .on_drag(SidebarDrag::Folder(folder_id), move |_, _, _, cx| {
                cx.new(|_| SidebarDragPreview::new(label.clone(), IconName::Folder))
            })
            .on_drop({
                let app = app_entity.clone();
                move |drag: &SidebarDrag, window, cx| {
                    let drag = *drag;
                    app.update(cx, |this, cx| {
                        this.handle_sidebar_drop(drag, SidebarTarget::Folder(folder_id), cx);
                        window.refresh();
                    });
                }
            })
            .context_menu({
                let app_entity = app_entity.clone();
                move |menu, _window, _cx| {
                    let app = app_entity.clone();
                    menu.item(
                        PopupMenuItem::new(rust_i18n::t!("menu.new_session_here").to_string())
                            .icon(IconName::Plus)
                            .on_click({
                                let app = app.clone();
                                move |_, window, cx| {
                                    app.update(cx, |this, cx| {
                                        this.open_new_session_in_folder(folder_id, window, cx);
                                    });
                                }
                            }),
                    )
                    .item(
                        PopupMenuItem::new(rust_i18n::t!("menu.new_folder").to_string())
                            .icon(IconName::FolderPlus)
                            .on_click({
                                let app = app.clone();
                                move |_, window, cx| {
                                    app.update(cx, |this, cx| {
                                        this.open_new_folder(Some(folder_id), window, cx);
                                    });
                                }
                            }),
                    )
                    .item(
                        PopupMenuItem::new(rust_i18n::t!("menu.rename").to_string())
                            .icon(IconName::FolderPen)
                            .on_click({
                                let app = app.clone();
                                move |_, window, cx| {
                                    app.update(cx, |this, cx| {
                                        this.open_rename_folder(folder_id, window, cx);
                                    });
                                }
                            }),
                    )
                    .item(PopupMenuItem::separator())
                    .item(
                        PopupMenuItem::new(rust_i18n::t!("menu.delete_folder").to_string())
                            .icon(IconName::Trash)
                            .on_click({
                                let app = app.clone();
                                move |_, _, cx| {
                                    app.update(cx, |this, cx| {
                                        this.delete_folder(folder_id, cx);
                                    });
                                }
                            }),
                    )
                }
            });
        // `drag_over` 闭包要求 `'static`，先取成局部颜色再捕获。
        let into_bg = p.sidebar_accent.opacity(TINT_FOLDER_INTO);
        // 悬浮样式必须与 `.id()` 落在同一个元素上：GPUI 只给带 id 的元素分配
        // global_id 与悬浮状态，缺少 id 时鼠标移入不会触发重绘，高亮只能等
        // 别的事件顺带重绘才补上，看上去就是"悬停不跟手"。
        let mut band = h_flex()
            .id(ElementId::Name(format!("folder-band-{folder_id}").into()))
            .relative()
            .w_full()
            .h(BAND_HEIGHT)
            .px_2()
            .gap_1()
            .items_center()
            .rounded(p.radius.md)
            .bg(p.sidebar_accent.opacity(TINT_FOLDER_IDLE))
            .border_b_1()
            .border_color(p.sidebar_border.opacity(0.55))
            .text_color(p.sidebar_foreground)
            .hover(|style| style.bg(p.sidebar_accent))
            .drag_over::<SidebarDrag>(move |style, _, _, _| style.bg(into_bg))
            .child(Icon::new(chevron).with_size(ROW_CHEVRON_SIZE))
            .child(
                Icon::new(if row.collapsed {
                    IconName::Folder
                } else {
                    IconName::FolderOpen
                })
                .with_size(ROW_ICON_SIZE),
            )
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .truncate()
                    .child(row.name.clone()),
            )
            .child(div().text_xs().child(row.session_count.to_string()));

        if row.folder_count > 0 {
            band = band.child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground.opacity(0.7))
                    .child(format!("({})", row.folder_count)),
            );
        }
        // 表头这一个按钮拨测整组（含子文件夹）：分组巡检是本功能的主用法。
        band = band.child(row_action_button(
            format!("folder-probe-{folder_id}"),
            IconName::Radar,
            rust_i18n::t!("probe.folder_tooltip").to_string(),
            p.muted_foreground,
            p,
            cx.listener(move |this, _: &ClickEvent, _window, cx| {
                // 点按钮不该顺带折叠/展开这个分组。
                cx.stop_propagation();
                this.probe_folder(folder_id, cx);
            }),
        ));
        let anchor = SidebarDrag::Folder(folder_id);
        // 文件夹行保留中段作为“放入该分组”，上下各三分之一用于同级换序。
        band = band.child(Self::reorder_overlay(&app_entity, anchor, true, p));
        element.child(band).into_any_element()
    }

    pub(crate) fn render_session_row(
        &mut self,
        row: &SessionRow,
        active_session: Option<SessionId>,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let session = &row.session;
        let session_id = session.id;
        let folder_id = session.folder_id;
        let app_entity = cx.entity().clone();
        let (icon_name, detail_text) = match &session.connection {
            Connection::Ssh(ssh) => (IconName::Server, format!("{}@{}", ssh.username, ssh.host)),
            Connection::Telnet(telnet) => (
                IconName::Network,
                format!("Telnet: {}:{}", telnet.host, telnet.port),
            ),
            Connection::Local(local) => {
                let shell = local.shell.as_deref().unwrap_or("sh");
                (IconName::Terminal, format!("Local: {shell}"))
            }
            Connection::Serial(serial) => (
                IconName::Cable,
                format!("Serial: {} @ {}", serial.port, serial.baud_rate),
            ),
            _ => (IconName::Server, String::new()),
        };
        let is_active = active_session == Some(session_id);
        let selection_mode = self.selection_mode;
        let is_selected = self.selected_sessions.contains(&session_id);
        let is_marked = self.tree_selection == Some(session_id);
        let drag_label = session.name.clone();
        let drag_icon = icon_name;
        let element = div()
            .id(ElementId::Name(format!("session-{session_id}").into()))
            .w_full()
            .h(ROW_HEIGHT)
            .pl(indent_padding(row.depth))
            .pr(px(4.0))
            .flex()
            .items_center()
            .cursor_pointer()
            // 单击只负责选中这一行，双击才建立连接：会话行是列表里最容易被顺手点到的
            // 元素，把连接挂在单击上会让"看一眼"和"连上去"没法区分。
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                if this.selection_mode {
                    let selected = this.selected_sessions.contains(&session_id);
                    this.toggle_session_selection(session_id, !selected, cx);
                } else if event.click_count() >= 2 {
                    this.connect_session(session_id, window, cx);
                } else {
                    this.tree_selection = Some(session_id);
                    cx.notify();
                }
            }))
            .on_drag(SidebarDrag::Session(session_id), move |_, _, _, cx| {
                cx.new(|_| SidebarDragPreview::new(drag_label.clone(), drag_icon))
            })
            .on_drop({
                let app = app_entity.clone();
                move |drag: &SidebarDrag, window, cx| {
                    let drag = *drag;
                    // 落在会话行中部：放进该会话所在层级末尾（根层级会话则回到根）。
                    let target = match folder_id {
                        Some(folder_id) => SidebarTarget::Folder(folder_id),
                        None => SidebarTarget::Root,
                    };
                    app.update(cx, |this, cx| {
                        this.handle_sidebar_drop(drag, target, cx);
                        window.refresh();
                    });
                }
            })
            .context_menu({
                let app_entity = app_entity.clone();
                move |menu, _window, _cx| {
                    let app = app_entity.clone();
                    let mut menu = menu.item(
                        PopupMenuItem::new(rust_i18n::t!("menu.connect").to_string())
                            .icon(IconName::Terminal)
                            .on_click({
                                let app = app.clone();
                                move |_, window, cx| {
                                    app.update(cx, |this, cx| {
                                        this.connect_session(session_id, window, cx);
                                    });
                                }
                            }),
                    );
                    if folder_id.is_some() {
                        menu = menu.item(
                            PopupMenuItem::new(rust_i18n::t!("menu.move_to_root").to_string())
                                .icon(IconName::FolderTree)
                                .on_click({
                                    let app = app.clone();
                                    move |_, _, cx| {
                                        app.update(cx, |this, cx| {
                                            this.handle_sidebar_drop(
                                                SidebarDrag::Session(session_id),
                                                SidebarTarget::Root,
                                                cx,
                                            );
                                        });
                                    }
                                }),
                        );
                    }
                    menu.item(
                        PopupMenuItem::new(rust_i18n::t!("menu.duplicate").to_string())
                            .icon(IconName::Plus)
                            .on_click({
                                let app = app.clone();
                                move |_, _, cx| {
                                    app.update(cx, |this, cx| {
                                        this.duplicate_session(session_id, cx);
                                    });
                                }
                            }),
                    )
                    .item(
                        PopupMenuItem::new(rust_i18n::t!("menu.edit").to_string())
                            .icon(IconName::Settings)
                            .on_click({
                                let app = app.clone();
                                move |_, window, cx| {
                                    app.update(cx, |this, cx| {
                                        this.open_edit_session(session_id, window, cx);
                                    });
                                }
                            }),
                    )
                    .item(PopupMenuItem::separator())
                    .item(
                        PopupMenuItem::new(rust_i18n::t!("menu.delete").to_string())
                            .icon(IconName::Trash)
                            .on_click({
                                let app = app.clone();
                                move |_, _, cx| {
                                    app.update(cx, |this, cx| {
                                        this.delete_session_by_id(session_id, cx);
                                    });
                                }
                            }),
                    )
                }
            });
        // 高亮与悬停只作用于内层内容条，缩进区保持透明，层级靠左偏移表达。
        // 单击选中的行与多选勾选的行共用"选中"这一档深浅，避免再引入第五种底色。
        let is_selected_row = is_marked || (selection_mode && is_selected);
        let tint = if is_active {
            1.0
        } else if is_selected_row {
            TINT_SELECTED
        } else {
            TINT_IDLE
        };
        // 悬浮样式必须与 `.id()` 落在同一个元素上：GPUI 只给带 id 的元素分配
        // global_id 与悬浮状态，缺少 id 时鼠标移入不会触发重绘，高亮只能等
        // 别的事件顺带重绘才补上，看上去就是"悬停不跟手"。
        let mut inner = h_flex()
            .id(ElementId::Name(format!("session-band-{session_id}").into()))
            .relative()
            .w_full()
            .h(BAND_HEIGHT)
            .px_2()
            .gap_2()
            .items_center()
            .rounded(p.radius.md)
            .bg(p.sidebar_accent.opacity(tint))
            .text_color(if is_active {
                p.sidebar_accent_foreground
            } else {
                p.sidebar_foreground
            });
        if is_active {
            // 活动会话与悬浮态同为实心强调色，靠左侧竖条区分"当前正在使用的会话"。
            // 竖条绝对定位、不占布局宽度，切换活动会话时行内图标不会发生位移。
            inner = inner.child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .bottom_0()
                    .w(px(2.0))
                    .rounded_full()
                    .bg(p.list_active_border),
            );
        } else {
            // 悬浮铺满实心强调色：低透明度叠加在深色侧边栏上几乎看不出差别，
            // 换成实色才有 SFTP 文件行那种"整条弹出"的清晰反馈。
            inner = inner.hover(|style| style.bg(p.sidebar_accent));
        }
        if selection_mode {
            inner = inner.child(
                Checkbox::new(ElementId::Name(
                    format!("session-select-{session_id}").into(),
                ))
                .small()
                .checked(is_selected)
                .on_click(cx.listener(move |this, &checked, _, cx| {
                    cx.stop_propagation();
                    this.toggle_session_selection(session_id, checked, cx);
                })),
            );
        }
        let inner = inner
            .child(
                Icon::new(icon_name)
                    .with_size(ROW_ICON_SIZE)
                    .text_color(if is_active {
                        p.sidebar_accent_foreground
                    } else {
                        p.muted_foreground
                    }),
            )
            .child(
                v_flex()
                    .flex_1()
                    .overflow_hidden()
                    .child(
                        div()
                            .text_sm()
                            .line_height(px(17.0))
                            .font_weight(FontWeight::MEDIUM)
                            .truncate()
                            .child(session.name.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .line_height(px(14.0))
                            .truncate()
                            .text_color(if is_active {
                                p.sidebar_accent_foreground
                            } else {
                                p.muted_foreground
                            })
                            .child(detail_text),
                    ),
            );
        // 行尾状态点只在有在途请求或结果时存在，到期由 probe 模块自行清理。
        let probe_badge = self
            .probe_badges
            .get(&session_id)
            .map(|badge| AppShell::render_probe_badge(session_id, badge, p));
        // 本地 Shell 与串口没有可拨测的网络地址，按钮也不画：点了只会弹一句
        // "没有可拨测主机"，不如根本不给出这个动作。
        let probe_button = HostAction::probe_for(session).map(|_| {
            row_action_button(
                format!("session-probe-{session_id}"),
                IconName::Radar,
                rust_i18n::t!("probe.session_tooltip").to_string(),
                if is_active {
                    p.sidebar_accent_foreground
                } else {
                    p.muted_foreground
                },
                p,
                cx.listener(move |this, _: &ClickEvent, _window, cx| {
                    // 拨测这一台不该同时选中这一行，更不该触发双击连接。
                    cx.stop_propagation();
                    this.probe_session(session_id, cx);
                }),
            )
        });
        let anchor = SidebarDrag::Session(session_id);
        // 会话行没有子级，整行二分为“插到之前/之后”，不存在落空的中间区。
        let inner = inner
            .children(probe_badge)
            .children(probe_button)
            .child(Self::reorder_overlay(&app_entity, anchor, false, p));
        element.child(inner).into_any_element()
    }
}
