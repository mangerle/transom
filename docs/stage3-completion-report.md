# Transom 阶段 3 收尾报告

- 完成日期：2026-11-09
- 范围：容器侧栏实体化、容器弹窗渲染与动作下沉、标签骨架、`with_container` 迁移桥删除、源码守卫与文档
- 结论：阶段 3 的 A-D 全部完成，`container/` 下不再有渲染方法挂在 `impl AppShell`，迁移桥已删除，全工作区四道门禁通过

## 一、完成项

### A. 容器侧栏实体化

- 新增单例实体 `Entity<ContainerSidebarView>`（`container/sidebar/entity.rs`），承接：
  - 搜索框与输入订阅
  - 当前选中引擎
  - 分组新建/重命名表单
  - 引擎发现面板与在途标记
- 侧栏渲染改为自由函数，目录（引擎、分组）与连接状态由外壳按渲染快照注入；
  侧栏文件不再出现 `impl AppShell` / `Context<AppShell>`。
- 跨域动作经 `ContainerSidebarEvent` 上交，`container/sidebar_events.rs` 统一处理：
  打开工作区/连接表单、删除引擎、折叠/改名/删除分组、拖放与目录持久化、
  引擎发现、SSH 会话联动与一次性提示。
- 删除 `container/sidebar/actions.rs` 与侧栏全部外壳渲染方法。

### B. 容器弹窗实体化与渲染下沉

以下 14 个渲染入口全部改为自由函数，取消/确认/提交直接调用 `ContainerWorkspace`
方法；`dialog_host` 只按弹窗种类定位工作区并注入状态：

- `modals/confirm.rs`、`modals/rename.rs`（本轮之前已完成）
- `modals/tag.rs`、`modals/export.rs`、`modals/commit.rs`、`modals/update.rs`
- `modals/forward.rs`、`modals/network.rs`、`modals/files.rs`（重命名/新建目录）
- `modals/pods.rs`、`modals/quadlet.rs`
- `push_dialog.rs`、`system_df_dialog.rs`、`build_dialog/render.rs`、`hub/dialog.rs`

动作归属：

- tag/commit/update/network/export/files/pods/quadlet/push/system_df/build/hub 的
  打开、取消、提交与选项切换迁入 `impl ContainerWorkspace`；
- 端口转发提交改为 `StartPortForward` 事件，规则参数随事件携带，外壳写转发状态
  并投递命令；
- 弹窗校验失败统一经 `ShowToast` 事件交外壳，弹窗保持打开、输入保留。

### C. 标签骨架、连接态与全屏

- `render_container_tab_body`、`render_container_connection_state`、
  `render_container_list_content` 改为自由函数；连接态重试直接调用
  `ContainerWorkspace::reconnect`。
- 外壳只保留最小窗口级入口（`workspace/panes.rs`）：
  - 列表焦点句柄、`ContainerList` 键位上下文与按键监听；
  - `notice` 在渲染帧后转通知；
  - 全屏覆盖层与 Esc 语义。
- `list/keyboard.rs` 与 `handle_container_list_key` 保留为键盘事件外壳包装，
  行集合在事件到来时按当前状态重算，渲染函数无副作用。

### D. 删除 `with_container` 迁移桥

- 删除 `container/mod.rs` 的 `with_container` / `with_container_mut`；
  全仓库调用点由 131 处降至 0。
- 请求反查（`take_container_request` / `container_request_tab` /
  `container_request_label`）从 `container/mod.rs` 的 `impl AppShell` 迁到
  `container/requests.rs` 的 `impl AppState`，`container/mod.rs` 不再暴露
  迁移期访问器。
- 事件处理改为「按 `request_id` 定位标签页 → `workspace.update`」。
- 列表、动作结果、日志/统计、文件、下载任务等状态流转收敛到
  `container/events/apply.rs` 的实体具名方法。
- 源码守卫 `container_migration_bridge_is_removed` 防止迁移桥复活。

### E. 守卫、文档与门禁

新增源码守卫（`crates/transom-ui/tests/source_guards.rs`）：

- `container_has_no_shell_render_methods`：容器渲染不得挂回 `impl AppShell`；
- `container_sidebar_is_owned_by_entity`：侧栏状态归实体，侧栏文件不得回挂外壳；
- `container_migration_bridge_is_removed`：`with_container*` 不得复活，
  请求反查不得回到 `container/mod.rs`；
