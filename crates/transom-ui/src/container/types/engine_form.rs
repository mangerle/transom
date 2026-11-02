//! 引擎连接表单的状态与校验。

use super::*;

/// 容器表单状态。
pub(crate) struct ContainerFormState {
    pub(crate) editing: Option<EngineProfileId>,
    pub(crate) name: Entity<InputState>,
    pub(crate) kind: ContainerEngineKind,
    pub(crate) endpoint_choice: EndpointChoice,
    pub(crate) endpoint_value: Entity<InputState>,
    /// 测试连接结果提示。
    pub(crate) test_status: Option<String>,
    pub(crate) test_ok: bool,
    pub(crate) test_request: Option<ContainerRequestId>,
}

impl ContainerFormState {
    /// 脏判定用的文本快照：名称与端点值。
    pub(crate) fn text_values(&self, cx: &App) -> Vec<String> {
        vec![
            self.name.read(cx).value().to_string(),
            self.endpoint_value.read(cx).value().to_string(),
        ]
    }

    pub(crate) fn new(
        window: &mut Window,
        cx: &mut Context<crate::app::AppShell>,
        profile: Option<&ContainerEngineProfile>,
    ) -> Self {
        let name_value = profile
            .map(|profile| profile.name.clone())
            .unwrap_or_default();
        let engine = profile
            .map(|profile| profile.engine.clone())
            .unwrap_or_else(|| ContainerEngineRef::auto(ContainerEngineKind::Docker));
        let endpoint_choice = EndpointChoice::from_endpoint(&engine.endpoint);
        let endpoint_value = EndpointChoice::value_for(&engine.endpoint);
        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(name_value)
                .placeholder(rust_i18n::t!("container.form.name_placeholder").to_string())
        });
        let endpoint = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(endpoint_value)
                .placeholder(rust_i18n::t!("container.form.endpoint_placeholder").to_string())
        });
        Self {
            editing: profile.map(|profile| profile.id),
            name,
            kind: engine.kind,
            endpoint_choice,
            endpoint_value: endpoint,
            test_status: None,
            test_ok: false,
            test_request: None,
        }
    }

    /// 构造保存/测试用的连接配置。
    ///
    /// # Errors
    /// 名称为空、需要端点值却留空，或端点在当前平台不可用时返回中文提示。
    pub(crate) fn to_profile(&self, cx: &App) -> Result<ContainerEngineProfile, String> {
        let name = self.name.read(cx).value().trim().to_string();
        if name.is_empty() {
            return Err(rust_i18n::t!("container.form.name_required").to_string());
        }
        let value = self.endpoint_value.read(cx).value().to_string();
        if self.endpoint_choice.needs_value() && value.trim().is_empty() {
            return Err(rust_i18n::t!("container.form.endpoint_required").to_string());
        }
        let endpoint = self.endpoint_choice.to_endpoint(&value);
        endpoint
            .ensure_supported()
            .map_err(|reason| reason.to_string())?;
        let engine = ContainerEngineRef::explicit(self.kind, endpoint);
        let mut profile = ContainerEngineProfile::new(name, engine);
        if let Some(id) = self.editing {
            profile.id = id;
        }
        Ok(profile)
    }
}

#[cfg(test)]
mod tests {
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
    fn checkbox_membership_and_batch_arm_toggle() {
        let mut set = std::collections::HashSet::new();
        super::toggle_membership(&mut set, "a");
        assert_eq!(set.len(), 1, "首次点击加入");
        super::toggle_membership(&mut set, "a");
        assert!(set.is_empty(), "再次点击移除");
        super::toggle_membership(&mut set, "b");
        super::toggle_membership(&mut set, "c");
        assert_eq!(set.len(), 2);

        assert_eq!(
            super::arm_or_fire(false, 3),
            (true, false),
            "第一次只是武装"
        );
        assert_eq!(super::arm_or_fire(true, 3), (false, true), "第二次才下发");
        assert_eq!(
            super::arm_or_fire(true, 0),
            (false, false),
            "没有选中项就不下发"
        );
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
            !ContainerSortKey::for_section(ContainerSection::Volumes)
                .contains(&ContainerSortKey::Size)
        );
        assert!(
            !ContainerSortKey::for_section(ContainerSection::Images)
                .contains(&ContainerSortKey::State)
        );
        assert!(
            ContainerSortKey::for_section(ContainerSection::Networks) == [ContainerSortKey::Name]
        );
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
}
