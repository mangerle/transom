# 标签模型清理实施基线

本文记录标签模型清理的目标、边界与验收口径。该清理只处理标签状态归属，
不包含 API 客户端功能迁移。

## 目标

- 将 `TerminalTab` 改造为通用 `WorkspaceTab`，公共层只保留标签栏共用的元数据。
- 会话专属的 `Session`、`SessionId`、`ConnectionId`、`cwd` 与终端窗格状态
  归入 `SessionTab`。
- Git、容器、播放器、监控使用各自的内容类型，不再通过占位 `Session` 复用
  会话标签。
- 标签组使用 `TabGroupOwner`，不再要求所有标签都拥有 `SessionId`。
- 会话事件、SFTP、转发、广播、审计与监控只作用于真正的会话标签。
- 为后续 API 工作区标签保留 `TabContent` 类型扩展点，但本阶段不实现 API UI。

## 类型边界

```text
WorkspaceTab
  ├─ 公共标签 ID、标题、展示状态
  └─ TabContent
       ├─ Session(Box<SessionTab>)
       ├─ Player(PlayerTab)
       ├─ Monitor(MonitorTab)
       ├─ Containers(ContainerTab)
       └─ Git(GitTab)
```

`TabStatus` 保留为标签栏展示状态，不作为判断标签是否为会话标签的依据。

## 标签组归属

- 会话标签：`TabGroupOwner::Session(session_id)`。
- 监控标签：`TabGroupOwner::Session(target_session_id)`，保持现有标签序号语义。
- 播放器、容器、Git：`TabGroupOwner::Workspace(tab_id)`。
- 后续 API 标签：`TabGroupOwner::Workspace(tab_id)`。

## 行为兼容

必须保持不变：

- 本地终端和 SSH 连接、断开、重连。
- 终端分屏、最大化、脱离窗格和关闭焦点回退。
- 标签拖拽、横向和纵向标签、并排网格。
- SFTP、转发、广播、审计与监控的会话目标选择。
- Git 和容器标签的打开、聚焦、关闭。
- 启动时恢复终端标签与分割布局。

有意修正：

- 非会话标签不再出现在会话专属右键菜单中。
- 容器标签关闭时发送容器断开命令。
- 监控不再把 Git、容器等非会话标签当作采样目标。
- 退出时不再保存非会话标签的占位会话键。

## 持久化

`settings.json` 结构不变。`previous_tab_sessions`、`previous_tab_groups` 和
`tab_tilings` 的序列化字段保持兼容。清理后只保存真正的会话标签；非会话标签
跨重启恢复不在本阶段实现。

## 验收

- 公共 `WorkspaceTab` 不再包含会话专属字段。
- Git、播放器和容器不再构造占位会话。
- 所有连接相关处理只通过会话标签访问器取数据。
- 现有标签行为测试、窗口恢复测试和架构守卫全部通过。
- `AppShell` 仍然只有一个 `state` 字段。
