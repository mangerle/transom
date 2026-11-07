use super::{
    ChangeItem, ContainerInspect, ContainerListItem, DockerEventMessage, HealthInfo, ImageListItem,
    ImagePruneResponse, NetworkItem, NetworkSettings, StatsResponse, SystemDfResponse, TopResponse,
    VersionResponse, VolumeItem, VolumePruneResponse,
};
use transom_core::ContainerState;

#[test]
fn container_list_item_tolerates_extra_fields_and_null_repo_tags() {
    let json = r#"[
        {
            "Id": "sha256:abc123",
            "Names": ["/web"],
            "Image": "nginx:latest",
            "ImageID": "sha256:def",
            "Created": 1700000000,
            "Ports": [{"IP": "0.0.0.0", "PrivatePort": 80, "PublicPort": 8080, "Type": "tcp"}],
            "Labels": {"com.example": "1"},
            "State": "running",
            "Status": "Up 3 hours",
            "Mounts": []
        }
    ]"#;
    let items: Vec<ContainerListItem> = serde_json::from_str(json).expect("列表必须能反序列化");
    let summary = items.into_iter().next().expect("至少一条").into_summary();
    assert_eq!(summary.name, "web");
    assert_eq!(summary.state, ContainerState::Running);
    assert_eq!(summary.ports_label(), "8080->80/tcp");

    let images = r#"[{"Id":"sha256:f00","RepoTags":null,"Created":1,"Size":10}]"#;
    let images: Vec<ImageListItem> = serde_json::from_str(images).expect("镜像必须能反序列化");
    let image = images.into_iter().next().expect("至少一个").into_summary();
    assert!(image.tags.is_empty());
    assert_eq!(image.display_name(), "f00");
}

/// 真机回归：命名卷的 `Source` 是宿主挂载点，只有 `Name` 才是卷名。
/// 业务层判断“卷是否在用”和按卷复用容器都必须使用 `Name`。
#[test]
fn named_volume_mount_keeps_name_for_identity() {
    let json = r#"[{
        "Id": "sha256:abc123",
        "Names": ["/web"],
        "Image": "nginx:latest",
        "Created": 1700000000,
        "Ports": [],
        "Labels": {},
        "State": "running",
        "Status": "Up 1 hour",
        "Mounts": [
            {
                "Type": "volume",
                "Name": "app-data",
                "Source": "/var/lib/docker/volumes/app-data/_data",
                "Destination": "/data",
                "Mode": "z",
                "RW": true
            },
            {
                "Type": "bind",
                "Source": "/host/config",
                "Destination": "/config",
                "Mode": "",
                "RW": false
            }
        ]
    }]"#;
    let container: ContainerListItem = serde_json::from_str::<Vec<ContainerListItem>>(json)
        .expect("容器列表必须能反序列化")
        .into_iter()
        .next()
        .expect("至少一条容器");
    let summary = container.into_summary();
    assert_eq!(summary.mounts[0].name.as_deref(), Some("app-data"));
    assert_eq!(summary.mounts[0].volume_name(), Some("app-data"));
    assert_eq!(summary.mounts[1].volume_name(), None);
}

/// 真机回归：内置网络的 `IPAM.Config` 与无标签网络的 `Labels` 会是显式 null，
/// 不能让整个网络列表解析失败。
#[test]
fn network_list_tolerates_null_ipam_and_labels() {
    let json = r#"{
        "Id": "n1",
        "Name": "bridge",
        "Driver": "bridge",
        "Scope": "local",
        "IPAM": { "Driver": "default", "Options": null, "Config": null },
        "Labels": null,
        "Containers": null
    }"#;
    let item: NetworkItem = serde_json::from_str(json).expect("网络列表必须能反序列化 null 字段");
    let summary = item.summary();
    assert_eq!(summary.name, "bridge");
    assert!(summary.subnet.is_none(), "Config 为 null 时应按无子网处理");
}

/// 统计快照要能给出每核、分网卡与分设备明细：这几项是统计页卡片的数据来源。
#[test]
fn stats_report_cores_interfaces_and_devices() {
    let current: StatsResponse = serde_json::from_str(
        r#"{
            "cpu_stats": {
                "cpu_usage": {"total_usage": 160000, "percpu_usage": [80000, 80000]},
                "system_cpu_usage": 2000000,
                "online_cpus": 2
            },
            "precpu_stats": {
                "cpu_usage": {"total_usage": 100000, "percpu_usage": [50000, 50000]},
                "system_cpu_usage": 1000000,
                "online_cpus": 2
            },
            "memory_stats": {
                "usage": 100,
                "limit": 200,
                "stats": {"rss": 60, "inactive_file": 20}
            },
            "networks": {"eth0": {"rx_bytes": 1000, "tx_bytes": 200}},
            "blkio_stats": {
                "io_service_bytes_recursive": [
                    {"major": 8, "minor": 0, "op": "read", "value": 10},
                    {"major": 8, "minor": 0, "op": "write", "value": 20}
                ]
            },
            "pids_stats": {"current": 3}
        }"#,
    )
    .expect("统计响应必须能反序列化");
    let previous: StatsResponse = serde_json::from_str(
        r#"{
            "cpu_stats": {
                "cpu_usage": {"total_usage": 100000, "percpu_usage": [50000, 50000]},
                "system_cpu_usage": 1000000,
                "online_cpus": 2
            }
        }"#,
    )
    .expect("上一帧统计必须能反序列化");
    let stats = current.to_stats(Some(&previous));
    assert!((stats.cpu_percent - 12.0).abs() < 0.01, "总占用");
    assert_eq!(stats.cpu_cores.len(), 2);
    assert!((stats.cpu_cores[0] - 6.0).abs() < 0.01, "每核占用");
    assert_eq!(stats.memory_rss, 60);
    assert_eq!(stats.memory_cache, 20);
    assert_eq!(stats.networks[0].name, "eth0");
    assert_eq!(stats.networks[0].rx_bytes, 1000);
    assert_eq!(stats.block_devices[0].device, "8:0");
    assert_eq!(stats.block_devices[0].read_bytes, 10);
    assert_eq!(stats.block_devices[0].write_bytes, 20);
}

