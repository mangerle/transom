//! 容器工作区纯逻辑的单元测试：筛选、排序、行选择与输入解析。

use super::{
    ContainerQuickFilter, ContainerSection, ContainerSortKey, ContainerStateFilter, SortFields,
    compare_fields, matches_query,
};

fn fields(name: &str, state: &str, created: i64, size: u64) -> SortFields {
    SortFields {
        name: name.to_string(),
        state: state.to_string(),
        created,
        size,
    }
}

fn order(items: &[SortFields], key: ContainerSortKey, desc: bool) -> Vec<String> {
    let mut sorted = items.to_vec();
    sorted.sort_by(|left, right| compare_fields(left, right, key, desc));
    sorted.into_iter().map(|item| item.name).collect()
}

#[test]
fn quick_filters_only_apply_where_they_mean_something() {
    assert_eq!(
        ContainerQuickFilter::for_section(ContainerSection::Volumes),
        [
            ContainerQuickFilter::All,
            ContainerQuickFilter::UnusedVolumes
        ]
    );
    assert_eq!(
        ContainerQuickFilter::for_section(ContainerSection::Containers),
        [ContainerQuickFilter::All]
    );
    assert!(super::volume_passes_filter(ContainerQuickFilter::All, true));
    assert!(!super::volume_passes_filter(
        ContainerQuickFilter::UnusedVolumes,
        true
    ));
    assert!(super::volume_passes_filter(
        ContainerQuickFilter::UnusedVolumes,
        false
    ));
    assert!(super::image_passes_filter(
        ContainerQuickFilter::All,
        &["nginx".to_string()]
    ));
    assert!(!super::image_passes_filter(
        ContainerQuickFilter::DanglingImages,
        &["nginx".to_string()]
    ));
    assert!(super::image_passes_filter(
        ContainerQuickFilter::DanglingImages,
        &[]
    ));
}

#[test]
fn log_lines_reverse_keeps_every_line() {
    let text = "a\nb\n\nc";
    assert_eq!(super::log_lines(text, false), vec!["a", "b", "", "c"]);
    assert_eq!(super::log_lines(text, true), vec!["c", "", "b", "a"]);
    assert!(super::log_lines("", false).is_empty());
}

#[test]
fn filtered_log_lines_matches_keyword_and_reverses() {
    let text = "INFO: starting service\nDEBUG: cache miss\nERROR: failed to connect\nINFO: ready";
    assert_eq!(
        super::filtered_log_lines(text, "info", false),
        vec!["INFO: starting service", "INFO: ready"]
    );
    assert_eq!(
        super::filtered_log_lines(text, "INFO", true),
        vec!["INFO: ready", "INFO: starting service"]
    );
    assert_eq!(
        super::filtered_log_lines(text, "failed", false),
        vec!["ERROR: failed to connect"]
    );
    assert_eq!(super::filtered_log_lines(text, "", false).len(), 4);
}

#[test]
fn state_filter_covers_both_states() {
    assert!(ContainerStateFilter::All.accepts(true));
    assert!(ContainerStateFilter::All.accepts(false));
    assert!(ContainerStateFilter::Running.accepts(true));
    assert!(!ContainerStateFilter::Running.accepts(false));
    assert!(ContainerStateFilter::Stopped.accepts(false));
    assert!(!ContainerStateFilter::Stopped.accepts(true));
}

#[test]
fn sorting_is_stable_across_equal_keys() {
    let items = [
        fields("web", "running", 30, 10),
        fields("api", "running", 10, 30),
        fields("db", "exited", 20, 20),
    ];
    assert_eq!(
        order(&items, ContainerSortKey::Name, false),
        ["api", "db", "web"]
    );
    assert_eq!(
        order(&items, ContainerSortKey::Name, true),
        ["web", "db", "api"]
    );
    assert_eq!(
        order(&items, ContainerSortKey::State, false),
        ["db", "api", "web"],
        "exited 排在 running 之前，同状态按名称收口"
    );
    assert_eq!(
        order(&items, ContainerSortKey::Created, false),
        ["api", "db", "web"]
    );
    assert_eq!(
        order(&items, ContainerSortKey::Size, true),
        ["api", "db", "web"]
    );
}

#[test]
fn each_section_only_offers_valid_sort_keys() {
    assert!(
        !ContainerSortKey::for_section(ContainerSection::Volumes).contains(&ContainerSortKey::Size)
    );
    assert!(
        !ContainerSortKey::for_section(ContainerSection::Images).contains(&ContainerSortKey::State)
    );
    assert!(ContainerSortKey::for_section(ContainerSection::Networks) == [ContainerSortKey::Name]);
    for section in ContainerSection::ALL {
        let keys = ContainerSortKey::for_section(section);
        assert!(!keys.is_empty(), "{section:?} 必须有可用排序键");
        assert!(keys.contains(&ContainerSortKey::default_for(section)));
    }
}

#[test]
fn cpu_cores_convert_to_nanocpus() {
    assert_eq!(super::parse_cores("0.5"), Some(500_000_000));
    assert_eq!(super::parse_cores(" 2 "), Some(2_000_000_000));
    assert_eq!(super::parse_cores(""), None);
    assert_eq!(super::parse_cores("0"), None, "0 表示不限制");
    assert_eq!(super::parse_cores("-1"), None);
    assert_eq!(super::parse_cores("abc"), None);
    assert_eq!(super::parse_cores("NaN"), None);
}

#[test]
fn port_and_list_parsing_follows_run_flags() {
    let ports = super::parse_ports("8080:80, 53:53/udp ,").expect("合法端口映射");
    assert_eq!(ports.len(), 2);
    assert_eq!(ports[0].host, 8080);
    assert_eq!(ports[0].protocol, "tcp");
    assert_eq!(ports[1].container, 53);
    assert_eq!(ports[1].protocol, "udp");
    assert!(super::parse_ports("8080-80").is_err(), "缺分隔符必须报错");
    assert!(super::parse_ports("abc:80").is_err(), "非数字端口必须报错");
    assert_eq!(super::split_list("A=1,,B=2"), vec!["A=1", "B=2"]);
    assert_eq!(
        super::split_argv("  nginx   -g  daemon"),
        vec!["nginx", "-g", "daemon"]
    );
    assert!(super::split_argv("").is_empty());
}

#[test]
fn query_matches_any_field_case_insensitively() {
    assert!(matches_query(
        "nginx",
        &["Web".to_string(), "nginx:latest".to_string()]
    ));
    assert!(!matches_query(
        "redis",
        &["Web".to_string(), "nginx:latest".to_string()]
    ));
    assert!(matches_query("", &["anything".to_string()]));
    assert!(!matches_query("x", &[] as &[String]));
}
