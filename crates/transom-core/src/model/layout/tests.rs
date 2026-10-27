use super::*;

fn top(layout: &ActivityBarLayout) -> Vec<ActivityItemId> {
    layout.top_items().collect()
}

fn bottom(layout: &ActivityBarLayout) -> Vec<ActivityItemId> {
    layout.bottom_items().collect()
}

#[test]
fn item_keys_match_serialized_names() {
    for id in ActivityItemId::DEFAULT_ORDER {
        let serialized = serde_json::to_value(id).unwrap();
        assert_eq!(serialized, serde_json::Value::String(id.key().to_string()));
    }
}

#[test]
fn default_layout_splits_five_and_five() {
    let layout = ActivityBarLayout::default();
    assert_eq!(top(&layout), ActivityItemId::DEFAULT_ORDER[..5].to_vec());
    assert_eq!(bottom(&layout), ActivityItemId::DEFAULT_ORDER[5..].to_vec());
    assert_eq!(layout.split_at(), 5);
}

#[test]
fn normalize_restores_missing_and_drops_unknown_and_duplicates() {
    let mut layout: ActivityBarLayout = serde_json::from_str(
        r#"{
                "order": ["sftp", "sessions", "not_a_real_item", "sftp"],
                "hidden": ["settings", "sftp", "ghost", "sftp"],
                "split_at": 99
            }"#,
    )
    .unwrap();
    layout.normalize();
    assert_eq!(layout.order().len(), ActivityItemId::DEFAULT_ORDER.len());
    for id in ActivityItemId::DEFAULT_ORDER {
        assert_eq!(
            layout.order().iter().filter(|item| **item == id).count(),
            1,
            "每个已知条目必须恰好出现一次"
        );
    }
    assert!(!layout.order().contains(&ActivityItemId::Unknown));
    assert!(!layout.is_hidden(ActivityItemId::Settings));
    assert!(layout.is_hidden(ActivityItemId::Sftp));
    assert_eq!(layout.split_at(), layout.order().len());
}

#[test]
fn normalize_keeps_a_valid_split_point() {
    let mut layout = ActivityBarLayout {
        order: ActivityItemId::DEFAULT_ORDER.to_vec(),
        hidden: Vec::new(),
        split_at: 4,
    };
    layout.normalize();
    assert_eq!(layout.split_at(), 4);
    assert_eq!(top(&layout).len(), 4);
    assert_eq!(bottom(&layout).len(), 6);
}

/// 快捷命令入口已从活动栏移除：旧设置文件里残留的 `"snippets"` 必须被安全剔除，
/// 且不能因为它补回任何条目或让布局失效。
#[test]
fn legacy_snippets_item_is_dropped_without_breaking_layout() {
    let mut layout: ActivityBarLayout = serde_json::from_str(
        r#"{"order":["sessions","snippets","quick_open"],"hidden":["snippets"],"split_at":2}"#,
    )
    .unwrap();
    layout.normalize();
    assert_eq!(layout.order(), ActivityItemId::DEFAULT_ORDER.to_vec());
    for id in ActivityItemId::DEFAULT_ORDER {
        assert!(!layout.is_hidden(id), "剔除未知条目后不应残留任何隐藏项");
    }
    assert_eq!(layout.split_at(), 2);
}