- `container_entities_have_no_parent_handle_or_sync_callback`：容器实体不得持有
  `WeakEntity<AppShell>` 或 `with_shell`/`read_shell` 同步通道；
- `migrated_container_render_slices_do_not_return_to_shell` 扩充全部已下沉文件。

`docs/architecture.md` 已同步容器侧最终形态与保留边界。

全工作区门禁（最终 HEAD）：

- `cargo fmt --all --check`
- `cargo check --workspace --all-targets --offline`
- `cargo clippy --workspace --all-targets --offline -- -D warnings`
- `cargo test --workspace --offline`

四项均通过，零警告。

## 二、保留的外壳入口与原因

| 入口 | 原因 |
|---|---|
| `workspace/panes.rs::render_container_tab` | 需要 `Window`/`Context` 做列表焦点、键位上下文、按键监听与通知转弹；正文全部是自由函数 |
| `workspace/panes.rs::render_container_fullscreen` | 全屏遮罩与 Esc 键语义属于窗口级覆盖层 |
| `workspace/panes.rs::flush_container_notice` | 需要 `Window` 延迟到渲染帧后弹通知 |
| `container/list.rs::handle_container_list_key` | 键盘事件外壳包装，按当前搜索/筛选重算行集合 |
| `container/port_forward.rs::open_container_port_forward` | 需要枚举全部标签页的活跃 SSH 连接并创建输入控件 |
| `container/engines.rs` | 打开/聚焦工作区、按 SSH 会话建工作区、保存/删除引擎配置，属于跨标签窗口编排 |
| `container/sidebar_events.rs` | 侧栏事件对应的目录写入、持久化、拖放与提示 |
| `container/events/*`、`container/event.rs` | 事件路由、`request_id` 反查标签页与窗口请求排队 |
| `dialog_host/*` | 模态栈层序、遮罩、焦点与取消/提交语义 |
| `container/actions/*` 等 | 文件选择器、命令编排与跨域动作入口 |

## 三、已知风险与取舍

1. **一帧延迟**：侧栏事件与容器窗口请求都在渲染期统一执行，表单提交、目录写入
   与打开弹窗可能晚一帧生效；与既有 `WindowRequests` 口径一致。
2. **提示路径差异**：镜像中心入队下载改用实体命令通道；队列满时只记录 `warn`
   日志，不再写入应用级错误队列。正常路径与提示文案不变。
3. **通知依赖 GPUI 访问追踪**：侧栏与工作区实体在渲染期被读取后由 GPUI 记录，
   实体的 `notify` 才能驱动整壳重绘；若未来把实体移出渲染读取链，需要显式观察器。
4. **验证范围**：门禁覆盖编译、静态分析与单元/集成测试；未做真实窗口的手工交互
   回归（无图形会话）。
5. **并行提交**：工作期间同一仓库有另一会话的 Windows/连接池修复提交
   （`b93ba250`、`5bc7781f`、`78917eb1`、`0e9c7299`）交错进入历史，与本任务范围
   无关；最终门禁在包含这些提交的 HEAD 上重跑通过。

## 四、提交范围

本任务提交（按时间顺序）：

- `72a4f195` refactor(container): 弹窗与标签骨架渲染下沉实体
- `0b614147` refactor(container): 容器侧栏迁入独立实体
- `f04aa83e` refactor(container): 删除 with_container 迁移桥并收敛事件写回
- `5a9e6e57` test(container): 补齐实体化守卫并同步架构文档
- `1e50c7d2` docs(container): 补充阶段 3 收尾报告
- `5d49316f` refactor(container): 请求反查迁入 AppState 并补齐实体守卫

## 五、完成定义核对

1. 阶段 3 剩余 A-D 全部完成，`container/` 下无渲染方法挂在 `impl AppShell`：是。
2. `with_container*` 迁移桥删除，事件与动作落在实体或明确的窗口级入口：是。
3. 全工作区 fmt/check/clippy/test 通过，零警告：是。
4. `docs/architecture.md` 与源码守卫反映最终归属和保留边界：是。
5. 报告列出提交、保留入口、风险：是。
