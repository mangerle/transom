//! 运维技能体系（Agent Skills：支持标准 `SKILL.md` 目录、YAML Frontmatter 解析与渐进式披露）。
//!
//! 设计原理（Why）：
//! - 兼容业界标准 `<skill-id>/SKILL.md`（含 `references/`、`scripts/` 附带资源）与扁平 `<skill-id>.md`；
//! - 系统提示词仅注入已启用技能的极简摘要 [`SkillSummary`]，命中场景时由 AI 通过 `load_skill` 按需加载完整指令与资源；
//! - 保留手动 `/command` 前缀触发，与自动按需加载双轨并行。

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub use crate::skills_io::{
    delete_custom_skill, import_skill_from_path, read_skill_resource, save_custom_skill,
};

const MAX_SKILL_BYTES: u64 = 128 * 1024;

/// 单个运维技能定义。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillDefinition {
    pub id: String,
    pub command: String,
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub prompt_template: String,
    #[serde(default)]
    pub builtin: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(skip)]
    pub source_dir: Option<PathBuf>,
    #[serde(default)]
    pub resources: Vec<String>,
}

/// 注入系统提示词的技能极简摘要（用于渐进式披露）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillSummary {
    pub id: String,
    pub command: String,
    pub title: String,
    pub description: String,
    pub resource_count: usize,
}

fn default_true() -> bool {
    true
}

impl SkillDefinition {
    pub fn to_summary(&self) -> SkillSummary {
        SkillSummary {
            id: self.id.clone(),
            command: self.command.clone(),
            title: self.title.clone(),
            description: self.description.clone(),
            resource_count: self.resources.len(),
        }
    }
}

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
    ]
}

/// 解析技能 Markdown（支持标准 YAML frontmatter：`name`、`title`、`description`、`command`、`tags`）。
pub fn parse_skill_markdown(
    stem: &str,
    raw: &str,
) -> (Option<String>, String, String, Vec<String>, String) {
    let normalized = raw.replace("\r\n", "\n");
    let Some(after_start) = normalized.strip_prefix("---\n") else {
        return (
            None,
            stem.to_owned(),
            "自定义运维技能".to_owned(),
            Vec::new(),
            normalized.trim().to_owned(),
        );
    };
    let Some(end_idx) = after_start.find("\n---") else {
        return (
            None,
            stem.to_owned(),
            "自定义运维技能".to_owned(),
            Vec::new(),
            normalized.trim().to_owned(),
        );
    };

    let fm = &after_start[..end_idx];
    let body = after_start[end_idx + 4..]
        .trim_start_matches(['\n', '-'])
        .trim()
        .to_owned();
    let mut name: Option<String> = None;
    let mut title: Option<String> = None;
    let mut desc: Option<String> = None;
    let mut cmd: Option<String> = None;
    let mut tags = Vec::new();

    for line in fm.lines() {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let key = k.trim().to_ascii_lowercase();
        let val = trim_yaml_val(v);
        if val.is_empty() {
            continue;
        }
        match key.as_str() {
            "name" => name = Some(val),
            "title" => title = Some(val),
            "description" => desc = Some(val),
            "command" => cmd = Some(val),
            "tags" => {
                let inner = val.trim_start_matches('[').trim_end_matches(']');
                tags = inner
                    .split(',')
                    .map(trim_yaml_val)
                    .filter(|s| !s.is_empty())
                    .collect();
            }
            _ => {}
        }
    }

    let final_title = title.or(name).unwrap_or_else(|| stem.to_owned());
    let final_desc = desc.unwrap_or_else(|| "自定义运维技能".to_owned());
    (cmd, final_title, final_desc, tags, body)
}

fn trim_yaml_val(raw: &str) -> String {
    let t = raw.trim();
    if (t.starts_with('"') && t.ends_with('"')) || (t.starts_with('\'') && t.ends_with('\'')) {
        t[1..t.len().saturating_sub(1)].trim().to_owned()
    } else {
        t.to_owned()
    }
}