#[test]
fn legacy_settings_without_activity_bar_use_defaults() {
    let settings: crate::AppSettings = serde_json::from_str(r#"{"language":"zh-CN"}"#).unwrap();
    assert_eq!(settings.activity_bar, ActivityBarLayout::default());
    assert_eq!(settings.status_bar, StatusBarLayout::default());
}

#[test]
fn unknown_item_does_not_break_deserialization() {
    let mut layout: ActivityBarLayout =
        serde_json::from_str(r#"{"order":["sessions","future_item","quick_open"]}"#).unwrap();
    layout.normalize();
    assert_eq!(layout.order().len(), ActivityItemId::DEFAULT_ORDER.len());
    assert_eq!(layout.order()[0], ActivityItemId::Sessions);
    assert_eq!(layout.order()[1], ActivityItemId::QuickOpen);
}

#[test]
fn move_within_top_only_reorders() {
    let mut layout = ActivityBarLayout::default();
    assert!(layout.move_relative_to(ActivityItemId::Sftp, ActivityItemId::Sessions, false));
    assert_eq!(
        top(&layout),
        vec![
            ActivityItemId::Sftp,
            ActivityItemId::Sessions,
            ActivityItemId::QuickOpen,
            ActivityItemId::Forwards,
            ActivityItemId::Monitor,
        ]
    );
    assert_eq!(layout.split_at(), 5);
    assert_eq!(bottom(&layout), ActivityItemId::DEFAULT_ORDER[5..].to_vec());
}

#[test]
fn moving_top_to_bottom_start_grows_bottom() {
    let mut layout = ActivityBarLayout::default();
    assert!(layout.move_to_bottom_start(ActivityItemId::Sessions));
    assert_eq!(layout.split_at(), 4);
    assert_eq!(
        top(&layout),
        vec![
            ActivityItemId::QuickOpen,
            ActivityItemId::Forwards,
            ActivityItemId::Sftp,
            ActivityItemId::Monitor,
        ]
    );
    assert_eq!(bottom(&layout)[0], ActivityItemId::Sessions);
}

#[test]
fn moving_bottom_to_top_end_grows_top() {
    let mut layout = ActivityBarLayout::default();
    assert!(layout.move_to_top_end(ActivityItemId::Settings));
    assert_eq!(layout.split_at(), 6);
    assert_eq!(top(&layout)[5], ActivityItemId::Settings);
    assert!(!bottom(&layout).contains(&ActivityItemId::Settings));
}

#[test]
fn dropping_on_top_anchor_moves_item_into_top() {
    let mut layout = ActivityBarLayout::default();
    assert!(layout.move_relative_to(ActivityItemId::Keys, ActivityItemId::Sessions, true));
    assert_eq!(layout.split_at(), 6);
    let top = top(&layout);
    assert_eq!(top[0], ActivityItemId::Sessions);
    assert_eq!(top[1], ActivityItemId::Keys);
    assert!(!bottom(&layout).contains(&ActivityItemId::Keys));
}

#[test]
fn dropping_on_bottom_anchor_moves_item_into_bottom() {
    let mut layout = ActivityBarLayout::default();
    assert!(layout.move_relative_to(ActivityItemId::Sessions, ActivityItemId::Language, true));
    assert_eq!(layout.split_at(), 4);
    assert_eq!(top(&layout).len(), 4);
    let bottom = bottom(&layout);
    assert_eq!(bottom[0], ActivityItemId::Language);
    assert_eq!(bottom[1], ActivityItemId::Sessions);
}

#[test]
fn moving_to_current_boundary_is_a_no_op() {
    let mut layout = ActivityBarLayout::default();
    assert!(!layout.move_to_top_end(ActivityItemId::Monitor));
    assert!(!layout.move_to_bottom_start(ActivityItemId::Language));
    assert!(!layout.move_relative_to(ActivityItemId::Sftp, ActivityItemId::Sftp, true));
    assert_eq!(layout, ActivityBarLayout::default());
}

#[test]
fn pinned_item_cannot_be_hidden_but_can_move() {
    let mut layout = ActivityBarLayout::default();
    assert!(!layout.set_visible(ActivityItemId::Settings, false));
    assert!(layout.set_visible(ActivityItemId::Sftp, false));
    assert!(!layout.set_visible(ActivityItemId::Sftp, false));
    assert!(layout.set_visible(ActivityItemId::Sftp, true));
    assert!(layout.move_to_top_end(ActivityItemId::Settings));
    assert_eq!(top(&layout)[5], ActivityItemId::Settings);
}

#[test]
fn restore_defaults_resets_order_hidden_and_split() {
    let mut layout = ActivityBarLayout::default();
    layout.set_visible(ActivityItemId::Sftp, false);
    layout.move_to_top_end(ActivityItemId::Settings);
    assert!(layout.restore_defaults());
    assert_eq!(layout, ActivityBarLayout::default());
    assert!(!layout.restore_defaults());
}

#[test]
fn status_default_order_matches_visible_items() {
    let layout = StatusBarLayout::default();
    assert_eq!(
        layout.visible_items().collect::<Vec<_>>(),
        StatusItemId::DEFAULT_ORDER.to_vec()
    );
}

#[test]
fn status_item_keys_match_serialized_names() {
    for id in StatusItemId::DEFAULT_ORDER {
        let serialized = serde_json::to_value(id).unwrap();
        assert_eq!(serialized, serde_json::Value::String(id.key().to_string()));
    }
}

#[test]
fn status_normalize_restores_missing_and_drops_unknown() {
    let mut layout: StatusBarLayout = serde_json::from_str(
        r#"{
                "order": ["disk", "network", "future_item", "disk"],
                "hidden": ["uptime", "ghost", "uptime"]
            }"#,
    )
    .unwrap();
    layout.normalize();
    assert_eq!(layout.order().len(), StatusItemId::DEFAULT_ORDER.len());
    for id in StatusItemId::DEFAULT_ORDER {
        assert_eq!(
            layout.order().iter().filter(|item| **item == id).count(),
            1,
            "每个已知条目必须恰好出现一次"
        );
    }
    assert!(!layout.order().contains(&StatusItemId::Unknown));
    assert!(layout.is_hidden(StatusItemId::Uptime));
}

#[test]
fn status_move_relative_reorders_single_row() {
    let mut layout = StatusBarLayout::default();
    assert!(layout.move_relative_to(StatusItemId::Network, StatusItemId::Disk, true));
    assert_eq!(
        layout.visible_items().collect::<Vec<_>>(),
        vec![
            StatusItemId::Cpu,
            StatusItemId::Memory,
            StatusItemId::Disk,
            StatusItemId::Network,
            StatusItemId::Os,
            StatusItemId::Uptime,
        ]
    );
    assert!(layout.move_relative_to(StatusItemId::Uptime, StatusItemId::Cpu, false));
    assert_eq!(layout.visible_items().next(), Some(StatusItemId::Uptime));
}

#[test]
fn status_move_to_current_position_is_a_no_op() {
    let mut layout = StatusBarLayout::default();
    assert!(!layout.move_relative_to(StatusItemId::Network, StatusItemId::Cpu, false));
    assert!(!layout.move_relative_to(StatusItemId::Network, StatusItemId::Network, true));
    assert!(!layout.move_to_front(StatusItemId::Network));
    assert_eq!(layout, StatusBarLayout::default());
}

#[test]
fn status_move_to_front_places_item_first() {
    let mut layout = StatusBarLayout::default();
    assert!(layout.move_to_front(StatusItemId::Uptime));
    assert_eq!(layout.visible_items().next(), Some(StatusItemId::Uptime));
    assert_eq!(layout.order().len(), StatusItemId::DEFAULT_ORDER.len());
}

#[test]
fn status_hide_show_and_restore() {
    let mut layout = StatusBarLayout::default();
    assert!(layout.set_visible(StatusItemId::Cpu, false));
    assert!(!layout.is_hidden(StatusItemId::Network));
    assert!(layout.is_hidden(StatusItemId::Cpu));
    assert!(!layout.set_visible(StatusItemId::Cpu, false));
    assert!(layout.set_visible(StatusItemId::Cpu, true));
    assert!(!layout.set_visible(StatusItemId::Cpu, true));
    assert!(layout.move_relative_to(StatusItemId::Uptime, StatusItemId::Network, false));
    assert!(layout.restore_defaults());
    assert_eq!(layout, StatusBarLayout::default());
    assert!(!layout.restore_defaults());
}
