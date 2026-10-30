use super::*;
use crate::ids::SnippetId;

/// 启动档位互斥、档名稳定：设置页分段按钮按 `ALL` 遍历，档名同时是落盘值。
#[test]
fn startup_enums_have_stable_names() {
    let tabs: Vec<&str> = StartupTabs::ALL.iter().map(|t| t.as_str()).collect();
    assert_eq!(
        tabs,
        vec!["restore_last", "empty_workspace", "new_local_terminal"]
    );
    let sidebar: Vec<&str> = SidebarStartup::ALL.iter().map(|s| s.as_str()).collect();
    assert_eq!(sidebar, vec!["remember_last", "always_show", "always_hide"]);
    for tab in StartupTabs::ALL {
        assert_eq!(StartupTabs::parse(tab.as_str()), tab, "档名要能原样读回");
    }
    for startup in SidebarStartup::ALL {
        assert_eq!(SidebarStartup::parse(startup.as_str()), startup);
    }
}

/// 侧栏初值的三档决策：显式档位必须压过记忆位，只有 RememberLast 才看上次。
#[test]
fn sidebar_startup_overrides_the_remembered_state() {
    let mut window = WindowSettings::default();
    assert!(
        window.sidebar_visible_at_startup(),
        "默认记住上次，且上次是展开"
    );

    window.last_sidebar_visible = false;
    assert!(
        !window.sidebar_visible_at_startup(),
        "RememberLast 应沿用上次的收起状态"
    );

    // 关键回归点：用户某次手动收起后，"始终展开"必须还能救回来，
    // 否则他每次启动都要手动展开一次。
    window.sidebar_startup = SidebarStartup::AlwaysShow;
    assert!(
        window.sidebar_visible_at_startup(),
        "AlwaysShow 要压过记忆位"
    );

    window.last_sidebar_visible = true;
    window.sidebar_startup = SidebarStartup::AlwaysHide;
    assert!(
        !window.sidebar_visible_at_startup(),
        "AlwaysHide 要压过记忆位"
    );
}