/// 加载全部内置与自定义技能（兼容 `<dir>/<id>/SKILL.md` 目录与 `<dir>/<id>.md` 扁平文件）并应用禁用列表。
pub fn load_skills_with_disabled(
    custom_dir: Option<&Path>,
    disabled_ids: &[String],
) -> Vec<SkillDefinition> {
    let mut list = builtin_skills();
    if let Some(dir) = custom_dir
        && let Ok(entries) = fs::read_dir(dir)
    {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let skill_file = path.join("SKILL.md");
                if !skill_file.is_file() {
                    continue;
                }
                let Some(stem) = path.file_name().and_then(|s| s.to_str()) else {
                    continue;
                };
                if let Some(skill) = load_single_skill_file(stem, &skill_file, Some(path.clone())) {
                    merge_custom_skill(&mut list, skill);
                }
            } else if path.extension().and_then(|s| s.to_str()) == Some("md") {
                let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                    continue;
                };
                if let Some(skill) = load_single_skill_file(stem, &path, None) {
                    merge_custom_skill(&mut list, skill);
                }
            }
        }
    }

    let disabled: HashSet<String> = disabled_ids
        .iter()
        .map(|s| s.trim().trim_start_matches('/').to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    for skill in &mut list {
        let id_lc = skill.id.to_ascii_lowercase();
        let cmd_lc = skill.command.trim_start_matches('/').to_ascii_lowercase();
        skill.enabled = !disabled.contains(&id_lc) && !disabled.contains(&cmd_lc);
    }
    list
}

/// 加载全部内置与自定义技能（不应用禁用过滤）。
pub fn load_all_skills(custom_dir: Option<&Path>) -> Vec<SkillDefinition> {
    load_skills_with_disabled(custom_dir, &[])
}

/// 提取已启用技能的极简摘要列表（供系统提示词渐进式披露使用）。
pub fn summarize_enabled_skills(
    custom_dir: Option<&Path>,
    disabled_ids: &[String],
) -> Vec<SkillSummary> {
    load_skills_with_disabled(custom_dir, disabled_ids)
        .into_iter()
        .filter(|s| s.enabled)
        .map(|s| s.to_summary())
        .collect()
}

pub(crate) fn load_single_skill_file(
    stem: &str,
    file: &Path,
    source_dir: Option<PathBuf>,
) -> Option<SkillDefinition> {
    let meta = fs::metadata(file).ok()?;
    if meta.len() > MAX_SKILL_BYTES {
        return None;
    }
    let content = fs::read_to_string(file).ok()?;
    if content.trim().is_empty() {
        return None;
    }
    let id = sanitize_skill_id(stem);
    if id.is_empty() {
        return None;
    }
    let (cmd_opt, title, description, tags, body) = parse_skill_markdown(&id, &content);
    if body.is_empty() {
        return None;
    }
    let command = match cmd_opt {
        Some(c) if c.starts_with('/') => c,
        Some(c) => format!("/{c}"),
        None => format!("/{id}"),
    };
    let resources = source_dir
        .as_deref()
        .map(collect_skill_resources)
        .unwrap_or_default();

    Some(SkillDefinition {
        id,
        command,
        title,
        description,
        tags,
        prompt_template: body,
        builtin: false,
        enabled: true,
        source_dir,
        resources,
    })
}

fn merge_custom_skill(list: &mut Vec<SkillDefinition>, mut custom: SkillDefinition) {
    if let Some(existing) = list
        .iter_mut()
        .find(|s| s.id.eq_ignore_ascii_case(&custom.id))
    {
        custom.builtin = existing.builtin;
        *existing = custom;
    } else {
        list.push(custom);
    }
}

fn collect_skill_resources(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for sub in ["references", "scripts"] {
        let sub_dir = dir.join(sub);
        if !sub_dir.is_dir() {
            continue;
        }
        let mut stack = vec![sub_dir];
        while let Some(cur) = stack.pop() {
            let Ok(rd) = fs::read_dir(&cur) else {
                continue;
            };
            for entry in rd.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.is_file()
                    && let Ok(rel) = p.strip_prefix(dir)
                {
                    out.push(rel.to_string_lossy().replace('\\', "/"));
                    if out.len() >= 64 {
                        return out;
                    }
                }
            }
        }
    }
    out.sort();
    out
}

/// 根据用户输入前缀匹配已启用技能，返回 `(命中的技能, 剥离斜杠指令后的用户补充文本)`。
pub fn match_skill<'a>(
    input: &'a str,
    skills: &'a [SkillDefinition],
) -> Option<(&'a SkillDefinition, &'a str)> {
    let trimmed = input.trim();
    if !trimmed.starts_with('/') {
        return None;
    }
    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let cmd = parts.next()?;
    let rest = parts.next().unwrap_or("").trim();
    let skill = skills
        .iter()
        .find(|item| item.enabled && item.command.eq_ignore_ascii_case(cmd))?;
    Some((skill, rest))
}

