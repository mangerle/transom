use super::*;

#[test]
fn test_timeline_uniform() {
    let timeline = StatusTimeline::uniform(HourlyStatus::Running);
    for bucket in timeline.buckets {
        assert_eq!(bucket, HourlyStatus::Running);
    }
}

#[test]
fn test_timeline_default_is_unknown() {
    let timeline = StatusTimeline::default();
    for bucket in timeline.buckets {
        assert_eq!(bucket, HourlyStatus::Unknown);
    }
}

#[test]
fn test_timeline_infer_running() {
    let timeline = StatusTimeline::infer(None, ContainerState::Running);
    assert_eq!(timeline.buckets[23], HourlyStatus::Running);
}

#[test]
fn test_timeline_infer_oom_killed() {
    let details = ContainerDetails {
        id: "cid-1".to_string(),
        name: "oom-app".to_string(),
        image: "test:latest".to_string(),
        image_id: "sha256:123".to_string(),
        command: "app".to_string(),
        entrypoint: Vec::new(),
        user: None,
        working_dir: None,
        env: Vec::new(),
        state: ContainerState::Exited,
        status: "Exited (137)".to_string(),
        created: 100,
        started_at: None,
        finished_at: None,
        exit_code: Some(137),
        health: None,
        health_details: None,
        restart_policy: None,
        ports: Vec::new(),
        mounts: Vec::new(),
        compose_project: None,
        oom_killed: true,
        size_rw: 0,
        size_root_fs: 0,
        exposed_ports: Vec::new(),
        inspect: String::new(),
    };

    let timeline = StatusTimeline::infer(Some(&details), ContainerState::Exited);
    assert_eq!(timeline.buckets[23], HourlyStatus::OomKilled);
}

#[test]
fn test_timeline_infer_newly_started_only_current_slot() {
    // 模拟当前时间 1730000000，容器是在 10 分钟前创建并启动的
    let now = 1730000000;
    let created = now - 600;
    let started_at = chrono::DateTime::from_timestamp(now - 600, 0)
        .unwrap()
        .to_rfc3339();

    let details = ContainerDetails {
        id: "cid-redis".to_string(),
        name: "redis".to_string(),
        image: "redis:latest".to_string(),
        image_id: "sha256:456".to_string(),
        command: "redis-server".to_string(),
        entrypoint: Vec::new(),
        user: None,
        working_dir: None,
        env: Vec::new(),
        state: ContainerState::Running,
        status: "Up 10 minutes".to_string(),
        created,
        started_at: Some(started_at),
        finished_at: None,
        exit_code: None,
        health: None,
        health_details: None,
        restart_policy: None,
        ports: Vec::new(),
        mounts: Vec::new(),
        compose_project: None,
        oom_killed: false,
        size_rw: 0,
        size_root_fs: 0,
        exposed_ports: Vec::new(),
        inspect: String::new(),
    };

    let timeline = StatusTimeline::infer_at(Some(&details), ContainerState::Running, now);
    // 最近这 1 小时是运行中
    assert_eq!(timeline.buckets[23], HourlyStatus::Running);
    // 1 小时前至 24 小时前，容器尚未创建，必须为 Unknown（无记录），绝对不能显示为运行中
    for (hour, status) in timeline.buckets.iter().enumerate().take(23) {
        assert_eq!(
            *status,
            HourlyStatus::Unknown,
            "第 {hour} 小时槽应当为 Unknown，但被误判"
        );
    }
}

#[test]
fn test_timeline_infer_long_running() {
    // 模拟运行了 3 天（72 小时）的容器
    let now = 1730000000;
    let created = now - 72 * 3600;
    let started_at = chrono::DateTime::from_timestamp(now - 72 * 3600, 0)
        .unwrap()
        .to_rfc3339();

    let details = ContainerDetails {
        id: "cid-long".to_string(),
        name: "postgres".to_string(),
        image: "postgres:16".to_string(),
        image_id: "sha256:789".to_string(),
        command: "postgres".to_string(),
        entrypoint: Vec::new(),
        user: None,
        working_dir: None,
        env: Vec::new(),
        state: ContainerState::Running,
        status: "Up 3 days".to_string(),
        created,
        started_at: Some(started_at),
        finished_at: None,
        exit_code: None,
        health: None,
        health_details: None,
        restart_policy: None,
        ports: Vec::new(),
        mounts: Vec::new(),
        compose_project: None,
        oom_killed: false,
        size_rw: 0,
        size_root_fs: 0,
        exposed_ports: Vec::new(),
        inspect: String::new(),
    };

    let timeline = StatusTimeline::infer_at(Some(&details), ContainerState::Running, now);
    // 过去 24 小时全程处于稳定运行中，24 格全部为 Running
    for status in &timeline.buckets {
        assert_eq!(*status, HourlyStatus::Running);
    }
}

