use super::backup::is_volume_already_exists;
use super::probe::select_probe_image;
use super::{map_volume_path, temp_path, unmap_volume_path};
use crate::error::ContainerError;

#[test]
fn test_volume_path_mapping() {
    let mount = "/transom_vol";
    assert_eq!(map_volume_path(mount, "/").unwrap(), "/transom_vol");
    assert_eq!(
        map_volume_path(mount, "/sub/dir").unwrap(),
        "/transom_vol/sub/dir"
    );
    assert_eq!(
        map_volume_path("/mnt/data/", "/file.txt").unwrap(),
        "/mnt/data/file.txt"
    );

    assert_eq!(unmap_volume_path(mount, "/transom_vol"), "/");
    assert_eq!(unmap_volume_path(mount, "/transom_vol/sub/dir"), "/sub/dir");
    assert_eq!(
        unmap_volume_path("/mnt/data", "/mnt/data/file.txt"),
        "/file.txt"
    );
}

#[test]
fn test_volume_temp_path() {
    use std::path::Path;
    let dest = Path::new("/tmp/my_vol.tar");
    let temp = temp_path(dest);
    assert_eq!(temp, Path::new("/tmp/my_vol.tar.transom-part"));
}

#[test]
fn test_select_probe_image() {
    use transom_core::ContainerImageSummary;

    let empty: Vec<ContainerImageSummary> = Vec::new();
    assert_eq!(select_probe_image(&empty), None);

    let regular_img = ContainerImageSummary {
        id: "sha256:111".to_string(),
        tags: vec!["my-custom-service:v1".to_string()],
        size: 1024,
        created: 0,
    };
    let alpine_img = ContainerImageSummary {
        id: "sha256:222".to_string(),
        tags: vec!["docker.io/library/alpine:3.18".to_string()],
        size: 512,
        created: 0,
    };

    // 优先匹配 alpine 关键字
    let images = vec![regular_img.clone(), alpine_img.clone()];
    assert_eq!(
        select_probe_image(&images),
        Some("docker.io/library/alpine:3.18".to_string())
    );

    // 无关键字时回落到有效 tag 镜像
    let images_no_preferred = vec![regular_img.clone()];
    assert_eq!(
        select_probe_image(&images_no_preferred),
        Some("my-custom-service:v1".to_string())
    );
}

#[test]
fn volume_create_conflict_is_tolerated_but_real_failure_is_not() {
    assert!(is_volume_already_exists(&ContainerError::Api {
        operation: "创建数据卷".into(),
        status: 409,
        body: "volume already exists".into(),
    }));
    assert!(!is_volume_already_exists(&ContainerError::Api {
        operation: "创建数据卷".into(),
        status: 500,
        body: "permission denied".into(),
    }));
    assert!(is_volume_already_exists(&ContainerError::Cli {
        command: "docker volume create".into(),
        message: "volume already exists".into(),
    }));
}
