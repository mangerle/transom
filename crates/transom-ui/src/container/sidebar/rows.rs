//! 引擎侧栏行构建：按分组与搜索词扁平化。

use super::*;

/// 侧栏图标按钮：与会话树的图标按钮同尺寸、同悬停口径。
pub(crate) fn engine_sidebar_button(
    id: &'static str,
    icon: IconName,
    tooltip: String,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    Button::new(id)
        .ghost()
        .xsmall()
        .icon(icon)
        .tooltip(tooltip)
        .on_click(on_click)
        .into_any_element()
}

/// 构建引擎分组树行：同级按目录数组顺序输出，搜索时保留命中条目的祖先。
pub(crate) fn build_engine_rows(catalog: &SessionCatalog, query: &str) -> Vec<ContainerSidebarRow> {
    let query = query.trim().to_ascii_lowercase();
    let mut child_folders: HashMap<Option<FolderId>, Vec<&EngineFolder>> = HashMap::new();
    for folder in &catalog.engine_folders {
        child_folders.entry(folder.parent).or_default().push(folder);
    }
    let mut child_engines: HashMap<Option<FolderId>, Vec<&ContainerEngineProfile>> = HashMap::new();
    for profile in &catalog.engines {
        child_engines
            .entry(profile.folder_id)
            .or_default()
            .push(profile);
    }

    let (engine_counts, folder_counts) = engine_subtree_counts(catalog);
    let folder_paths: HashMap<FolderId, String> = catalog
        .engine_folders
        .iter()
        .filter_map(|folder| {
            catalog
                .engine_folder_path(folder.id)
                .map(|path| (folder.id, path))
        })
        .collect();
    let matched_engines: HashSet<EngineProfileId> = catalog
        .engines
        .iter()
        .filter(|profile| {
            if query.is_empty() {
                return true;
            }
            let path = profile
                .folder_id
                .and_then(|id| folder_paths.get(&id))
                .map(String::as_str)
                .unwrap_or_default();
            let haystack = format!(
                "{} {} {}",
                profile.display_name().to_ascii_lowercase(),
                profile.engine.describe().to_ascii_lowercase(),
                path.to_ascii_lowercase()
            );
            haystack.contains(&query)
        })
        .map(|profile| profile.id)
        .collect();

    let mut visible_folders = HashSet::new();
    for folder in &catalog.engine_folders {
        let name_matches = !query.is_empty() && folder.name.to_ascii_lowercase().contains(&query);
        let has_engine = child_engines.get(&Some(folder.id)).is_some_and(|engines| {
            engines
                .iter()
                .any(|profile| matched_engines.contains(&profile.id))
        });
        if name_matches || has_engine {
            let mut current = Some(folder.id);
            while let Some(id) = current {
                if !visible_folders.insert(id) {
                    break;
                }
                current = catalog.engine_folder(id).and_then(|folder| folder.parent);
            }
        }
    }

    enum Pending {
        Folder(FolderId, usize, bool),
        EngineList(Option<FolderId>, usize, bool),
    }
    let mut rows = Vec::with_capacity(catalog.engines.len() + catalog.engine_folders.len());
    let mut stack = vec![Pending::EngineList(None, 0, false)];
    if let Some(roots) = child_folders.get(&None) {
        for folder in roots.iter().rev() {
            stack.push(Pending::Folder(folder.id, 0, false));
        }
    }
    while let Some(item) = stack.pop() {
        match item {
            Pending::EngineList(parent, depth, show_all) => {
                let Some(engines) = child_engines.get(&parent) else {
                    continue;
                };
                for profile in engines {
                    if query.is_empty() || show_all || matched_engines.contains(&profile.id) {
                        rows.push(ContainerSidebarRow::Engine(EngineRow {
                            profile: Box::new((*profile).clone()),
                            depth,
                        }));
                    }
                }
            }
            Pending::Folder(id, depth, ancestor_matched) => {
                let Some(folder) = catalog.engine_folder(id) else {
                    continue;
                };
                let name_matched =
                    !query.is_empty() && folder.name.to_ascii_lowercase().contains(&query);
                let show_all = ancestor_matched || name_matched;
                if !query.is_empty() && !show_all && !visible_folders.contains(&id) {
                    continue;
                }
                rows.push(ContainerSidebarRow::Folder(EngineFolderRow {
                    id,
                    name: folder.name.clone(),
                    depth,
                    collapsed: folder.collapsed && query.is_empty(),
                    engine_count: engine_counts.get(&id).copied().unwrap_or(0),
                    folder_count: folder_counts.get(&id).copied().unwrap_or(0),
                }));
                if query.is_empty() && folder.collapsed {
                    continue;
                }
                stack.push(Pending::EngineList(Some(id), depth + 1, show_all));
                if let Some(children) = child_folders.get(&Some(id)) {
                    for child in children.iter().rev() {
                        stack.push(Pending::Folder(child.id, depth + 1, show_all));
                    }
                }
            }
        }
    }
    rows
}

/// 统计每个引擎分组子树中的连接与子分组总数（沿父链累加，避免递归）。
pub(crate) fn engine_subtree_counts(
    catalog: &SessionCatalog,
) -> (HashMap<FolderId, usize>, HashMap<FolderId, usize>) {
    let mut direct_engines: HashMap<Option<FolderId>, usize> = HashMap::new();
    for profile in &catalog.engines {
        *direct_engines.entry(profile.folder_id).or_default() += 1;
    }
    let mut engine_counts: HashMap<FolderId, usize> = HashMap::new();
    let mut folder_counts: HashMap<FolderId, usize> = HashMap::new();
    for folder in &catalog.engine_folders {
        let direct = direct_engines.get(&Some(folder.id)).copied().unwrap_or(0);
        *engine_counts.entry(folder.id).or_default() += direct;
        let mut current = folder.parent;
        while let Some(parent) = current {
            *engine_counts.entry(parent).or_default() += direct;
            *folder_counts.entry(parent).or_default() += 1;
            current = catalog
                .engine_folder(parent)
                .and_then(|folder| folder.parent);
        }
    }
    (engine_counts, folder_counts)
}
