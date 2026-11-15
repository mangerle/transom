//! 设计令牌、交互语义与渲染副作用守卫。

use crate::common::ui_source_files;

#[test]
fn ui_text_sizes_use_design_tokens() {
    let mut offenders = Vec::new();
    for (path, content) in ui_source_files() {
        let lines: Vec<&str> = content.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            let previous_is_exception = index > 0 && lines[index - 1].contains("设计令牌例外");
            if line.contains(".text_size(px(") && !previous_is_exception {
                offenders.push(format!("{path}:{}: {}", index + 1, line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "页面层字号必须使用 rem/token：\n{}",
        offenders.join("\n")
    );
}

/// 关键结构尺寸必须使用 rem/token，而不是固定像素。
#[test]
fn key_structural_sizes_use_rem_tokens() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let checks = [
        (
            "src/container/list/render.rs",
            &[".h(px(40.0))", ".h(px(40))"][..],
            "rems(2.5)",
        ),
        (
            "src/sidebar/tree/mod.rs",
            &[".h(px(38.0))"][..],
            "ROW_HEIGHT_REM",
        ),
        (
            "src/settings/modal.rs",
            &[".sidebar_width(px(220.0))"][..],
            "rem_px(13.75",
        ),
    ];
    for (relative, forbidden, required) in checks {
        let content = std::fs::read_to_string(root.join(relative)).expect("读取结构尺寸文件失败");
        for needle in forbidden {
            assert!(
                !content.contains(needle),
                "{relative} 仍使用固定结构尺寸：{needle}"
            );
        }
        assert!(
            content.contains(required),
            "{relative} 缺少 rem 结构尺寸：{required}"
        );
    }
}

/// 组件层必须走设计令牌：字号、间距、控件尺寸与弹窗宽度不能在这里重新写
/// `text_xs()` / `px(...)`，否则页面迁移到组件层后仍会随密度切换失调。
#[test]
fn widgets_use_design_tokens() {
    let mut offenders = Vec::new();
    for (path, content) in ui_source_files() {
        if !path.contains("/widgets/") {
            continue;
        }
        for (index, line) in content.lines().enumerate() {
            let magic_px = line
                .match_indices("px(")
                .any(|(index, _)| !line[index + 3..].starts_with("rems("));
            if line.contains("text_xs(")
                || line.contains("text_sm(")
                || magic_px
                || line.contains("gap_1(")
                || line.contains("gap_2(")
                || line.contains("gap_3(")
                || line.contains("gap_4(")
            {
                offenders.push(format!("{path}:{}: {}", index + 1, line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "widgets 组件必须使用 theme::tokens 中的字号/间距/控件令牌：\n{}",
        offenders.join("\n")
    );
}

/// 高频透明度必须使用应用级语义令牌。
///
/// 这些表达式曾散落在业务页面中，导致同一视觉层级由不同文件各自决定透明度。
/// 集中到 `Palette` 后，页面只选择 surface、stroke、tone、text 语义，主题切换
/// 才能整体调整最终观感。
#[test]
fn high_frequency_opacity_uses_semantic_tokens() {
    const RAW_EXPRESSIONS: &[&str] = &[
        "p.muted.opacity(0.04)",
        "p.muted.opacity(0.08)",
        "p.muted.opacity(0.12)",
        "p.muted.opacity(0.3)",
        "p.muted.opacity(0.30)",
        "p.muted.opacity(0.35)",
        "p.muted.opacity(0.4)",
        "p.muted.opacity(0.40)",
        "p.border.opacity(0.12)",
        "p.border.opacity(0.15)",
        "p.border.opacity(0.3)",
        "p.border.opacity(0.30)",
        "p.border.opacity(0.4)",
        "p.border.opacity(0.40)",
        "p.border.opacity(0.5)",
        "p.border.opacity(0.50)",
        "p.danger.opacity(0.12)",
        "p.warning.opacity(0.12)",
        "p.success.opacity(0.12)",
        "p.primary.opacity(0.12)",
        "p.muted_foreground.opacity(0.8)",
        "p.muted_foreground.opacity(0.80)",
        "p.muted_foreground.opacity(0.7)",
        "p.muted_foreground.opacity(0.70)",
        "p.muted_foreground.opacity(0.4)",
        "p.muted_foreground.opacity(0.40)",
        "p.foreground.opacity(0.08)",
    ];

    let mut offenders = Vec::new();
    for (path, content) in ui_source_files() {
        if (!path.contains("/crates/transom-ui/src/")
            && !path.contains("/crates/transom-ui-kit/src/"))
            || path.ends_with("/palette.rs")
        {
            continue;
        }
        for (index, line) in content.lines().enumerate() {
            for expression in RAW_EXPRESSIONS {
                if line.contains(expression) {
                    offenders.push(format!("{path}:{}: {}", index + 1, line.trim()));
                }
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "业务页面仍直接使用高频透明度，请改用 Palette 的 surface/stroke/tone/text 语义令牌：\n{}",
        offenders.join("\n")
    );
}

/// 终端专属按键必须由应用初始化一次性注册，不能在每个终端实体构造时追加。
#[test]
fn terminal_view_does_not_bind_global_keys() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../transom-terminal/src/view/mod.rs");
    let content = std::fs::read_to_string(&path).expect("读取终端视图失败");
    assert!(
        !content.contains("cx.bind_keys("),
        "TerminalView::new 不得追加全局 keymap；终端按键应在应用初始化阶段注册"
    );
}

/// UI 文案统一使用单字符省略号；技术转义示例除外。
#[test]
fn ui_copy_uses_single_ellipsis() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../locales/ui.yml");
    let content = std::fs::read_to_string(path).expect("读取界面文案失败");
    let offenders: Vec<String> = content
        .lines()
        .enumerate()
        .filter(|(_, line)| line.contains("...") && !line.contains("\\u{...}"))
        .map(|(index, line)| format!("{}: {}", index + 1, line.trim()))
        .collect();
    assert!(
        offenders.is_empty(),
        "界面文案应使用单字符省略号：\n{}",
        offenders.join("\n")
    );
}

/// 拖拽预览和加载占位必须复用主题与组件，而不是固定颜色或字符点。
#[test]
fn feedback_uses_theme_and_progress_components() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let drag = std::fs::read_to_string(root.join("src/sftp/side_remote/types.rs"))
        .expect("读取 SFTP 拖拽预览失败");
    assert!(
        drag.contains("Palette::from(cx)"),
        "拖拽预览必须读取主题调色板"
    );
    assert!(!drag.contains("hsla("), "拖拽预览不得硬编码颜色");
    let system_df = std::fs::read_to_string(root.join("src/container/system_df_dialog.rs"))
        .expect("读取磁盘用量弹窗失败");
    assert!(
        !system_df.contains("\"...\".to_string()"),
        "加载态不得使用字符点"
    );
}

/// 选择型控件和应用内命令必须使用正确的语义组件与变体。
#[test]
fn selection_and_command_semantics_are_explicit() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let auth = std::fs::read_to_string(root.join("src/session_form/ssh/mod.rs"))
        .expect("读取 SSH 表单失败");
    assert!(auth.contains("ButtonGroup::new(\"auth-methods\")"));
    let forward =
        std::fs::read_to_string(root.join("src/forward/view.rs")).expect("读取转发视图失败");
    assert!(forward.contains("ButtonGroup::new(\"forward-kind\")"));
    let snippets =
        std::fs::read_to_string(root.join("src/snippets/card.rs")).expect("读取片段卡片失败");
    assert!(
        !snippets.contains(".link()"),
        "应用内新增命令不得使用 Link 语义"
    );
    let container_list = std::fs::read_to_string(root.join("src/container/list/render.rs"))
        .expect("读取容器列表失败");
    assert!(
        !container_list.contains(".left_0()"),
        "选中行不得使用左侧强调条"
    );
}

/// Toast 定时器必须纳入任务容器，不得脱缰 detach。
#[test]
fn toast_tasks_are_retained() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/app/mod.rs");
    let content = std::fs::read_to_string(path).expect("读取应用外壳失败");
    let start = content.find("fn push_toast(").expect("未找到 Toast 定时器");
    let end = content[start..]
        .find(
            "
    }
",
        )
        .map_or(content.len(), |offset| start + offset);
    let body = &content[start..end];
    assert!(body.contains("toast_tasks"), "Toast 定时器必须纳入 TaskHub");
    assert!(!body.contains(".detach()"), "Toast 定时器不得脱缰 detach");
}

/// 渲染主路径不得直接执行帧副作用或弹窗对账。
#[test]
fn render_effects_are_deferred() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let render =
        std::fs::read_to_string(root.join("src/app/render.rs")).expect("读取外壳渲染文件失败");
    assert!(
        render.contains("schedule_frame_effects("),
        "渲染应只安排延迟副作用"
    );
    for forbidden in [
        "run_frame_effects(",
        "reconcile_dialogs(",
        "pending_notices.drain()",
    ] {
        assert!(
            !render.contains(forbidden),
            "Render::render 不得直接执行副作用：{forbidden}"
        );
    }
}
