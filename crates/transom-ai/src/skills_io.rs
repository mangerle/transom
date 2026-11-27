//! 技能文件沙箱读写、资源加载与本地 `SKILL.md` 文件夹一键导入。

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::AiError;
use crate::memory::resolve_sandbox_path;
use crate::skills::{
    SkillDefinition, load_single_skill_file, parse_skill_markdown, sanitize_skill_id,
};

const MAX_RESOURCE_BYTES: usize = 64 * 1024;

/// 保存自定义技能为标准 `<skills_dir>/<id>/SKILL.md` 格式。
pub fn save_custom_skill(
    skills_dir: &Path,
    raw_id: &str,
    title: &str,
    description: &str,
    tags: &[String],
    body: &str,
) -> Result<PathBuf, AiError> {
    let clean = sanitize_skill_id(raw_id);
    if clean.is_empty() || body.trim().is_empty() {
        return Err(AiError::StorageFailed {
            target: "skills".to_owned(),
            reason: "技能标识与正文均不能为空（标识仅支持字母、数字、-、_）".to_owned(),
        });
    }
    fs::create_dir_all(skills_dir).map_err(|e| AiError::StorageFailed {
        target: skills_dir.display().to_string(),
        reason: e.to_string(),
    })?;
    let skill_dir = resolve_sandbox_path(skills_dir, Path::new(&clean))?;
    fs::create_dir_all(&skill_dir).map_err(|e| AiError::StorageFailed {
        target: skill_dir.display().to_string(),
        reason: e.to_string(),
    })?;
    let path = skill_dir.join("SKILL.md");
    let title_val = if title.trim().is_empty() {
        clean.as_str()
    } else {
        title.trim()
    };
    let desc_val = if description.trim().is_empty() {
        "自定义运维技能"
    } else {
        description.trim()
    };
    let tags_line = if tags.is_empty() {
        String::new()
    } else {
        format!("tags: [{}]\n", tags.join(", "))
    };
    let md = format!(
        "---\nname: {clean}\ntitle: \"{title_val}\"\ncommand: /{clean}\ndescription: \"{desc_val}\"\n{tags_line}---\n\n{}\n",
        body.trim()
    );
    fs::write(&path, md).map_err(|e| AiError::StorageFailed {
        target: path.display().to_string(),
        reason: e.to_string(),
    })?;
    Ok(path)
}

/// 删除自定义技能（同时清理目录型与扁平 `.md`）。
pub fn delete_custom_skill(skills_dir: &Path, raw_id: &str) -> Result<bool, AiError> {
    let clean = sanitize_skill_id(raw_id);
    if clean.is_empty() {
        return Ok(false);
    }
    let mut removed = false;
    let dir_target = resolve_sandbox_path(skills_dir, Path::new(&clean))?;
    if dir_target.is_dir() {
        fs::remove_dir_all(&dir_target).map_err(|e| AiError::StorageFailed {
            target: dir_target.display().to_string(),
            reason: e.to_string(),
        })?;
        removed = true;
    }
    let flat_target = resolve_sandbox_path(skills_dir, Path::new(&format!("{clean}.md")))?;
    if flat_target.is_file() {
        fs::remove_file(&flat_target).map_err(|e| AiError::StorageFailed {
            target: flat_target.display().to_string(),
            reason: e.to_string(),
        })?;
        removed = true;
    }
    Ok(removed)
}

/// 安全读取目录型技能附带的 `references/` 或 `scripts/` 资源文件。
pub fn read_skill_resource(skill: &SkillDefinition, rel_path: &str) -> Result<String, AiError> {
    let Some(dir) = &skill.source_dir else {
        return Err(AiError::StorageFailed {
            target: skill.id.clone(),
            reason: "该技能不包含目录资源".to_owned(),
        });
    };
    let resolved = resolve_sandbox_path(dir, Path::new(rel_path))?;
    let bytes = fs::read(&resolved).map_err(|e| AiError::StorageFailed {
        target: resolved.display().to_string(),
        reason: e.to_string(),
    })?;
    let slice = &bytes[..bytes.len().min(MAX_RESOURCE_BYTES)];
    Ok(String::from_utf8_lossy(slice).into_owned())
}

