//! 容器列表键盘导航支持：j/k 上下移动、Enter 打开、l 日志、s 停止/启动、/ 聚焦搜索。
//!
//! 设计原理（Why）：
//! - 终端与运维用户习惯在列表界面全程脱离鼠标完成定位与生命周期干涉；
//! - 提取独立的纯算法函数计算导航步进（上下越界保护、首尾停顿），与 UI 调度解耦并具备确定性单测；
//! - 按键动作严格分流：输入框未失焦时不拦截普通字符，确保输入与全局快捷键互不干扰。

use super::{ListRow, list_rows, row_key};
use crate::container::ContainerDetailTab;
use crate::container::entity::ContainerWorkspace;
use crate::container::types::RowTarget;
use gpui_kit::{Context, KeyDownEvent, Window};
use transom_core::{ComposeAction, ContainerLifecycleAction};

/// 根据步进偏移量计算下一项选中目标。
///
/// - `rows` 为空时返回 `None`；
/// - 当前未选中任何行时，向上或向下移动均默认选中首项（索引 0）；
/// - 到达边界时饱和停靠在首尾，不进行不可预测的跳跃。
pub(crate) fn navigate_selection(
    rows: &[ListRow],
    current_selected_key: Option<&str>,
    delta: isize,
) -> Option<RowTarget> {
    if rows.is_empty() {
        return None;
    }

    let current_index =
        current_selected_key.and_then(|key| rows.iter().position(|r| row_key(&r.target) == key));

    let next_index = match current_index {
        Some(idx) => {
            let target_idx = idx as isize + delta;
            target_idx.clamp(0, (rows.len() - 1) as isize) as usize
        }
        None => 0,
    };

    rows.get(next_index).map(|r| r.target.clone())
}

/// 查找当前被选中的行数据引用。
pub(crate) fn find_selected_row<'a>(
    rows: &'a [ListRow],
    current_selected_key: Option<&str>,
) -> Option<&'a ListRow> {
    let key = current_selected_key?;
    rows.iter().find(|r| row_key(&r.target) == key)
}

impl ContainerWorkspace {
    /// 键盘事件到来时按当前状态重算行集合并处理列表快捷键。
    pub(crate) fn handle_list_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 存在控制修饰符（Ctrl/Alt/Meta）时放行，交由全局快捷键系统处理
        if event.keystroke.modifiers.control
            || event.keystroke.modifiers.alt
            || event.keystroke.modifiers.platform
        {
            return;
        }
        let query = self.state.query(cx);
        let rows = list_rows(&self.state, &query);
        let current_selected = self.state.selected.clone();

