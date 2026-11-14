use super::*;

#[test]
fn image_prune_filter_distinguishes_dangling_and_all_unused() {
    assert_eq!(image_prune_filter(false), r#"{"dangling":["true"]}"#);
    assert_eq!(image_prune_filter(true), r#"{"dangling":["false"]}"#);
}

#[test]
fn reference_splits_tag_without_confusing_registry_ports() {
    assert_eq!(
        split_reference("nginx"),
        ("nginx".to_string(), Some("latest".to_string()))
    );
    assert_eq!(
        split_reference("ghcr.io/team/app:v1"),
        ("ghcr.io/team/app".to_string(), Some("v1".to_string()))
    );
    assert_eq!(
        split_reference("localhost:5000/app"),
        ("localhost:5000/app".to_string(), Some("latest".to_string()))
    );
    assert_eq!(
        split_reference("alpine@sha256:abc"),
        ("alpine@sha256:abc".to_string(), None)
    );
}
