# UI 实体化交接清单（滚动更新）

本文件是跨会话交接入口：新会话先读本文件，再按顺序执行，不要重新讨论方案。

## 一、总目标

按"状态按生命周期归属 + 实现归属实体"重构 `transom-ui`：窗口级面板用
`Entity<面板>` 持有状态并实现 `Render`，窗格级面板每个窗格一份实体，外壳只保留
标签/窗格编排与跨域协调。禁止为 `AppShell` 实现 `Deref`；设置是 `Global`。

## 二、已完成（提交即事实）

| 提交 | 内容 |
| :--- | :--- |
| `d5cfc07` | 移除 `AppShell` 的 `Deref`/`DerefMut`，1887 处访问显式化 |
| `c9b55e7` | 监控抽屉实体化 `MonitorView` |
| `4728283` | 修复面板回调双重租借崩溃，加守卫 `monitor_with_shell_targets_do_not_reenter_panel` |
| `9eed0dd` | 编辑面板实体化 `EditorPanels`（状态 + 渲染 + 拖拽） |
| `6cdb7e9` | `AppSettings` 提为 `Global`（`app::settings_global`） |
| `dd9e50d` | 转发面板实体化 `ForwardView` |
| `6cf991a` | 记录 `cloud_sync` 不实体化的判定 |

`AppShell` 方法数：672 → 549。

## 三、待执行（按顺序）

### 1. `diff_viewer`（21 个方法）

- 新建 `diff_viewer/entity.rs`：`DiffViewerView { state: DiffViewerState, app: Option<WeakEntity<AppShell>> }`
- `AppState.diff_viewer: Option<DiffViewerState>` → `Option<Entity<DiffViewerView>>`
- 创建点两处（`diff_viewer/mod.rs` 约 233、292 行）：`cx.new` + 创建后 `attach`
- `recompute_diff`、`apply_prepared_diff` 整体搬进实体（内部只用自身状态、
  `spawn_blocking_local`、`cx.new`，自洽）
- `merge.rs` 四个方法留在外壳，读写实体用 `handle.update(cx, |v, _| ...)`
- 渲染留在 `dialog_host`，读 `handle.read(cx).state`
- 注意：`compute.rs`/`merge.rs` 是"连读多字段 + 就地改多字段"，必须整块进
  `update` 闭包，不能逐字段改写

### 2. `settings`（74 个方法）

- `SettingsView` 持 `SettingsModalState`，`modals.settings` 换成句柄
- 设置值继续读 `app_settings(cx)`（已是 `Global`），不穿透状态
- 表单输入为 `Entity<InputState>`，随实体创建

## 四、明确不做

| 模块 | 理由 |
| :--- | :--- |
| `cloud_sync` | 14 个方法全是跨域编排（命令、合并、安全副本、凭据），无自家 UI 状态 |
| `workspace`/`tabs`/`sidebar` | 标签与窗格生命周期本就是外壳职责 |

## 四之二、已知未修的系统性风险

外壳持有 `&mut AppShell` 时调用 `view.update(...)`，若实体方法内部再
`read_shell_parts` / `with_shell_parts` 读外壳，会 panic：
`cannot read AppShell while it is already being updated`。

已修：`request_sftp_list` / `request_sftp_list_pane`（连接号由外壳算好传入）。

**未修（按风险排序，需逐个改成"外壳算好参数 → 实体只用手上数据"）**：

1. `SftpPanelView::send_op_for_connection`（外壳 `send_sftp_op_for_connection` 调用）
2. `SftpPanelView::enqueue_uploads`、`process_queue`（外壳队列/拖拽入口调用）
3. 其余 `with_shell_parts` 副作用型调用：建议改为 `cx.defer` 延后到租借释放后执行

判据：实体方法体内出现 `read_shell_parts` / `with_shell_parts` 的，一律不能被
外壳在持有借用时直接调用。

## 四之三、协议修正（2026-11-01 决定）

跨实体调用只允许单向：

- **父 → 子**：父把数据算好传下去（连接号、快照、参数），子只改自身状态并返回结果，
  跨域动作（发命令、开弹窗、切标签）由父在 `update` 返回之后完成。
- **子 → 父**：只在实体自己的回调上下文里调用；或 `cx.emit` 事件 + 父 `cx.subscribe`。

反模式：子实体方法体内出现 `with_shell_parts` / `read_shell_parts`，而该方法会被
父在持有借用时 `view.update` 调用——必然 panic。

待清理（`with_shell_parts` 21 处、`read_shell_parts` 28 处，集中在 SFTP）：

1. `SftpPanelView::send_op_for_connection`（父：`send_sftp_op_for_connection`）
2. `SftpPanelView::enqueue_uploads`、`process_queue`（父：队列与拖拽入口）
3. `sftp/entity/{chmod,delete,queue,task_control,local_diff}.rs` 内其余回交点
4. `sftp/remote_actions.rs`（7 处）

修法二选一：把父需要的数据作为参数传入（首选，无时序变化）；或把回交改成
`cx.defer` + 捕获 owned 数据（注意闭包需 `'static`，不能带 `&mut Window`）。

已按新协议改好的样板：`request_sftp_list` / `request_sftp_list_pane` →
`repo: prepare_remote_list` 返回连接号，父负责 `send_command`。

## 五、硬约束（违反会 panic 或回归）

1. 实体回调回交外壳后，外壳方法不得再 `read`/`update` 同一实体（双重租借 panic）。
2. 状态单份：切换期间不保留影子副本。
3. 单文件 500 行；同一实体的 `impl` 可按关注点拆多文件。
4. 消费外部文件先检查体积与事件上限。

## 六、验证

每个提交前：`cargo fmt --all`、`cargo clippy --workspace --all-targets -- -D warnings`、
`cargo check --workspace --all-targets`、`cargo test --workspace`。