/// `docker top` 的列顺序不固定，按标题定位。
#[test]
fn top_response_maps_columns_by_title() {
    let top: TopResponse = serde_json::from_str(
        r#"{
            "Titles": ["PID", "USER", "%CPU", "%MEM", "COMMAND"],
            "Processes": [
                ["1", "root", "0.0", "0.1", "sleep 3600"],
                ["7", "nobody", "1.5", "0.2", "sh -c echo hi"]
            ]
        }"#,
    )
    .expect("top 响应必须能反序列化");
    let processes = top.into_processes();
    assert_eq!(processes.len(), 2);
    assert_eq!(processes[0].pid, "1");
    assert_eq!(processes[0].user, "root");
    assert!((processes[1].cpu_percent - 1.5).abs() < f32::EPSILON);
    assert_eq!(processes[1].command, "sh -c echo hi");
}

/// 清理接口在「没有可删条目」时返回 null 数组，不能因此报解析失败。
#[test]
fn prune_responses_tolerate_null_arrays() {
    let images: ImagePruneResponse =
        serde_json::from_str(r#"{"ImagesDeleted":null,"SpaceReclaimed":0}"#)
            .expect("镜像清理响应必须能反序列化");
    assert!(images.images_deleted.is_empty());
    let volumes: VolumePruneResponse =
        serde_json::from_str(r#"{"VolumesDeleted":null,"SpaceReclaimed":0}"#)
            .expect("卷清理响应必须能反序列化");
    assert!(volumes.volumes_deleted.is_empty());
}

#[test]
fn version_detects_podman_from_platform() {
    let json = r#"{"Version":"5.2.0","ApiVersion":"1.41","Platform":{"Name":"Podman Engine"}}"#;
    let version: VersionResponse = serde_json::from_str(json).expect("版本必须能反序列化");
    assert!(version.is_podman());
    assert_eq!(version.platform_name(), "Podman Engine");
}

/// 真机回归：Docker 29 返回 `"Platform":{"Name":""}`，展示必须落到 `os/arch`，
/// 且空平台名不能被误判成 Podman。
#[test]
fn blank_platform_falls_back_to_os_arch() {
    let json = r#"{"Version":"29.1.3","ApiVersion":"1.52","Platform":{"Name":""},"Os":"linux","Arch":"amd64","Components":[{"Name":"Engine"}]}"#;
    let version: VersionResponse =
        serde_json::from_str(json).expect("Docker 29 版本响应必须能反序列化");
    assert!(!version.is_podman());
    assert_eq!(version.platform_name(), "linux/amd64");

    let bare: VersionResponse =
        serde_json::from_str(r#"{"Version":"1"}"#).expect("裸版本必须能反序列化");
    assert_eq!(bare.platform_name(), "Docker/Podman");
}

#[test]
fn inspect_tty_flag_defaults_to_false() {
    let inspect: ContainerInspect =
        serde_json::from_str(r#"{"Id":"x","Config":{}}"#).expect("检视必须能反序列化");
    assert!(!inspect.config.expect("配置存在").tty);
}

/// 顶层 `Image` 是镜像 ID，`Config.Image` 才是镜像引用，不能混用。
#[test]
fn inspect_keeps_image_reference_and_image_id_separate() {
    let inspect: ContainerInspect = serde_json::from_str(
        r#"{"Id":"cid","Image":"sha256:deadbeef","Config":{"Image":"nginx:latest"}}"#,
    )
    .expect("检视必须能反序列化");
    let details = inspect.into_details(String::new());
    assert_eq!(details.image, "nginx:latest");
    assert_eq!(details.image_id, "sha256:deadbeef");
}

/// 真机回归：Docker 对无标签数据卷返回 `"Labels": null`，端口未映射时
/// `"Ports": null`；两者都必须收敛成空集合而不是报解析错误。
#[test]
fn null_labels_and_ports_deserialize_as_empty() {
    let volume: VolumeItem = serde_json::from_str(
        r#"{"Name":"plain","Driver":"local","Mountpoint":"/var/lib/docker/volumes/plain/_data","CreatedAt":"2026-10-30T23:25:09+08:00","Labels":null,"Options":null,"Scope":"local"}"#,
    )
    .expect("空标签数据卷必须能反序列化");
    assert!(volume.labels.is_empty());
    assert_eq!(volume.into_summary().name, "plain");

    let settings: NetworkSettings =
        serde_json::from_str(r#"{"Ports":null}"#).expect("空端口必须能反序列化");
    assert!(settings.ports.is_empty());
}

#[test]
fn health_info_deserialization_and_details_conversion() {
    let json = r#"{
        "Status": "unhealthy",
        "FailingStreak": 3,
        "Log": [
            {
                "Start": "2026-11-04T12:00:00Z",
                "End": "2026-11-04T12:00:01Z",
                "ExitCode": 1,
                "Output": "curl: (7) Failed to connect"
            }
        ]
    }"#;
    let health: HealthInfo = serde_json::from_str(json).expect("健康信息必须能反序列化");
    assert_eq!(health.status.as_deref(), Some("unhealthy"));
    assert_eq!(health.failing_streak, 3);
    assert_eq!(health.log.len(), 1);

    let details = health
        .into_health_details()
        .expect("必须能转成健康诊断详情");
    assert_eq!(details.status, "unhealthy");
    assert_eq!(details.failing_streak, 3);
    assert_eq!(details.log[0].exit_code, 1);
    assert_eq!(details.log[0].output, "curl: (7) Failed to connect");
}

#[test]
fn change_item_deserialization_and_conversion() {
    let json = r#"[
        {"Path": "/etc/hosts", "Kind": 0},
        {"Path": "/app/new_file.txt", "Kind": 1},
        {"Path": "/tmp/old.log", "Kind": 2}
    ]"#;
    let items: Vec<ChangeItem> = serde_json::from_str(json).expect("必须能反序列化变动列表");
    assert_eq!(items.len(), 3);
    assert_eq!(items[0].kind, 0);
    let mut iter = items.into_iter();
    assert_eq!(
        iter.next().unwrap().into_change().kind,
        transom_core::ContainerPathChangeKind::Modified
    );
    assert_eq!(
        iter.next().unwrap().into_change().kind,
        transom_core::ContainerPathChangeKind::Added
    );
    assert_eq!(
        iter.next().unwrap().into_change().kind,
        transom_core::ContainerPathChangeKind::Deleted
    );
}

#[test]
fn system_df_deserialization_and_conversion() {
    let json = r#"{
        "LayersSize": 1024000,
        "Images": [
            {"Id": "img1", "Size": 1000, "SharedSize": 0, "Containers": 0},
            {"Id": "img2", "Size": 2000, "SharedSize": 0, "Containers": 1}
        ],
        "Containers": [
            {"Id": "c1", "SizeRw": 500, "SizeRootFs": 3000}
        ],
        "Volumes": [
            {"Name": "v1", "UsageData": {"Size": 800, "RefCount": 0}},
            {"Name": "v2", "UsageData": {"Size": 1200, "RefCount": 2}}
        ],
        "BuildCache": [
            {"ID": "bc1", "Size": 600, "InUse": false, "Shared": false}
        ]
    }"#;
    let resp: SystemDfResponse =
        serde_json::from_str(json).expect("必须成功反序列化 SystemDfResponse");
    let usage = resp.into_disk_usage();
    assert_eq!(usage.images_count, 2);
    assert_eq!(usage.images_size, 3000);
    assert_eq!(usage.images_reclaimable, 1000);
    assert_eq!(usage.containers_count, 1);
    assert_eq!(usage.containers_size, 500);
    assert_eq!(usage.volumes_count, 2);
    assert_eq!(usage.volumes_size, 2000);
    assert_eq!(usage.volumes_reclaimable, 800);
    assert_eq!(usage.build_cache_count, 1);
    assert_eq!(usage.build_cache_size, 600);
    assert_eq!(usage.build_cache_reclaimable, 600);
    assert_eq!(usage.total_size, 6100);
    assert_eq!(usage.total_reclaimable, 2900);
}

#[test]
fn test_docker_event_deserialize_and_convert() {
    let json = r#"{
        "Type": "container",
        "Action": "die",
        "Actor": {
            "ID": "1234567890abcdef",
            "Attributes": {
                "name": "web_server",
                "image": "nginx:alpine",
                "exitCode": "137"
            }
        },
        "time": 1700000000
    }"#;
    let event_msg: DockerEventMessage =
        serde_json::from_str(json).expect("必须成功解析 DockerEventMessage");
    let event = event_msg
        .into_engine_event()
        .expect("必须成功转换为 EngineEvent");
    assert_eq!(event.resource_type, "container");
    assert_eq!(event.action, "die");
    assert_eq!(event.id, "1234567890abcdef");
    assert_eq!(event.name.as_deref(), Some("web_server"));
    assert_eq!(event.image.as_deref(), Some("nginx:alpine"));
    assert_eq!(event.exit_code, Some(137));
    assert_eq!(event.timestamp, 1700000000);
    assert!(event.is_abnormal_container_exit());
}
