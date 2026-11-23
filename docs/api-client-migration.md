# API 客户端迁移

本文记录 CyberAPI 桌面核心能力迁入 Transom 的边界、结构与验收结果。本阶段不增加账号、团队、邀请、服务端同步或云端项目，不复用 CyberAPI 的 crate。

## 产品入口

API 客户端沿用 Git、容器和会话目录的导航模型：

```text
左侧活动栏 ApiClient
  └─ API 项目侧栏
       ├─ 搜索项目
       ├─ 新建、重命名、删除项目
       ├─ 导入本地 JSON 项目
       └─ 双击或点击项目
            └─ 打开或聚焦独立 API 工作区标签
```

终端、Git、容器和 API 标签可以同时存在。API 项目不会切换整个主界面，也不会占用终端标签的会话字段。

## 类型与状态边界

- `ApiSidebarView`：窗口单例实体，保存项目登记表、搜索词、内联编辑器和删除确认。
- `ApiWorkspace`：每个项目标签独立实体，保存接口树选择、当前请求草稿、响应和发送任务。
- 项目数据属于 `transom-api`，窗口级项目列表属于侧栏实体，请求编辑与响应属于工作区实体。
- `ApiClientTab`：`TabContent` 的类型化内容，只持有 `ApiProjectId` 与 `Entity<ApiWorkspace>`。
- 关闭 API 标签会释放工作区实体；正在发送的 Future 随实体任务释放而被取消。
- API 侧栏和工作区只通过事件把完整意图交给外壳，外壳不回读派发事件的实体。

## 数据与存储

`transom-api` 定义项目、目录、请求、请求头和 HTTP 方法模型，以及 `ApiProjectStore` 契约。

`transom-store::JsonApiProjectStore` 是首个实现，文件路径为：

```text
<config_dir>/transom/api_projects.json
```

写入沿用 Transom 的原子写路径：临时文件、`fsync`、原子替换和损坏文件隔离。UI 通过 `Persistence` 的单槽异步写入器保存项目，避免在渲染和点击回调中同步写盘。

当前导入格式支持：

- 单个 `ApiProject` JSON；
- 含 `version` 与 `projects` 的 Transom API 项目登记表 JSON。

Postman、OpenAPI、Insomnia、cURL 等复杂格式转换器不在本阶段。

## HTTP 执行

`ApiHttpService` 持有全局复用的 `reqwest::Client`，连接池随应用进程复用。它必须运行在应用已有的 Tokio Runtime 中：

- GPUI 执行器不是 Tokio reactor，不能直接轮询 `reqwest` Future；
- `ApiHttpService::start` 在 Tokio 控制面派发请求；
- 结果经容量为 1 的 `oneshot` 返回 GPUI；
- 明细 `ApiExchange` 被丢弃时中止 Tokio 任务，关闭标签或重新发送不会留下旧请求；
- 响应体按流读取，超过 4 MiB 立即停止并返回中文错误；
- 网络请求配置连接超时与总超时，日志不输出请求头、请求体或完整敏感 URL。

## 当前 UI 能力

- 项目列表、搜索、新建、重命名、删除确认和本地 JSON 导入。
- 项目内一级目录与根请求的接口树。
- 新建、选择和删除请求。
- GET、POST、PUT、PATCH、DELETE、HEAD、OPTIONS 方法选择。
- URL、请求名称、逐行请求头和请求体编辑。
- 显式保存项目和发送当前请求。
- 响应状态、耗时、字节数、响应头和响应体查看。
- JSON 响应自动美化；非 JSON 文本原样展示。

## 明确未包含

- CyberAPI 的账号、团队、邀请、权限、MCP 服务端与服务器同步。
- GraphQL 补全、gRPC、WebSocket、SSE 长连接。
- 环境变量、Cookie 管理、OAuth、AWS SigV4、证书管理与代理 UI。
- Postman/OpenAPI/HAR/cURL 导入导出。
- 请求历史、代码生成、断言与提取、批量运行、Mock 服务。
- Git、容器和 API 标签的跨重启恢复。

这些能力如需迁移，应在独立阶段按 Transom 的实体、事件、错误与有界通道契约实现，不得直接引入 CyberAPI 包。

## 验收

自动化验证：

```text
cargo test -p transom-api
cargo test -p transom-store
cargo test -p transom-core
cargo test -p transom-ui --lib
cargo test -p transom-ui --test source_guards
cargo clippy -p transom-api -p transom-store -p transom-core -p transom-ui --all-targets -- -D warnings
```

真实窗口验证：

- 活动栏 API Client 入口打开项目侧栏；
- 预置项目与请求在真实窗口中加载；
- 项目标签与终端/Git/容器标签共用同一标签栏；
- 选择请求后编辑器正确回填；
- 新建项目内联编辑器能够打开；
- 向本地 HTTP 服务发送 GET 请求，界面显示 `200 OK`、耗时、字节数和美化后的 JSON 响应。
