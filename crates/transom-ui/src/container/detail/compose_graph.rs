//! Compose 依赖图算法：YAML depends_on 解析、反向依赖与 DAG 拓扑分层。
//!
//! 设计原理（Why）：
//! - 将 YAML 解析与有向无环依赖图算法从 UI 渲染逻辑中剥离，保持无状态纯函数设计；
//! - 便于编写高覆盖率的单元测试，涵盖标准数组语法、对象语法、循环依赖边界与孤立节点分层。

use std::collections::{HashMap, HashSet};

/// 单个服务卡片在拓扑看板中的高亮聚焦状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TopologyHighlightState {
    /// 无任何服务被悬停，卡片处于常态展示。
    Normal,
    /// 鼠标正悬停在当前卡片上，处于主色聚焦态。
    Focused,
    /// 当前卡片是悬停服务的「直接上游依赖」（供悬停服务调用）。
    Upstream,
    /// 当前卡片是悬停服务的「直接下游消费端」（调用悬停服务）。
    Downstream,
    /// 与当前悬停服务无关的其他服务，进行视觉淡化。
    Dimmed,
}

/// 解析 Compose YAML 中的 depends_on 映射。
///
/// 支持 Compose 规范中的两种主流语法：
/// 1. 数组形式：`depends_on: [ db, redis ]`
/// 2. 对象形式（含健康检查条件）：`depends_on: { db: { condition: service_healthy } }`
pub(crate) fn parse_compose_dependencies(yaml_text: &str) -> HashMap<String, Vec<String>> {
    let mut map = HashMap::new();
    let Ok(value) = serde_yaml_ng::from_str::<serde_yaml_ng::Value>(yaml_text) else {
        return map;
    };
    let Some(services) = value.get("services").and_then(|s| s.as_mapping()) else {
        return map;
    };

    for (svc_key, svc_val) in services {
        let Some(svc_name) = svc_key.as_str() else {
            continue;
        };
        let mut deps = Vec::new();
        if let Some(dep_val) = svc_val.get("depends_on") {
            if let Some(seq) = dep_val.as_sequence() {
                for item in seq {
                    if let Some(dep_name) = item.as_str() {
                        deps.push(dep_name.to_string());
                    }
                }
            } else if let Some(mapping) = dep_val.as_mapping() {
                for (dep_k, _) in mapping {
                    if let Some(dep_name) = dep_k.as_str() {
                        deps.push(dep_name.to_string());
                    }
                }
            }
        }
        deps.sort();
        deps.dedup();
        map.insert(svc_name.to_string(), deps);
    }
    map
}

/// 根据依赖表（服务 -> 上游依赖）构建反向依赖表（服务 -> 下游依赖者）。
pub(crate) fn build_dependents_map(
    deps: &HashMap<String, Vec<String>>,
) -> HashMap<String, Vec<String>> {
    let mut rev: HashMap<String, Vec<String>> = HashMap::new();
    for (consumer, upstreams) in deps {
        for upstream in upstreams {
            rev.entry(upstream.clone()).or_default().push(consumer.clone());
        }
    }
    for dependents in rev.values_mut() {
        dependents.sort();
        dependents.dedup();
    }
    rev
}

/// 根据依赖关系将所有已知服务划分为拓扑层级。
///
/// 层级规则：
/// - Tier 0（底层）：不依赖本项目的任何其他服务（如纯数据库、缓存组件）；
/// - Tier N：依赖第 N-1 层的服务；
/// - 若存在循环依赖或深层依赖，做最大深度截断保护。
pub(crate) fn calculate_topology_tiers(
    all_services: &[String],
    deps: &HashMap<String, Vec<String>>,
) -> Vec<Vec<String>> {
    if all_services.is_empty() {
        return Vec::new();
    }

    let service_set: HashSet<String> = all_services.iter().cloned().collect();
    let mut depth_map: HashMap<String, usize> = HashMap::new();

    fn compute_depth(
        svc: &str,
        deps: &HashMap<String, Vec<String>>,
        visited: &mut HashSet<String>,
        memo: &mut HashMap<String, usize>,
    ) -> usize {
        if let Some(&cached) = memo.get(svc) {
            return cached;
        }
        if !visited.insert(svc.to_string()) {
            return 0;
        }

        let mut max_parent_depth = 0;
        if let Some(parents) = deps.get(svc) {
            for parent in parents {
                let d = compute_depth(parent, deps, visited, memo);
                max_parent_depth = max_parent_depth.max(d + 1);
            }
        }

        visited.remove(svc);
        memo.insert(svc.to_string(), max_parent_depth);
        max_parent_depth
    }

    let mut memo = HashMap::new();
    let mut max_depth = 0;
    for svc in &service_set {
        let mut visited = HashSet::new();
        let d = compute_depth(svc, deps, &mut visited, &mut memo);
        depth_map.insert(svc.clone(), d);
        max_depth = max_depth.max(d);
    }

    let mut tiers: Vec<Vec<String>> = vec![Vec::new(); max_depth + 1];
    for svc in all_services {
        if let Some(&d) = depth_map.get(svc) {
            tiers[d].push(svc.clone());
        }
    }

    tiers.retain(|tier| !tier.is_empty());
    if tiers.is_empty() {
        vec![all_services.to_vec()]
    } else {
        tiers
    }
}