/// 老设置文件里残留的 `restore_previous_tabs` 布尔被忽略，新字段按默认档走。
#[test]
fn legacy_restore_switch_is_ignored_without_breaking_load() {
    let window: WindowSettings =
        serde_json::from_str(r#"{"restore_previous_tabs":false,"window_width":1000.0}"#)
            .expect("未知旧键不应让窗口设置加载失败");
    assert_eq!(window.startup_tabs, StartupTabs::RestoreLast);
    assert_eq!(
        window.sidebar_startup,
        SidebarStartup::RememberLast,
        "缺字段应退回默认档，而不是让整个设置加载失败"
    );
    assert!(window.last_sidebar_visible);
}

/// 日志级别来自设置文件（外部输入）：无法识别的值退回默认档 `info`，
/// 既不能让整份设置加载失败，更不能因为一个错值把日志整个关掉。
#[test]
fn log_level_degrades_to_info_on_unknown_value() {
    let loose: AppSettings = serde_json::from_str(r#"{"logs":{"level":"VERBOSE"}}"#).unwrap();
    assert_eq!(loose.logs.level, LogLevel::Trace);

    let unknown: AppSettings = serde_json::from_str(r#"{"logs":{"level":"chatty"}}"#).unwrap();
    assert_eq!(unknown.logs.level, LogLevel::Info);

    let absent: AppSettings = serde_json::from_str(r#"{"logs":{}}"#).unwrap();
    assert_eq!(absent.logs.level, LogLevel::Info);

    let legacy: AppSettings = serde_json::from_str(r#"{"language":"zh-CN"}"#).unwrap();
    assert_eq!(legacy.logs.level, LogLevel::Info);

    // 序列化必须写回稳定小写名，否则往返一次就变成另一个值。
    assert_eq!(serde_json::to_string(&LogLevel::Warn).unwrap(), "\"warn\"");
    assert_eq!(LogLevel::Warn.to_filter(), log::LevelFilter::Warn);
}

#[test]
fn old_settings_without_tab_groups_still_load() {
    let settings: AppSettings = serde_json::from_str(r#"{"language":"zh-CN"}"#).unwrap();
    assert!(settings.window.previous_tab_groups.is_empty());
    assert_eq!(settings.window.previous_tab_sessions.len(), 0);
}

/// 通知开关是后加的：老设置里没有它时按"开启 + 10 秒阈值"降级，
/// 而不是让整个窗口设置加载失败。
#[test]
fn notification_defaults_apply_to_legacy_settings() {
    let mut value = serde_json::to_value(AppSettings::default()).unwrap();
    let window = value["window"].as_object_mut().unwrap();
    window.remove("desktop_notifications");
    window.remove("notify_threshold_secs");
    let settings: AppSettings = serde_json::from_value(value).unwrap();
    assert!(settings.window.desktop_notifications);
    assert_eq!(settings.window.notify_threshold_secs, 10);
}

/// 快捷命令库是后加的字段：老设置文件里没有它也要能加载，并按空库降级。
#[test]
fn settings_without_snippets_still_load() {
    let settings: AppSettings = serde_json::from_str(r#"{"language":"zh-CN"}"#).unwrap();
    assert!(settings.snippets.items.is_empty());
    assert!(!settings.snippets.execute_directly);
}

/// 直接执行默认关闭；显式开启后必须原样往返，避免被清洗逻辑吞掉。
#[test]
fn snippet_execute_mode_defaults_off_and_round_trips() {
    let decoded: SnippetSettings = serde_json::from_str(r#"{"items":[]}"#).unwrap();
    assert!(!decoded.execute_directly);

    let settings = SnippetSettings {
        execute_directly: true,
        ..Default::default()
    };
    let json = serde_json::to_string(&settings).unwrap();
    let decoded: SnippetSettings = serde_json::from_str(&json).unwrap();
    assert!(decoded.execute_directly);
}

#[test]
fn snippets_round_trip_and_reject_invalid_entries() {
    let settings = SnippetSettings {
        execute_directly: false,
        items: vec![
            Snippet {
                id: SnippetId::new(),
                name: "看日志".into(),
                command: "journalctl -f".into(),
            },
            Snippet {
                id: SnippetId::new(),
                name: "  ".into(),
                command: "ls".into(),
            },
            Snippet {
                id: SnippetId::new(),
                name: "空命令".into(),
                command: "".into(),
            },
        ],
    };
    let json = serde_json::to_string(&settings).unwrap();
    let decoded: SnippetSettings = serde_json::from_str(&json).unwrap();
    let cleaned = decoded.sanitized();
    assert_eq!(cleaned.items.len(), 1);
    assert_eq!(cleaned.items[0].name, "看日志");
}

/// 同名条目留着两个只会让底栏出现两个分不清的按钮，后出现的覆盖先出现的。
#[test]
fn snippets_deduplicate_by_name() {
    let settings = SnippetSettings {
        execute_directly: false,
        items: vec![
            Snippet {
                id: SnippetId::new(),
                name: "重启".into(),
                command: "systemctl reboot a".into(),
            },
            Snippet {
                id: SnippetId::new(),
                name: "重启 ".into(),
                command: "systemctl reboot b".into(),
            },
        ],
    };
    let cleaned = settings.sanitized();
    assert_eq!(cleaned.items.len(), 1);
    assert_eq!(cleaned.items[0].command, "systemctl reboot b");
}

/// 超长命令不该把底栏撑坏：超过上限的条目直接判为不合法。
#[test]
fn snippets_enforce_length_bounds() {
    let long = Snippet {
        id: SnippetId::new(),
        name: "n".into(),
        command: "y".repeat(5_000),
    };
    assert!(!long.is_valid());
    let long_name = Snippet {
        id: SnippetId::new(),
        name: "名".repeat(41),
        command: "ls".into(),
    };
    assert!(!long_name.is_valid());
}

#[test]
fn shortcut_overrides_round_trip() {
    let mut settings = AppSettings::default();
    settings
        .shortcuts
        .bindings
        .insert("new_session".into(), vec!["ctrl-shift-q".into()]);
    let json = serde_json::to_string(&settings).unwrap();
    let decoded: AppSettings = serde_json::from_str(&json).unwrap();
    assert_eq!(
        decoded.shortcuts.bindings.get("new_session"),
        Some(&vec!["ctrl-shift-q".to_string()])
    );
}

/// 嵌套布局必须原样往返：窗格数量与每层方向都只能靠树形表达。
#[test]
fn saved_tab_groups_round_trip_nested_layout() {
    let mut settings = AppSettings::default();
    settings.window.previous_tab_groups = vec![SavedTabGroup {
        session: "session-id".into(),
        layout: SavedLayout::Split {
            axis: SplitAxis::Horizontal,
            first: Box::new(SavedLayout::Pane),
            second: Box::new(SavedLayout::Split {
                axis: SplitAxis::Vertical,
                first: Box::new(SavedLayout::Pane),
                second: Box::new(SavedLayout::Pane),
            }),
        },
    }];
    let json = serde_json::to_string(&settings).unwrap();
    let decoded: AppSettings = serde_json::from_str(&json).unwrap();
    assert_eq!(
        decoded.window.previous_tab_groups,
        settings.window.previous_tab_groups
    );
}

/// 设置文件属外部输入：缺 `layout` 字段时退化为单窗格，而不是让整份设置加载失败。
#[test]
fn saved_group_without_layout_degrades_to_single_pane() {
    let decoded: SavedTabGroup = serde_json::from_str(r#"{"session":"local"}"#).unwrap();
    assert_eq!(decoded.layout, SavedLayout::Pane);
}

/// 并排视图是后加字段：老设置没有它时按单标签视图启动，且完整往返不丢方向与焦点。
#[test]
fn saved_tab_tiling_round_trip_and_missing_field() {
    let decoded: AppSettings = serde_json::from_str(r#"{"language":"zh-CN"}"#).unwrap();
    assert!(decoded.window.tab_tilings.is_empty());

    let mut settings = AppSettings::default();
    settings.window.tab_tilings = vec![
        SavedTabTiling {
            rows: 2,
            cols: 2,
            slots: vec![Some(0), None, Some(2), Some(3)],
            focused: 2,
            auto: true,
        },
        SavedTabTiling {
            rows: 1,
            cols: 2,
            slots: vec![Some(4), Some(5)],
            focused: 1,
            auto: false,
        },
    ];
    let json = serde_json::to_string(&settings).unwrap();
    let decoded: AppSettings = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded.window.tab_tilings, settings.window.tab_tilings);

    // 旧版单张快照的字段被忽略，slots 为空 → 启动时按单标签恢复。
    let legacy: SavedTabTiling =
        serde_json::from_str(r#"{"first":0,"second":1,"axis":"vertical","focused_second":true}"#)
            .unwrap();
    assert!(legacy.slots.is_empty());
    assert_eq!(legacy.rows, 0);
}

/// 界面外观是后加的字段：老设置里没有它时按内置默认降级，而不是加载失败。
#[test]
fn settings_without_appearance_still_load() {
    let settings: AppSettings = serde_json::from_str(r#"{"language":"zh-CN"}"#).unwrap();
    assert_eq!(settings.appearance.mode, UiThemeMode::System);
    assert!(settings.appearance.ui_theme.is_empty());
    assert_eq!(settings.appearance.density, UiDensity::Standard);
}

/// 明暗与密度是外部输入：未知值必须落进兜底变体而不是让整份设置加载失败。
#[test]
fn appearance_unknown_values_fall_back_to_defaults() {
    let decoded: AppearanceSettings =
        serde_json::from_str(r#"{"mode":"neon","ui_theme":"Transom Dark","density":"huge"}"#)
            .unwrap();
    assert_eq!(decoded.mode, UiThemeMode::Unknown);
    assert_eq!(decoded.density, UiDensity::Unknown);
    // 兜底值的行为与默认一致，但写盘前必须洗成合法状态。
    assert_eq!(decoded.density.font_size(), UiDensity::Standard.font_size());
    let mut normalized = decoded.clone();
    normalized.normalize();
    assert_eq!(normalized.mode, UiThemeMode::System);
    assert_eq!(normalized.density, UiDensity::Standard);
    assert_eq!(normalized.ui_theme, "Transom Dark");
}

/// 外观偏好必须原样往返：主题选择保留在设置里，与运行期回退解耦。
#[test]
fn appearance_round_trips() {
    let settings = AppearanceSettings {
        mode: UiThemeMode::Dark,
        ui_theme: "Transom Dark".into(),
        density: UiDensity::Roomy,
        ui_font_family: Some("Sarasa Mono SC".into()),
        divider: DividerSettings {
            color: DividerColor::Muted,
            width: DividerWidth::Medium,
        },
    };
    let json = serde_json::to_string(&settings).unwrap();
    let decoded: AppearanceSettings = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, settings);
    assert_eq!(decoded.ui_font_family.as_deref(), Some("Sarasa Mono SC"));
    assert_eq!(decoded.divider.color, DividerColor::Muted);
    assert_eq!(decoded.divider.width, DividerWidth::Medium);
}

/// 分割线是后加的字段：老设置里没有它时必须落进默认档，保持零观感变化。
#[test]
fn appearance_without_divider_uses_defaults() {
    let decoded: AppearanceSettings =
        serde_json::from_str(r#"{"mode":"light","density":"compact"}"#).unwrap();
    assert_eq!(decoded.divider.color, DividerColor::Theme);
    assert_eq!(decoded.divider.width, DividerWidth::Hairline);
}

/// 分割线的颜色与粗细同样来自外部文件：未知值进兜底变体，归一化后写盘合法。
#[test]
fn divider_unknown_values_fall_back_to_defaults() {
    let decoded: AppearanceSettings =
        serde_json::from_str(r#"{"divider":{"color":"neon","width":"huge"}}"#).unwrap();
    assert_eq!(decoded.divider.color, DividerColor::Unknown);
    assert_eq!(decoded.divider.width, DividerWidth::Unknown);
    // 兜底变体在解析口径上与默认档一致，但写盘前必须洗成合法状态。
    assert_eq!(decoded.divider.width.pixels(), 1.0);
    let mut normalized = decoded;
    normalized.normalize();
    assert_eq!(normalized.divider.color, DividerColor::Theme);
    assert_eq!(normalized.divider.width, DividerWidth::Hairline);
}

/// 粗细档位直接折算像素：静态线用边框实现，档位值必须与边框方法一一对应。
#[test]
fn divider_width_pixels_are_stable() {
    let widths: Vec<f32> = DividerWidth::ALL
        .iter()
        .map(|width| width.pixels())
        .collect();
    assert_eq!(widths, vec![1.0, 2.0, 3.0]);
}

/// 密度档位是全局字号契约：mono 与 sans 的差距必须始终落在 2 至 3px，
/// 否则等宽区域（SFTP 预览、指纹）与周边文字的比例会随档位失调。
#[test]
fn density_sans_mono_gap_stays_within_bounds() {
    for density in UiDensity::ALL {
        let gap = density.font_size() - density.mono_font_size();
        assert!(
            (2.0..=3.0).contains(&gap),
            "{density:?} 的字号差距 {gap} 越界"
        );
    }
}