        match event.keystroke.key.as_str() {
            // 下移一行
            "j" | "ArrowDown" | "down" => {
                if let Some(target) = navigate_selection(&rows, current_selected.as_deref(), 1) {
                    self.select_row(&target, cx);
                }
            }
            // 上移一行
            "k" | "ArrowUp" | "up" => {
                if let Some(target) = navigate_selection(&rows, current_selected.as_deref(), -1) {
                    self.select_row(&target, cx);
                }
            }
            // 回车：打开/激活当前条目
            "Enter" | "enter" => {
                if let Some(row) = find_selected_row(&rows, current_selected.as_deref()) {
                    match &row.target {
                        RowTarget::Container { .. } => {
                            // 若已选中容器，打开概览页
                            self.set_detail_tab(ContainerDetailTab::Overview, cx);
                        }
                        _ => {
                            self.select_row(&row.target, cx);
                        }
                    }
                } else if let Some(first) = rows.first() {
                    self.select_row(&first.target, cx);
                }
            }
            // 'l'：快速切换至日志页签
            "l" => {
                if let Some(row) = find_selected_row(&rows, current_selected.as_deref())
                    && matches!(row.target, RowTarget::Container { .. })
                {
                    self.set_detail_tab(ContainerDetailTab::Logs, cx);
                }
            }
            // 's'：启停当前选中的容器或 Compose 项目
            "s" => {
                if let Some(row) = find_selected_row(&rows, current_selected.as_deref()) {
                    match &row.target {
                        RowTarget::Container { name, .. } => {
                            let is_running = row.running == Some(true);
                            let action = if is_running {
                                ContainerLifecycleAction::Stop
                            } else {
                                ContainerLifecycleAction::Start
                            };
                            self.container_lifecycle(name.clone(), action, cx);
                        }
                        RowTarget::Compose(name) => {
                            let is_running = row.running == Some(true);
                            let action = if is_running {
                                ComposeAction::Down
                            } else {
                                ComposeAction::Up
                            };
                            self.run_compose(name.clone(), action, cx);
                        }
                        _ => {}
                    }
                }
            }
            // '/'：聚焦搜索框
            "/" => {
                let search = self.state.search.clone();
                search.update(cx, |state, cx| state.focus(window, cx));
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::types::SortFields;

    fn mock_row(id: &str, name: &str, running: bool) -> ListRow {
        ListRow {
            key: format!("c-{id}"),
            title: name.to_string(),
            subtitle: Some("nginx:alpine".to_string()),
            badge: None,
            running: Some(running),
            target: RowTarget::Container {
                id: id.to_string(),
                name: name.to_string(),
            },
            fields: SortFields::new(
                name.to_string(),
                if running {
                    "running".to_string()
                } else {
                    "exited".to_string()
                },
                100,
                200,
            ),
        }
    }

    #[test]
    fn test_navigate_selection_empty() {
        assert!(navigate_selection(&[], None, 1).is_none());
        assert!(navigate_selection(&[], Some("c1"), 1).is_none());
    }

    #[test]
    fn test_navigate_selection_initial() {
        let rows = vec![
            mock_row("c1", "web-1", true),
            mock_row("c2", "web-2", false),
        ];
        // 无初始选中项时，步进默认选中首项
        let next = navigate_selection(&rows, None, 1);
        assert_eq!(
            next,
            Some(RowTarget::Container {
                id: "c1".to_string(),
                name: "web-1".to_string()
            })
        );
    }

    #[test]
    fn test_navigate_selection_down_and_up() {
        let rows = vec![
            mock_row("c1", "web-1", true),
            mock_row("c2", "web-2", false),
            mock_row("c3", "web-3", true),
        ];

        // 从 c1 向下移动到 c2
        let next = navigate_selection(&rows, Some("c1"), 1);
        assert_eq!(
            next,
            Some(RowTarget::Container {
                id: "c2".to_string(),
                name: "web-2".to_string()
            })
        );

        // 从 c2 向上移动到 c1
        let prev = navigate_selection(&rows, Some("c2"), -1);
        assert_eq!(
            prev,
            Some(RowTarget::Container {
                id: "c1".to_string(),
                name: "web-1".to_string()
            })
        );

        // 到达末尾 c3 继续向下停在 c3
        let end = navigate_selection(&rows, Some("c3"), 1);
        assert_eq!(
            end,
            Some(RowTarget::Container {
                id: "c3".to_string(),
                name: "web-3".to_string()
            })
        );

        // 到达顶部 c1 继续向上停在 c1
        let top = navigate_selection(&rows, Some("c1"), -1);
        assert_eq!(
            top,
            Some(RowTarget::Container {
                id: "c1".to_string(),
                name: "web-1".to_string()
            })
        );
    }

    #[test]
    fn test_find_selected_row() {
        let rows = vec![
            mock_row("c1", "web-1", true),
            mock_row("c2", "web-2", false),
        ];

        let found = find_selected_row(&rows, Some("c2"));
        assert!(found.is_some());
        assert_eq!(found.unwrap().title, "web-2");

        let not_found = find_selected_row(&rows, Some("c999"));
        assert!(not_found.is_none());

        let none = find_selected_row(&rows, None);
        assert!(none.is_none());
    }
}
