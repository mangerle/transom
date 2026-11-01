# 面板实体通信协议（架构设计）

## 一、问题定义

GPUI 的实体是**单租借**的：同一实体在同一时刻只能被 `read`/`update` 一次。
我们当前的实现存在**运行期调用环**：

```
AppShell.listener()            ← 外壳已持有 &mut AppShell
  → view.update(|view| ...)    ← 同时租借面板
      → with_shell_parts()     ← 面板回头 update 外壳 → panic
```

这不是某几处写错了，而是"子实体持有父实体引用并同步回调"这个结构本身允许成环。
只要结构不变，清理 49 处之后仍会新增。

## 二、方案对比

| 方案 | 做法 | 结论 |
| :--- | :--- | :--- |
| A. `cx.defer` 兜底 | 回交改成延后执行 | **不采用**：要求闭包 `'static`，现有调用点大量捕获 `&mut Window`/`&str`；时序从"同步"变"延后"，语义漂移且难以推理 |
| B. 面板退化为纯视图 | 状态全回外壳，面板只渲染 | **不采用**：等于回退实体化，丢掉按面板重绘与归属划分 |
| C. 去父化 + 事件上行 | 子实体**不持有父引用**；数据向下推、意图向上发 | **采用** |

## 三、目标架构（方案 C）

三条规则，缺一不可：

1. **子实体不持有父实体引用**。`SftpPanelView` 删除 `app: Option<WeakEntity<AppShell>>`，
   `with_shell_parts` / `read_shell_parts` 两个通道整体删除。
2. **数据只向下推**。父在数据变化时主动 `view.update(|v, cx| v.set_*(...))` 注入
   （连接号、会话 `forwards`、外部宽度等）。子渲染只读自己的状态与 `Global`
   （设置已是 `Global`，不需要外壳）。
3. **意图只向上发事件**。子需要父做事时 `cx.emit(SftpPanelEvent::...)`，
   父在创建面板时 `cx.subscribe(&view, handler)` 订阅一次。

```rust
pub(crate) enum SftpPanelEvent {
    /// 请求投递一条 SFTP 命令；命令在事件里构造好，父不再回读面板。
    SendCommand(SftpCommand),
    /// 请求打开某个弹窗（chmod/属性/过滤/输入），参数随事件携带。
    OpenModal(SftpModalRequest),
    /// 请求关闭本面板（父负责回收窗格里的句柄）。
    CloseRequested,
    /// 请求调整面板宽度。
    RequestWidth(f32),
}
```

事件回调里父**只使用事件携带的数据**，绝不回读面板——这条是安全的关键：
GPUI 的事件派发发生在当前 `update` 之后，父在回调里租借自己不会与子冲突。

## 四、为什么这样能根治

- 子实体没有父句柄 → 编译期就不可能同步回调父；
- 父→子只走 `update` 且子方法内不碰父 → 单向；
- 父的事件回调只用事件数据 → 不构成回头租借。

三者合起来，`cannot read/update ... while it is already being updated` 在结构上不可达，
而不是靠逐个调用点自觉。

## 五、迁移步骤（以 SFTP 为试点）

1. 定义 `SftpPanelEvent` 与 `SftpModalRequest`，实现 `EventEmitter<SftpPanelEvent>`。
2. 创建面板处（`build_sftp_panel` / 本地单栏）注册 `cx.subscribe`，处理器只做
   "发命令 / 开弹窗 / 回收句柄"三件事。
3. 逐个把 49 处 `with_shell_parts` / `read_shell_parts` 换成：
   - 发命令、开弹窗 → `cx.emit(...)`
   - 需要外壳数据 → 改由父在数据变化时 `set_*` 注入
4. 删除 `SftpPanelView.app`、`with_shell_parts`、`read_shell_parts`。
5. 守卫测试：
   - 面板实体结构体不得含 `WeakEntity<AppShell>` 字段；
   - `sftp/` 下不得再出现 `with_shell_parts` / `read_shell_parts`；
   - 父的订阅回调里不得出现 `sftp_view(` / `view.update(`。
6. 其余面板（editor/monitor/forward）按同形状收敛，最后统一删掉通用通道。

## 六、代价

- 需要给面板补"数据注入"入口（连接号、会话数据）并在外壳数据变化时调用；
- 事件需要携带足够数据，事件定义会比现在的方法调用啰嗦一些；
- 一次性改动量约等于 SFTP 迁移的一半，但会**永久消除**该类崩溃。