/// 计算单个服务在当前悬停状态下的高亮模式。
pub(crate) fn calculate_highlight_state(
    service_name: &str,
    hovered: Option<&str>,
    deps: &HashMap<String, Vec<String>>,
    rev: &HashMap<String, Vec<String>>,
) -> TopologyHighlightState {
    let Some(hovered) = hovered else {
        return TopologyHighlightState::Normal;
    };

    if service_name == hovered {
        return TopologyHighlightState::Focused;
    }

    if let Some(upstreams) = deps.get(hovered)
        && upstreams.iter().any(|u| u == service_name)
    {
        return TopologyHighlightState::Upstream;
    }

    if let Some(downstreams) = rev.get(hovered)
        && downstreams.iter().any(|d| d == service_name)
    {
        return TopologyHighlightState::Downstream;
    }

    TopologyHighlightState::Dimmed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_array_and_object_depends_on() {
        let yaml = r#"
services:
  web:
    image: nginx
    depends_on:
      - api
      - redis
  api:
    image: my-api
    depends_on:
      db:
        condition: service_healthy
  db:
    image: postgres
  redis:
    image: redis
"#;
        let deps = parse_compose_dependencies(yaml);
        assert_eq!(
            deps.get("web").unwrap(),
            &vec!["api".to_string(), "redis".to_string()]
        );
        assert_eq!(deps.get("api").unwrap(), &vec!["db".to_string()]);
        assert!(deps.get("db").unwrap().is_empty());
        assert!(deps.get("redis").unwrap().is_empty());
    }

    #[test]
    fn calculates_tiers_and_dependents() {
        let mut deps = HashMap::new();
        deps.insert("web".to_string(), vec!["api".to_string()]);
        deps.insert("api".to_string(), vec!["db".to_string()]);
        deps.insert("db".to_string(), Vec::new());

        let rev = build_dependents_map(&deps);
        assert_eq!(rev.get("db").unwrap(), &vec!["api".to_string()]);
        assert_eq!(rev.get("api").unwrap(), &vec!["web".to_string()]);

        let services = vec!["web".to_string(), "api".to_string(), "db".to_string()];
        let tiers = calculate_topology_tiers(&services, &deps);
        assert_eq!(tiers.len(), 3);
        assert_eq!(tiers[0], vec!["db".to_string()]);
        assert_eq!(tiers[1], vec!["api".to_string()]);
        assert_eq!(tiers[2], vec!["web".to_string()]);
    }

    #[test]
    fn highlight_states_distinguish_upstream_and_downstream() {
        let mut deps = HashMap::new();
        deps.insert("web".to_string(), vec!["api".to_string()]);
        deps.insert("api".to_string(), vec!["db".to_string()]);
        let rev = build_dependents_map(&deps);

        assert_eq!(
            calculate_highlight_state("api", Some("api"), &deps, &rev),
            TopologyHighlightState::Focused
        );
        assert_eq!(
            calculate_highlight_state("db", Some("api"), &deps, &rev),
            TopologyHighlightState::Upstream
        );
        assert_eq!(
            calculate_highlight_state("web", Some("api"), &deps, &rev),
            TopologyHighlightState::Downstream
        );
        assert_eq!(
            calculate_highlight_state("standalone", Some("api"), &deps, &rev),
            TopologyHighlightState::Dimmed
        );
    }
}
