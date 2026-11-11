# Transom 自有图标

- 来源：Transom 项目自行绘制或基于既有品牌轮廓改绘，与 IntelliJ 图标资产分开维护。
- 用途：网络、云、数据库、磁盘、系统、数据卷、挂载、端口、进程、镜像、容器引擎与连接测试等需要稳定业务语义的界面。
- 许可：随项目按 Apache-2.0 分发。
- 维护：每个图标同时提供浅色与深色版本，颜色分别对齐 `#6C707E` 与 `#CED0D6`。
- 清晰度：所有描边统一使用继承的 `1.5px` 光学线宽，禁止在 14px 紧凑尺寸下使用 1px 或小数局部线宽。
- 保留语义图标：`database`、`cloud`、`info`、`connection_test` 等；文件浏览、编辑、折叠、上传、下载与 Git 标识统一复用 IntelliJ New UI 图标。
- 容器引擎：`docker`、`podman` 沿用原品牌轮廓并改为单色界面图标，避免高饱和品牌色在侧栏和标签栏中突兀。
- 通用动作：`upload`、`download`、`rename`、`desktop` 与 Git 标识已经统一改用 IntelliJ New UI 或 `UiIcon::VCS`，不再在自有图标目录中重复维护。
