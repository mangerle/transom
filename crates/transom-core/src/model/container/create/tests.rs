use super::{ContainerCreateSpec, ContainerNetworkSpec, ContainerPortPublish};

fn spec() -> ContainerCreateSpec {
    ContainerCreateSpec {
        name: "web".to_string(),
        image: "nginx:latest".to_string(),
        command: vec![
            "nginx".to_string(),
            "-g".to_string(),
            "daemon off;".to_string(),
        ],
        env: vec!["FOO=bar".to_string()],
        ports: vec![ContainerPortPublish {
            host: 8080,
            container: 80,
            protocol: "TCP".to_string(),
        }],
        binds: vec!["data:/usr/share/nginx/html:ro".to_string()],
        restart: Some("always".to_string()),
        health_cmd: Some("pg_isready".to_string()),
        health_interval_secs: Some(5),
        memory_mb: Some(256),
        nano_cpus: Some(500_000_000),
        ..ContainerCreateSpec::default()
    }
}

#[test]
fn valid_spec_normalizes_input() {
    let mut item = spec();
    item.name = "  web-1  ".to_string();
    item.ports[0].protocol = "TCP".to_string();
    item.validate().expect("合法参数必须通过校验");
    assert_eq!(item.name, "web-1");
    assert_eq!(item.ports[0].protocol, "tcp", "协议统一小写");
}

#[test]
fn blank_image_and_bad_name_are_rejected() {
    let mut item = spec();
    item.image = "   ".to_string();
    assert!(item.validate().is_err(), "镜像必填");
    let mut item = spec();
    item.name = "_web".to_string();
    assert!(item.validate().is_err(), "首字符必须是字母或数字");
    let mut item = spec();
    item.name.clear();
    assert!(item.validate().is_ok(), "留空由引擎生成随机名");
}

#[test]
fn advanced_options_are_validated_and_normalized() {
    let mut item = spec();
    item.network_mode = Some("  host  ".to_string());
    item.labels
        .insert("team".to_string(), " infra ".to_string());
    item.cap_add = vec![" NET_ADMIN ".to_string(), String::new()];
    item.dns = vec![" 1.1.1.1 ".to_string()];
    item.validate().expect("高级选项必须合法");
    assert_eq!(item.network_mode.as_deref(), Some("host"));
    assert_eq!(item.labels.get("team").map(String::as_str), Some("infra"));
    assert_eq!(item.cap_add, vec!["NET_ADMIN"]);
    assert_eq!(item.dns, vec!["1.1.1.1"]);

    let mut item = spec();
    item.auto_remove = true;
    assert!(item.validate().is_err(), "自动删除不能和重启策略同时存在");

    let mut item = spec();
    item.labels.insert(String::new(), "value".to_string());
    assert!(item.validate().is_err(), "空标签键必须拒绝");
}

#[test]
fn ports_and_binds_are_validated() {
    let mut item = spec();
    item.ports[0].host = 0;
    assert!(item.validate().is_err(), "端口不能为 0");

    let mut item = spec();
    item.ports[0].protocol = "sctp".to_string();
    assert!(item.validate().is_err(), "只允许 tcp/udp");

    let mut item = spec();
    item.binds = vec!["data:nginx/html".to_string()];
    assert!(item.validate().is_err(), "容器内路径必须绝对");

    let mut item = spec();
    item.binds = vec!["../etc:/data".to_string()];
    assert!(item.validate().is_err(), "不能包含 .. 段");

    let mut item = spec();
    item.binds = vec!["data:/data:xyz".to_string()];
    assert!(item.validate().is_err(), "挂载模式只允许 ro/rw");

    let mut item = spec();
    item.binds = vec!["  ".to_string()];
    assert!(item.validate().is_ok(), "空白挂载项被丢弃而不是报错");
    assert!(item.binds.is_empty());
}

#[test]
fn env_and_restart_policy_are_validated() {
    let mut item = spec();
    item.env = vec!["FOO".to_string()];
    assert!(item.validate().is_err(), "环境变量必须 KEY=VALUE");

    let mut item = spec();
    item.restart = Some("never".to_string());
    assert!(item.validate().is_err(), "重启策略必须在允许集合内");
}

/// 健康检查留空要归一化成 `None`，否则会下发一个空命令的检查项，容器永远不健康。
#[test]
fn zero_limits_are_dropped_and_absurd_cpu_is_rejected() {
    let mut item = spec();
    item.memory_mb = Some(0);
    item.nano_cpus = Some(0);
    item.validate().expect("0 表示不限制，不该报错");
    assert_eq!(item.memory_mb, None);
    assert_eq!(item.nano_cpus, None);

    let mut item = spec();
    item.nano_cpus = Some(1_000_000_000_000);
    assert!(item.validate().is_err(), "把核数当纳核填要被拦下");
}

#[test]
fn health_interval_requires_a_command() {
    let mut item = spec();
    item.health_interval_secs = Some(0);
    assert!(item.validate().is_err(), "间隔为 0 会被引擎拒绝");

    let mut item = spec();
    item.health_cmd = None;
    item.health_interval_secs = Some(5);
    item.validate().expect("没有命令时间隔无意义");
    assert_eq!(item.health_interval_secs, None, "间隔必须跟着命令一起清掉");
}

#[test]
fn blank_health_command_becomes_none() {
    let mut item = spec();
    item.health_cmd = Some("   ".to_string());
    item.validate().expect("空白健康检查不算非法");
    assert_eq!(item.health_cmd, None);
}

#[test]
fn spec_round_trips_through_json() {
    let json = serde_json::to_value(spec()).expect("必须可序列化");
    let parsed: ContainerCreateSpec = serde_json::from_value(json).expect("必须可反序列化");
    assert_eq!(parsed, spec());
    // 缺字段的旧配置必须仍能解析：全部字段都有默认值。
    let minimal: ContainerCreateSpec =
        serde_json::from_str("{}").expect("空对象必须能解析成默认参数");
    assert!(minimal.image.is_empty());
}

#[test]
fn network_spec_normalizes_blanks_and_validates_shapes() {
    let mut spec = ContainerNetworkSpec {
        name: "  edge  ".to_string(),
        driver: Some("   ".to_string()),
        subnet: Some("172.28.0.0/16".to_string()),
        gateway: Some("172.28.0.1".to_string()),
        internal: true,
    };
    spec.validate().expect("合法参数必须通过");
    assert_eq!(spec.name, "edge", "名称要削掉首尾空白");
    assert_eq!(spec.driver, None, "空白驱动按未填写处理");

    let mut no_cidr = ContainerNetworkSpec {
        name: "edge".to_string(),
        subnet: Some("172.28.0.0".to_string()),
        ..Default::default()
    };
    let error = no_cidr.validate().expect_err("缺掩码的子网必须被拒");
    assert!(error.starts_with("subnet:"), "实际: {error}");

    let mut masked_gateway = ContainerNetworkSpec {
        name: "edge".to_string(),
        gateway: Some("172.28.0.1/16".to_string()),
        ..Default::default()
    };
    let error = masked_gateway.validate().expect_err("带掩码的网关必须被拒");
    assert!(error.starts_with("gateway:"), "实际: {error}");

    let mut blank = ContainerNetworkSpec::default();
    let error = blank.validate().expect_err("空名称必须被拒");
    assert!(error.starts_with("name:"), "实际: {error}");
}