#[test]
fn test_timeline_update_current() {
    let mut timeline = StatusTimeline::default();
    timeline.update_current(HourlyStatus::Running);
    assert_eq!(timeline.buckets[23], HourlyStatus::Running);
}

#[test]
fn test_timeline_infer_recently_stopped_shows_exited_for_current_slot() {
    // 模拟当前时间，容器在 10 分钟前启动，在 2 分钟前退出
    let now = 1730000000;
    let created = now - 600;
    let started_at = chrono::DateTime::from_timestamp(now - 600, 0)
        .unwrap()
        .to_rfc3339();
    let finished_at = chrono::DateTime::from_timestamp(now - 120, 0)
        .unwrap()
        .to_rfc3339();

    let details = ContainerDetails {
        id: "cid-redis".to_string(),
        name: "redis".to_string(),
        image: "redis:latest".to_string(),
        image_id: "sha256:456".to_string(),
        command: "redis-server".to_string(),
        entrypoint: Vec::new(),
        user: None,
        working_dir: None,
        env: Vec::new(),
        state: ContainerState::Exited,
        status: "Exited (0) 2 minutes ago".to_string(),
        created,
        started_at: Some(started_at),
        finished_at: Some(finished_at),
        exit_code: Some(0),
        health: None,
        health_details: None,
        restart_policy: None,
        ports: Vec::new(),
        mounts: Vec::new(),
        compose_project: None,
        oom_killed: false,
        size_rw: 0,
        size_root_fs: 0,
        exposed_ports: Vec::new(),
        inspect: String::new(),
    };

    let timeline = StatusTimeline::infer_at(Some(&details), ContainerState::Exited, now);
    // 当前时刻已停止，最右侧槽位（第 23 格）必须为 Exited，杜绝显示为 Running
    assert_eq!(timeline.buckets[23], HourlyStatus::Exited);
    // 1 小时前容器尚未创建，必须为 Unknown
    for status in timeline.buckets.iter().take(23) {
        assert_eq!(*status, HourlyStatus::Unknown);
    }
}

#[test]
fn test_timeline_infer_stopped_after_running() {
    // 模拟运行了 5 小时后在当前这 1 小时内退出的容器
    let now = 1730000000;
    let created = now - 5 * 3600;
    let started_at = chrono::DateTime::from_timestamp(now - 5 * 3600, 0)
        .unwrap()
        .to_rfc3339();
    let finished_at = chrono::DateTime::from_timestamp(now - 1800, 0)
        .unwrap()
        .to_rfc3339();

    let details = ContainerDetails {
        id: "cid-app".to_string(),
        name: "app".to_string(),
        image: "app:latest".to_string(),
        image_id: "sha256:789".to_string(),
        command: "app".to_string(),
        entrypoint: Vec::new(),
        user: None,
        working_dir: None,
        env: Vec::new(),
        state: ContainerState::Exited,
        status: "Exited (0) 30 minutes ago".to_string(),
        created,
        started_at: Some(started_at),
        finished_at: Some(finished_at),
        exit_code: Some(0),
        health: None,
        health_details: None,
        restart_policy: None,
        ports: Vec::new(),
        mounts: Vec::new(),
        compose_project: None,
        oom_killed: false,
        size_rw: 0,
        size_root_fs: 0,
        exposed_ports: Vec::new(),
        inspect: String::new(),
    };

    let timeline = StatusTimeline::infer_at(Some(&details), ContainerState::Exited, now);
    // 当前时刻已退出
    assert_eq!(timeline.buckets[23], HourlyStatus::Exited);
    // 1~4 小时前的槽位全程在运行，必须为 Running
    for (hour, status) in timeline.buckets.iter().enumerate().skip(19).take(4) {
        assert_eq!(
            *status,
            HourlyStatus::Running,
            "第 {hour} 小时槽应当为 Running"
        );
    }
}