/// 在整段输入中提取第一个命中的已启用 `/command` 技能，并保留移除该命令后的其余文本（如 `@local`）。
pub fn extract_skill_command<'a>(
    input: &'a str,
    skills: &'a [SkillDefinition],
) -> Option<(&'a SkillDefinition, String)> {
    let tokens: Vec<&str> = input.split_whitespace().collect();
    for (idx, token) in tokens.iter().enumerate() {
        if !token.starts_with('/') {
            continue;
        }
        if let Some(skill) = skills
            .iter()
            .find(|item| item.enabled && item.command.eq_ignore_ascii_case(token))
        {
            let rest = tokens
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != idx)
                .map(|(_, t)| *t)
                .collect::<Vec<_>>()
                .join(" ");
            return Some((skill, rest));
        }
    }
    None
}

pub fn sanitize_skill_id(raw: &str) -> String {
    raw.trim()
        .trim_start_matches('/')
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        builtin_skills, delete_custom_skill, extract_skill_command, import_skill_from_path,
        load_skills_with_disabled, match_skill, parse_skill_markdown, read_skill_resource,
        save_custom_skill,
    };

    #[test]
    fn match_skill_extracts_command_and_arguments() {
        let skills = builtin_skills();
        let (skill, rest) =
            match_skill("/diagnose 订单服务报错 502", &skills).expect("应命中 /diagnose");
        assert_eq!(skill.command, "/diagnose");
        assert_eq!(rest, "订单服务报错 502");
    }

    #[test]
    fn extract_skill_command_supports_mentions_before_or_after() {
        let skills = builtin_skills();
        let (skill, rest) =
            extract_skill_command("@local /inspect", &skills).expect("应命中 /inspect");
        assert_eq!(skill.command, "/inspect");
        assert_eq!(rest, "@local");

        let (skill2, rest2) =
            extract_skill_command("/deploy @prod-01 nginx", &skills).expect("应命中 /deploy");
        assert_eq!(skill2.command, "/deploy");
        assert_eq!(rest2, "@prod-01 nginx");
    }

    #[test]
    fn parses_yaml_frontmatter_and_tags() {
        let raw = "---\nname: k8s-check\ntitle: \"K8s 巡检\"\ncommand: /k8s\ndescription: \"集群节点与 Pod 排查\"\ntags: [k8s, 容器]\n---\n\n请检查 kubectl get nodes。\n";
        let (cmd, title, desc, tags, body) = parse_skill_markdown("k8s-check", raw);
        assert_eq!(cmd.as_deref(), Some("/k8s"));
        assert_eq!(title, "K8s 巡检");
        assert_eq!(desc, "集群节点与 Pod 排查");
        assert_eq!(tags, vec!["k8s", "容器"]);
        assert_eq!(body, "请检查 kubectl get nodes。");
    }

    #[test]
    fn loads_directory_skill_with_resources_and_respects_disabled() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("skills");
        let saved = save_custom_skill(
            &dir,
            "nginx-audit",
            "Nginx 审计",
            "检查 Nginx 安全配置",
            &["nginx".to_owned()],
            "先执行 nginx -t",
        )
        .unwrap();
        let ref_dir = saved.parent().unwrap().join("references");
        std::fs::create_dir_all(&ref_dir).unwrap();
        std::fs::write(ref_dir.join("checklist.md"), "检查 TLSv1.3").unwrap();

        let all = load_skills_with_disabled(Some(&dir), &["inspect".to_owned()]);
        let inspect = all.iter().find(|s| s.id == "inspect").unwrap();
        assert!(!inspect.enabled);
        assert!(match_skill("/inspect", &all).is_none());

        let custom = all.iter().find(|s| s.id == "nginx-audit").unwrap();
        assert!(custom.enabled);
        assert_eq!(custom.resources, vec!["references/checklist.md"]);
        assert_eq!(
            read_skill_resource(custom, "references/checklist.md").unwrap(),
            "检查 TLSv1.3"
        );
        assert!(read_skill_resource(custom, "../secret").is_err());

        assert!(delete_custom_skill(&dir, "nginx-audit").unwrap());
    }

    #[test]
    fn imports_external_skill_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let ext = tmp.path().join("external").join("redis-tune");
        std::fs::create_dir_all(ext.join("scripts")).unwrap();
        std::fs::write(
            ext.join("SKILL.md"),
            "---\nname: redis-tune\ndescription: \"Redis 内存与慢查询调优\"\n---\n\n执行 INFO memory。",
        )
        .unwrap();
        std::fs::write(ext.join("scripts").join("check.sh"), "redis-cli info").unwrap();

        let skills_dir = tmp.path().join("store_skills");
        let imported = import_skill_from_path(&ext, &skills_dir).unwrap();
        assert_eq!(imported.id, "redis-tune");
        assert_eq!(imported.resources, vec!["scripts/check.sh"]);
    }
}