/// 从本地 `SKILL.md` 文件夹或 `.md` 文件一键导入技能到 `skills_dir`。
pub fn import_skill_from_path(src: &Path, skills_dir: &Path) -> Result<SkillDefinition, AiError> {
    let src_dir = if src.is_file() && src.file_name().and_then(|s| s.to_str()) == Some("SKILL.md") {
        src.parent().unwrap_or(src)
    } else {
        src
    };

    if src_dir.is_dir() {
        let skill_md = src_dir.join("SKILL.md");
        if !skill_md.is_file() {
            return Err(AiError::StorageFailed {
                target: src_dir.display().to_string(),
                reason: "所选目录中未找到 SKILL.md 文件".to_owned(),
            });
        }
        let stem = src_dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("imported-skill");
        let id = sanitize_skill_id(stem);
        let dest_dir = resolve_sandbox_path(skills_dir, Path::new(&id))?;
        copy_skill_dir(src_dir, &dest_dir)?;
        return load_single_skill_file(&id, &dest_dir.join("SKILL.md"), Some(dest_dir)).ok_or_else(
            || AiError::StorageFailed {
                target: id,
                reason: "导入后的 SKILL.md 内容为空或无效".to_owned(),
            },
        );
    }

    if src.is_file() && src.extension().and_then(|s| s.to_str()) == Some("md") {
        let stem = src
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("imported");
        let raw = fs::read_to_string(src).map_err(|e| AiError::StorageFailed {
            target: src.display().to_string(),
            reason: e.to_string(),
        })?;
        let (_, title, desc, tags, body) = parse_skill_markdown(stem, &raw);
        let id = sanitize_skill_id(stem);
        let path = save_custom_skill(skills_dir, &id, &title, &desc, &tags, &body)?;
        return load_single_skill_file(&id, &path, path.parent().map(Path::to_path_buf))
            .ok_or_else(|| AiError::StorageFailed {
                target: id,
                reason: "导入的 Markdown 技能内容为空".to_owned(),
            });
    }

    Err(AiError::StorageFailed {
        target: src.display().to_string(),
        reason: "仅支持导入包含 SKILL.md 的文件夹或 .md 文件".to_owned(),
    })
}

fn copy_skill_dir(src: &Path, dst: &Path) -> Result<(), AiError> {
    fs::create_dir_all(dst).map_err(|e| AiError::StorageFailed {
        target: dst.display().to_string(),
        reason: e.to_string(),
    })?;
    let mut stack = vec![(src.to_path_buf(), dst.to_path_buf())];
    while let Some((cur_src, cur_dst)) = stack.pop() {
        let rd = fs::read_dir(&cur_src).map_err(|e| AiError::StorageFailed {
            target: cur_src.display().to_string(),
            reason: e.to_string(),
        })?;
        for entry in rd.flatten() {
            let name = entry.file_name();
            if name.to_string_lossy() == ".git" {
                continue;
            }
            let s_path = entry.path();
            let d_path = cur_dst.join(&name);
            if s_path.is_dir() {
                fs::create_dir_all(&d_path).map_err(|e| AiError::StorageFailed {
                    target: d_path.display().to_string(),
                    reason: e.to_string(),
                })?;
                stack.push((s_path, d_path));
            } else if s_path.is_file() && entry.metadata().is_ok_and(|m| m.len() <= 256 * 1024) {
                fs::copy(&s_path, &d_path).map_err(|e| AiError::StorageFailed {
                    target: d_path.display().to_string(),
                    reason: e.to_string(),
                })?;
            }
        }
    }
    Ok(())
}
