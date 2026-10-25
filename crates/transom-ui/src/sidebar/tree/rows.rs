//! 会话树的过滤与扁平化行构建。
//!
//! 同级条目严格按 `SessionCatalog` 中数组的原始顺序输出，不再按名称排序：
//! 数组顺序就是用户拖拽排定的显示顺序，一旦在这里排序，拖拽结果会被静默覆盖。

use super::{FolderRow, SessionRow, SidebarRow};
use std::collections::{HashMap, HashSet};
use transom_core::{Connection, FolderId, Session, SessionCatalog, SessionFolder, SessionId};

pub(crate) fn build_rows(catalog: &SessionCatalog, query: &str) -> Vec<SidebarRow> {
    let query = query.trim().to_ascii_lowercase();
    let mut child_folders: HashMap<Option<FolderId>, Vec<&SessionFolder>> = HashMap::new();
    for folder in &catalog.folders {
        child_folders.entry(folder.parent).or_default().push(folder);
    }
    let mut child_sessions: HashMap<Option<FolderId>, Vec<&Session>> = HashMap::new();
    for session in &catalog.sessions {
        child_sessions
            .entry(session.folder_id)
            .or_default()
            .push(session);
    }

    let (session_counts, folder_counts) = subtree_counts(catalog);
    let folder_paths: HashMap<FolderId, String> = catalog
        .folders
        .iter()
        .filter_map(|folder| catalog.folder_path(folder.id).map(|path| (folder.id, path)))
        .collect();

    let matched_sessions: HashSet<SessionId> = catalog
        .sessions
        .iter()
        .filter(|session| {
            query.is_empty()
                || session_matches(
                    session,
                    &query,
                    session
                        .folder_id
                        .and_then(|id| folder_paths.get(&id))
                        .map(String::as_str)
                        .unwrap_or_default(),
                )
        })
        .map(|session| session.id)
        .collect();

    // 名称命中的文件夹会展示整个子树；其余文件夹仅在包含命中会话时显示。
    let mut visible_folders = HashSet::new();
    for folder in &catalog.folders {
        let name_matches = !query.is_empty() && folder.name.to_ascii_lowercase().contains(&query);
        let has_session = child_sessions
            .get(&Some(folder.id))
            .is_some_and(|sessions| {
                sessions
                    .iter()
                    .any(|session| matched_sessions.contains(&session.id))
            });
        if name_matches || has_session {
            let mut current = Some(folder.id);
            while let Some(id) = current {
                if !visible_folders.insert(id) {
                    break;
                }
                current = catalog.folder(id).and_then(|folder| folder.parent);
            }
        }
    }

    enum Pending {
        Folder(FolderId, usize, bool),
        SessionList(Option<FolderId>, usize, bool),
    }

    let mut rows = Vec::with_capacity(catalog.sessions.len() + catalog.folders.len());
    let mut stack = vec![Pending::SessionList(None, 0, false)];
    if let Some(roots) = child_folders.get(&None) {
        for folder in roots.iter().rev() {
            stack.push(Pending::Folder(folder.id, 0, false));
        }
    }

    while let Some(item) = stack.pop() {
        match item {
            Pending::SessionList(parent, depth, show_all) => {
                let Some(sessions) = child_sessions.get(&parent) else {
                    continue;
                };
                for session in sessions {
                    if query.is_empty() || show_all || matched_sessions.contains(&session.id) {
                        rows.push(SidebarRow::Session(SessionRow {
                            session: Box::new((*session).clone()),
                            depth,
                        }));
                    }
                }
            }
            Pending::Folder(id, depth, ancestor_matched) => {
                let Some(folder) = catalog.folder(id) else {
                    continue;
                };
                let name_matched =
                    !query.is_empty() && folder.name.to_ascii_lowercase().contains(&query);
                let show_all = ancestor_matched || name_matched;
                if !query.is_empty() && !show_all && !visible_folders.contains(&id) {
                    continue;
                }
                rows.push(SidebarRow::Folder(FolderRow {
                    id,
                    name: folder.name.clone(),
                    depth,
                    collapsed: folder.collapsed && query.is_empty(),
                    session_count: session_counts.get(&id).copied().unwrap_or(0),
                    folder_count: folder_counts.get(&id).copied().unwrap_or(0),
                }));
                if query.is_empty() && folder.collapsed {
                    continue;
                }
                stack.push(Pending::SessionList(Some(id), depth + 1, show_all));
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

/// 统计每个文件夹子树中的会话与子文件夹总数。
///
/// 通过沿父链向上累加实现，避免递归；`catalog.normalize()` 已保证无环。
fn subtree_counts(
    catalog: &SessionCatalog,
) -> (HashMap<FolderId, usize>, HashMap<FolderId, usize>) {
    let mut direct_sessions: HashMap<Option<FolderId>, usize> = HashMap::new();
    for session in &catalog.sessions {
        *direct_sessions.entry(session.folder_id).or_default() += 1;
    }
    let mut session_counts: HashMap<FolderId, usize> = HashMap::new();
    let mut folder_counts: HashMap<FolderId, usize> = HashMap::new();
    for folder in &catalog.folders {
        let direct = direct_sessions.get(&Some(folder.id)).copied().unwrap_or(0);
        *session_counts.entry(folder.id).or_default() += direct;
        let mut current = folder.parent;
        while let Some(parent) = current {
            *session_counts.entry(parent).or_default() += direct;
            *folder_counts.entry(parent).or_default() += 1;
            current = catalog.folder(parent).and_then(|folder| folder.parent);
        }
    }
    (session_counts, folder_counts)
}

fn session_matches(session: &Session, query: &str, folder_path: &str) -> bool {
    let endpoint = match &session.connection {
        Connection::Ssh(ssh) => format!("{} {} {}", ssh.username, ssh.host, ssh.port),
        Connection::Telnet(telnet) => format!("telnet {} {}", telnet.host, telnet.port),
        Connection::Local(local) => {
            format!("local {}", local.shell.as_deref().unwrap_or("sh"))
        }
        Connection::Serial(serial) => {
            format!("serial {} {} {}", serial.port, serial.baud_rate, "com tty")
        }
        _ => String::new(),
    };
    let haystack = format!(
        "{} {} {}",
        session.name.to_ascii_lowercase(),
        folder_path.to_ascii_lowercase(),
        endpoint.to_ascii_lowercase()
    );
    haystack.contains(query)
}
