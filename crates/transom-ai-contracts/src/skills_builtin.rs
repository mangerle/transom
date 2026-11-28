//! 内置高频运维技能预设表。

use crate::skills::SkillDefinition;

/// 内置高频运维技能预设表（已全面适配全局助手与本机/远端协同路由）。
pub fn builtin_skills() -> Vec<SkillDefinition> {
    vec![
        SkillDefinition {
            id: "inspect".to_owned(),
            command: "/inspect".to_owned(),
            title: "主机全景巡检".to_owned(),
            description: "全链路只读巡检 CPU/内存/磁盘/容器/systemd 服务与近 24h 错误日志并分级汇总".to_owned(),
            tags: vec!["巡检".to_owned(), "健康检查".to_owned()],
            prompt_template: "请执行结构化只读健康巡检：\n\
                1. **确定目标**：若当前未绑定活动 SSH 会话且用户未指定目标，先调用 `session_catalog(action=\"list\")` 查看已保存主机后按用户指定的 `target` 执行（支持 `target=\"local\"` 巡检本机）；若已绑定当前标签页且无额外目标，直接对当前主机执行。\n\
                2. **同步计划**：调用 `update_plan` 列出巡检步骤并推进进度。\n\
                3. **只读采集**：优先调用 `inspect_host_runtime`（若在当前活动主机）或 `remote_exec` 采集 `uname -a; uptime; free -m; df -hT; df -i; systemctl --failed --no-pager 2>/dev/null; journalctl -p err -n 30 --no-pager 2>/dev/null || dmesg -T | tail -n 30`，并用 `container_workspace(action=\"list_containers\")` 检查容器状态。\n\
                4. **分级输出**：按「总体结论摘要 -> P0 严重异常 -> P1 警告风险 -> 正常项概览 -> 可执行修复建议」输出；严禁擅自修改系统状态。".to_owned(),
            builtin: true,
            enabled: true,
            source_dir: None,
            resources: Vec::new(),
        },
        SkillDefinition {
            id: "diagnose".to_owned(),
            command: "/diagnose".to_owned(),
            title: "故障根因诊断".to_owned(),
            description: "结合终端报错、日志与配置定位故障根因，复杂场景支持并发只读子 Agent 排查".to_owned(),
            tags: vec!["排障".to_owned(), "日志".to_owned()],
            prompt_template: "请按系统化排障流程定位问题根因：\n\
                1. **收集现场**：若在当前终端会话，先调用 `get_recent_blocks` 查看最近失败命令与退出码；若指定了其他服务器或本机，用 `remote_exec(target=...)` 与 `remote_fs(action=\"read\", ...)` 读取关联服务日志、进程状态与配置文件。\n\
                2. **并行下钻**：若涉及多台主机或多组件交叉排查，可调用 `delegate_task(mode=\"readonly\")` 并行收集证据，避免海量日志污染主上下文。\n\
                3. **输出结论**：严格按「现象摘要 -> 关键证据链 -> 明确根因 -> 最小修复方案与回滚/验证步骤」输出；未获用户确认前不直接改动生产配置。".to_owned(),
            builtin: true,
            enabled: true,
            source_dir: None,
            resources: Vec::new(),
        },
        SkillDefinition {
            id: "deploy".to_owned(),
            command: "/deploy".to_owned(),
            title: "标准安全部署".to_owned(),
            description: "覆盖本机构建/离线包上传、Docker/Compose 部署与远端服务升级的全流程安全发布".to_owned(),
            tags: vec!["部署".to_owned(), "发布".to_owned(), "容器".to_owned()],
            prompt_template: "请按 Transom 标准安全部署规范执行发布：\n\
                1. **目标确认与规划**：未指定服务器时先用 `session_catalog(action=\"list\")` 确认目标 `target`；先调 `update_plan` 拆分步骤。\n\
                2. **只读探测**：检查目标机 OS、架构、运行环境、目标目录权限与端口占用；若涉及本地构建或下载离线包，先用 `remote_exec(target=\"local\", ...)` 验证。\n\
                3. **选择最优部署路径**：\n\
                   - **文件/目录发布**：使用 `transfer_file` 上传单文件或整个构建目录，使用 `remote_fs(action=\"edit\")` 生成配置 Diff 并自动备份；\n\
                   - **容器/Compose 部署**：优先使用 `container_workspace` 的 `create_and_start_container` 或 `deploy_compose_yaml`（支持本机与远端 `target`）。\n\
                4. **风险与回滚**：写操作前向用户说明变更影响面与回滚路径，所有变更走审批门后执行。\n\
                5. **验收沉淀**：部署后执行端口/HTTP/容器健康探活，并提议将关键部署路径与端口约束写入长期记忆。".to_owned(),
            builtin: true,
            enabled: true,
            source_dir: None,
            resources: Vec::new(),
        },
        SkillDefinition {
            id: "disk".to_owned(),
            command: "/disk".to_owned(),
            title: "磁盘与 Inode 深度分析".to_owned(),
            description: "只读定位磁盘容量、Inode、大文件、已删未释放句柄及容器/日志占用并给清理建议".to_owned(),
            tags: vec!["磁盘".to_owned(), "存储".to_owned()],
            prompt_template: "请对目标主机（支持本机 `target=\"local\"` 或指定远端 `target`）执行只读磁盘与 Inode 分析：\n\
                1. 采集 `df -hT` 与 `df -i`，识别使用率最高的挂载点；\n\
                2. 安全统计顶层及可疑目录占用（`du -xhd1 /var /home /opt 2>/dev/null | sort -hr | head -n 15`），排查大日志、Docker 占用（`docker system df 2>/dev/null`）及已删除但被进程占用的句柄（`lsof +L1 2>/dev/null | head -n 20`）；\n\
                3. 汇总占用排行与可安全清理项（注明预估可释放空间与风险等级），严禁擅自执行删除命令。".to_owned(),
            builtin: true,
            enabled: true,
            source_dir: None,
            resources: Vec::new(),
        },
        SkillDefinition {
            id: "docker".to_owned(),
            command: "/docker".to_owned(),
            title: "容器与 Compose 诊断".to_owned(),
            description: "排查异常退出、重启、OOM 或不健康容器及镜像/卷/网络资源占用".to_owned(),
            tags: vec!["Docker".to_owned(), "Podman".to_owned(), "容器".to_owned()],
            prompt_template: "请优先利用 `container_workspace`（支持本机或传入 `target` 指向远端主机）与 `remote_exec` 排查容器状态：\n\
                1. 列出全部容器（含已停止/退出状态），重点筛选 `Exited`、`Restarting`、OOMKilled 或 `unhealthy` 容器；\n\
                2. 读取异常容器最近日志（`container_logs`）与 `inspect_container` 退出码/OOM 标记/挂载配置；\n\
                3. 结合磁盘、端口冲突与环境变量给出明确的故障原因与修复/Compose 调整方案。".to_owned(),
            builtin: true,
            enabled: true,
            source_dir: None,
            resources: Vec::new(),
        },
        SkillDefinition {
            id: "sessions".to_owned(),
            command: "/sessions".to_owned(),
            title: "管理软件会话".to_owned(),
            description: "列出已保存的服务器会话与分组目录，或协助快速连接与调整配置".to_owned(),
            tags: vec!["会话".to_owned(), "SSH".to_owned(), "分组".to_owned()],
            prompt_template: "请调用 `session_catalog` 工具列出所有已保存的服务器会话和分组目录：\n\
                1. 按分组清晰展示会话名称、主机地址、端口与保护级别；\n\
                2. 若用户需要连接或调整会话配置，给出对应的操作建议并协助执行。"
                .to_owned(),
            builtin: true,
            enabled: true,
            source_dir: None,
            resources: Vec::new(),
        },
    ]
}
