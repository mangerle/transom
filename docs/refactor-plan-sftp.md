# SFTP 面板实体化执行清单

状态：待执行（架构选型已定：方案 A，每标签/每窗格一个实体）。

## 一、为什么是每窗格实体

SFTP 状态挂在窗格上（`SessionPane.sftp: Option<SftpPanelState>`），随连接建立、断开、
标签关闭而生死。窗口级单实体（把 N 份状态塞进一个对象）会把"某个标签的传输进度变化"
放大成整个实体重绘，等于回到整壳重绘，不符合"状态按生命周期归属"。

因此：`Entity<SftpPanelView>` 每个持有 SFTP 的窗格一份，由窗格强持有，实体用
`WeakEntity<AppShell>` 回指外壳。

## 二、实测规模

| 指标 | 数量 |
| :--- | ---: |
| 待迁移的 `impl AppShell` 方法 | 158 |
| 含 `impl AppShell` 的文件 | 44 |
| 模块代码行数 | 7,660 |
| 状态访问点（模块内 / 模块外） | 105 / 11 |
| 状态创建与销毁点 | 6 |

## 三、分步执行

### 第 1 步：骨架与生命周期

1. 新建 `sftp/entity.rs`：`SftpPanelView { state: SftpPanelState, app: Option<WeakEntity<AppShell>> }`，
   提供 `new` / `attach` / 读写访问器。
2. `SessionPane.sftp` 改为 `sftp_view: Option<Entity<SftpPanelView>>`；
   `TerminalTab::sftp()` / `sftp_mut()` 改为返回句柄的 `sftp_view()`，状态读取经
   `self.sftp_state(tab_id, cx)` 辅助方法。
3. 六个创建/销毁点改造：
   - `sftp/local_only.rs:90`、`sftp/nav_actions/init.rs:199`：`cx.new(|cx| SftpPanelView::new(...))`
   - `connection/mod.rs:204`、`sftp/panel_view.rs:427`、`sftp/nav_actions/init.rs:75`、
     `sftp/local_pane_view/panel_header.rs:36`：置 `None` 即销毁实体
4. 生命周期收尾：标签关闭、连接断开、`prune_tab_overlays` 时确保实体随之释放。

### 第 2 步：渲染归属

1. `sftp/panel_view.rs::render_sftp_panel` 改为 `impl Render for SftpPanelView`。
2. `app_render.rs` 里改为挂实体（`Entity<SftpPanelView>` 实现 `IntoElement`）。
3. 面板内按钮回调改为实体方法；跨域动作（标签操作、命令投递）走
   `with_shell(cx, |shell, cx| ...)`。

### 第 3 步：导航与文件操作

把 `nav_actions/`、`remote_nav.rs`、`file_ops/`、`selection.rs`、`row_interaction.rs`
里的方法搬进 `impl SftpPanelView`。只读写面板状态的直接搬；需要发 `SftpCommand` 的，
按"实体算参数，外壳发命令"两段式拆。

### 第 4 步：队列、预览与模态

`queue_actions/`、`queue_view/`、`preview_actions.rs`、`preview_view.rs`、`modals/`、
`search_modal.rs` 同样处理。模态弹窗本体仍留在 `dialog_host`，实体只持有打开状态。

## 四、硬约束（违反会 panic 或回归）

1. **实体回调禁止重入自身**：点击回调里回交外壳后，外壳方法不得再 `read`/`update`
   同一个 `SftpPanelView`。状态变更留在实体，外壳只收结果（快照、布尔、参数）。
2. **状态单份**：切换期间不允许 `tab.sftp` 与实体同时存在状态，必须先搬后删，
   不保留影子副本。
3. **单文件 500 行**：44 个文件的 `impl` 合并进实体后按关注点分文件
   （`entity/render.rs`、`entity/nav.rs`、`entity/queue.rs`、`entity/modals.rs`）。
4. **不静默降级**：搬错的 `Option` 不得用 `unwrap_or_default()` 兜掉，保持原有错误路径。

## 五、验证

每个提交前：`cargo fmt --all`、`cargo clippy --workspace --all-targets -- -D warnings`、
`cargo check --workspace --all-targets`、`cargo test --workspace`。

第 2 步完成后需手测：打开 SFTP、切换远端目录、上传/下载、预览、队列暂停/重试、
本地/远端双栏拖拽、断线重连、关闭标签后重开。
