//! 技能系统（0028）：SKILL.md 文件式技能，一个技能一个目录。

use crate::error::{AppError, AppResult};
use serde::Deserialize;
use serde_json::{json, Value};
use axum::extract::{Path as AxumPath, State};
use axum::Json;
use std::path::PathBuf;

/// 技能根目录：专门文件夹 data/skills（运行时可增删；REAL_SKILLS_ROOT 可覆盖）
pub fn skills_root() -> PathBuf {
    if let Ok(r) = std::env::var("REAL_SKILLS_ROOT") {
        if !r.trim().is_empty() {
            return PathBuf::from(r);
        }
    }
    crate::path::data_root::skills_dir()
}

/// 目录名消毒：只留 字母/数字/中文/连字符/下划线，防路径逃逸
fn sanitize_name(raw: &str) -> String {
    raw.trim()
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .filter(|c| (*c as u32) < 0x2E80 || (*c as u32) >= 0x4E00 || c.is_alphanumeric())
        .collect::<String>()
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_' || (*c as u32) >= 0x2E80)
        .collect()
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SkillEntry {
    pub name: String,
    pub description: String,
    /// 分类（架构 / UI / 工程 / 排障 / 应用…）：技能归属的领域，列表显示为标题前缀。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub category: String,
    /// 正文要点（## / ### 标题，去掉 markdown 符号，最多 6 条）
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub outline: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub summary: String,
}

/// 抽导语：H1 之后到首个二级标题为止的段落，去 `>` 与 `**`，超 400 字截断。
fn extract_summary(body: &str) -> String {
    let mut seen_h1 = false;
    let mut buf: Vec<String> = Vec::new();
    for line in body.lines() {
        let t = line.trim();
        if !seen_h1 {
            if t.starts_with("# ") {
                seen_h1 = true;
            }
            continue;
        }
        if t.starts_with('#') {
            break;
        }
        if t.is_empty() {
            continue;
        }
        buf.push(
            t.trim_start_matches('>')
                .replace("**", "")
                .trim()
                .to_string(),
        );
    }
    let s = buf.join("\n");
    let s = s.trim();
    if s.chars().count() > 400 {
        let mut out: String = s.chars().take(400).collect();
        out.push('…');
        out
    } else {
        s.to_string()
    }
}

/// 从正文抽要点：取 ## / ### 标题行，去 markdown 符号（* ` #），最多 max 条。
fn extract_outline(body: &str, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    for line in body.lines() {
        let t = line.trim();
        let rest = match t.strip_prefix("## ").or_else(|| t.strip_prefix("### ")) {
            Some(r) => r,
            None => continue,
        };
        let s = rest
            .replace(['*', '`', '#'], "")
            .trim()
            .to_string();
        if s.is_empty() {
            continue;
        }
        out.push(s);
        if out.len() >= max {
            break;
        }
    }
    out
}

/// 解析 SKILL.md：frontmatter（--- 包裹的 name/description/category）+ 正文。
pub fn parse_skill_md(raw: &str) -> (String, String, String, String) {
    let mut name = String::new();
    let mut description = String::new();
    let mut category = String::new();
    let mut body = raw.to_string();
    let t = raw.trim_start();
    if let Some(rest) = t.strip_prefix("---") {
        if let Some(end) = rest.find("\n---") {
            let fm = &rest[..end];
            body = rest[end + 4..].trim_start_matches(['\n', '\r']).to_string();
            for line in fm.lines() {
                if let Some(v) = line.strip_prefix("name:") {
                    name = v.trim().to_string();
                } else if let Some(v) = line.strip_prefix("description:") {
                    description = v.trim().to_string();
                } else if let Some(v) = line.strip_prefix("category:") {
                    category = v.trim().to_string();
                }
            }
        }
    }
    (name, description, category, body)
}

fn skill_file(name: &str) -> AppResult<PathBuf> {
    let n = sanitize_name(name);
    if n.is_empty() {
        return Err(AppError::Validation("技能名不能为空".into()));
    }
    Ok(skills_root().join(&n).join("SKILL.md"))
}

/// GET /api/skills —— 技能目录清单（薄索引：name + description）
pub async fn list_skills(State(_): State<crate::state::AppState>) -> AppResult<Json<Value>> {
    let root = skills_root();
    let mut skills = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&root) {
        for e in rd.flatten() {
            let f = e.path().join("SKILL.md");
            if let Ok(raw) = std::fs::read_to_string(&f) {
                let (name, description, category, body) = parse_skill_md(&raw);
                if !name.is_empty() {
                    skills.push(SkillEntry {
                        outline: extract_outline(&body, 12),
                        summary: extract_summary(&body),
                        name,
                        description,
                        category,
                    });
                }
            }
        }
    }
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Json(json!({ "skills": skills })))
}

/// 读单个技能全文（编排注入用）：命中返回 Some((name, description, body))
pub fn load_skill_text(name: &str) -> Option<String> {
    let f = skill_file(name).ok()?;
    std::fs::read_to_string(f).ok()
}

/// GET /api/skills/{name}/raw —— 技能全文（技能详情独立窗口渲染用）。
pub async fn skill_raw(AxumPath(name): AxumPath<String>) -> AppResult<Json<Value>> {
    let text = load_skill_text(&name)
        .ok_or_else(|| AppError::NotFound(format!("技能「{name}」不存在")))?;
    Ok(Json(json!({ "name": name, "text": text })))
}

#[derive(Debug, Deserialize)]
pub struct SkillBody {
    pub name: String,
    pub description: String,
    pub prompt: String,
    /// 分类（可选）：架构 / UI / 工程 / 排障 / 应用… 未填则不写入 frontmatter
    #[serde(default)]
    pub category: Option<String>,
    /// 并入目标（可选，09-10 沉淀候选写入）：给了就追加到该技能 SKILL.md 末尾一节，
    #[serde(default)]
    pub merge_into: Option<String>,
}

/// POST /api/skills —— 添加技能（写 skills\<name>\SKILL.md）
pub async fn create_skill(
    AxumPath(_ws): AxumPath<()>,
    Json(req): Json<SkillBody>,
) -> AppResult<Json<Value>> {
    let _ = _ws;
    let name = sanitize_name(&req.name);
    if name.is_empty() {
        return Err(AppError::Validation("技能名不能为空（字母/数字/中文/-/_）".into()));
    }
    if req.prompt.trim().is_empty() {
        return Err(AppError::Validation("技能正文（工作流话术）不能为空".into()));
    }
    let dir = skills_root().join(&name);
    std::fs::create_dir_all(&dir).map_err(|e| AppError::Internal(format!("创建技能目录失败: {e}")))?;
    // 分类写进 frontmatter（技能自带归属领域，列表按它做标题前缀；未填就不写这一行）
    let cat = req
        .category
        .as_deref()
        .unwrap_or("")
        .trim()
        .to_string();
    let cat_line = if cat.is_empty() {
        String::new()
    } else {
        format!("category: {cat}\n")
    };
    let md = format!(
        "---\nname: {}\ndescription: {}\n{cat_line}---\n\n{}\n",
        name,
        req.description.trim(),
        req.prompt.trim()
    );
    std::fs::write(dir.join("SKILL.md"), md)
        .map_err(|e| AppError::Internal(format!("写入 SKILL.md 失败: {e}")))?;
    Ok(Json(json!({"ok": true, "name": name})))
}

/// 按 frontmatter name 定位技能目录（目录名可能与 name 不一致，逐个解析匹配）
fn skill_dir_by_name(name: &str) -> Option<PathBuf> {
    let root = skills_root();
    if let Ok(rd) = std::fs::read_dir(&root) {
        for e in rd.flatten() {
            let p = e.path();
            if !p.is_dir() {
                continue;
            }
            if let Ok(raw) = std::fs::read_to_string(p.join("SKILL.md")) {
                let (n, _, _, _) = parse_skill_md(&raw);
                if n == name {
                    return Some(p);
                }
            }
        }
    }
    // 兜底：目录名恰等于 name
    let d = root.join(sanitize_name(name));
    if d.is_dir() {
        Some(d)
    } else {
        None
    }
}

/// 只替换 frontmatter 段里的 name 行（保留其余字段与正文原文，不重建文件）
fn rewrite_frontmatter_name(raw: &str, new: &str) -> String {
    let mut out = String::new();
    let mut in_fm = false;
    let mut fm_done = false;
    let mut replaced = false;
    for (i, line) in raw.lines().enumerate() {
        if i == 0 && line.trim() == "---" {
            in_fm = true;
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if in_fm {
            if line.trim() == "---" {
                in_fm = false;
                fm_done = true;
                out.push_str(line);
                out.push('\n');
                continue;
            }
            if !replaced && line.starts_with("name:") {
                out.push_str(&format!("name: {new}\n"));
                replaced = true;
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    let _ = fm_done;
    if !replaced {
        // 无 frontmatter name → 在文件头补一段最小 frontmatter
        return format!("---\nname: {new}\n---\n\n{raw}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rename_only_touches_frontmatter_name() {
        let raw = "---\nname: old\ndescription: 说明保持\n---\n\n正文里的 name: old 不该被动\n";
        let out = rewrite_frontmatter_name(raw, "new");
        assert!(out.contains("name: new"), "frontmatter name 应被替换");
        assert!(out.contains("description: 说明保持"), "其余 frontmatter 字段应保留");
        assert!(out.contains("正文里的 name: old 不该被动"), "正文不得被改");
    }

    #[test]
    fn rename_adds_frontmatter_when_missing() {
        let out = rewrite_frontmatter_name("纯正文没有 frontmatter", "new");
        assert!(out.starts_with("---\nname: new\n---\n\n纯正文没有 frontmatter"));
    }

    #[test]
    fn outline_takes_h2_h3_and_skips_h1() {
        // 一级标题是技能名（"# 自进化飞轮"），不算要点；## / ### 才是"它会做什么"
        let body = "# 自进化飞轮\n\n## 何时用\n\n正文\n\n### 输入\n\n## 流程（严格按顺序）\n";
        assert_eq!(
            extract_outline(body, 6),
            vec!["何时用".to_string(), "输入".to_string(), "流程（严格按顺序）".to_string()]
        );
    }

    #[test]
    fn outline_strips_markdown_and_respects_cap() {
        let body = "## **粗体** 与 `代码`\n## 二\n## 三\n## 四\n";
        let v = extract_outline(body, 2);
        assert_eq!(v, vec!["粗体 与 代码".to_string(), "二".to_string()]);
    }

    #[test]
    fn sanitize_rejects_path_escape() {
        assert_eq!(sanitize_name("../../etc/passwd"), "etcpasswd");
        assert_eq!(sanitize_name("  my skill! "), "myskill");
    }
}

#[derive(Debug, Deserialize)]
pub struct RenameBody {
    pub new_name: String,
}

/// POST /api/skills/{name}/rename —— 改名：目录名 + frontmatter name 同步
pub async fn rename_skill(
    AxumPath(name): AxumPath<String>,
    Json(req): Json<RenameBody>,
) -> AppResult<Json<Value>> {
    let new = sanitize_name(&req.new_name);
    if new.is_empty() {
        return Err(AppError::Validation("新技能名不能为空（字母/数字/中文/-/_）".into()));
    }
    if new == name {
        return Ok(Json(json!({ "ok": true, "name": new })));
    }
    let dir = skill_dir_by_name(&name)
        .ok_or_else(|| AppError::Validation(format!("技能「{name}」不存在")))?;
    let target = skills_root().join(&new);
    if target.exists() {
        return Err(AppError::Validation(format!("已存在同名技能「{new}」")));
    }
    std::fs::rename(&dir, &target)
        .map_err(|e| AppError::Internal(format!("重命名失败: {e}")))?;
    let f = target.join("SKILL.md");
    if let Ok(raw) = std::fs::read_to_string(&f) {
        let _ = std::fs::write(&f, rewrite_frontmatter_name(&raw, &new));
    }
    Ok(Json(json!({ "ok": true, "name": new })))
}

/// 在系统文件管理器中打开目录（跨平台：explorer / open / xdg-open）
fn open_in_explorer(dir: &std::path::Path) -> AppResult<()> {
    let prog = if cfg!(target_os = "windows") {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(prog)
        .arg(dir)
        .spawn()
        .map_err(|e| AppError::Internal(format!("打开文件夹失败: {e}")))?;
    Ok(())
}

/// POST /api/skills/{name}/open —— 打开该技能所在目录
pub async fn open_skill_dir(AxumPath(name): AxumPath<String>) -> AppResult<Json<Value>> {
    let dir = skill_dir_by_name(&name)
        .ok_or_else(|| AppError::Validation(format!("技能「{name}」不存在")))?;
    open_in_explorer(&dir)?;
    Ok(Json(json!({ "ok": true, "path": dir.to_string_lossy() })))
}

/// DELETE /api/skills/{name} —— 删除技能目录
pub async fn delete_skill(
    AxumPath(name): AxumPath<String>,
) -> AppResult<Json<Value>> {
    // 列表给的是 frontmatter name，按它定位真实目录（目录名可能与之不一致）
    if let Some(dir) = skill_dir_by_name(&name) {
        std::fs::remove_dir_all(&dir)
            .map_err(|e| AppError::Internal(format!("删除技能失败: {e}")))?;
    }
    Ok(Json(json!({"ok": true})))
}

/// POST /api/skills/absorb —— 沉淀候选一键落盘（09-10 用户：预览卡要能写入）。
pub async fn absorb_candidate(Json(req): Json<SkillBody>) -> AppResult<Json<Value>> {
    let name = sanitize_name(&req.name);
    if name.is_empty() {
        return Err(AppError::Validation("技能名不能为空（字母/数字/中文/-/_）".into()));
    }
    if req.prompt.trim().is_empty() {
        return Err(AppError::Validation("沉淀正文（summary+证据）不能为空".into()));
    }
    match req.merge_into.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        Some(target) => {
            let t = sanitize_name(target);
            let p = skills_root().join(&t).join("SKILL.md");
            if !p.exists() {
                return Err(AppError::Validation(format!("并入目标技能不存在: {t}")));
            }
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&p)
                .map_err(|e| AppError::Internal(format!("打开目标技能失败: {e}")))?;
            writeln!(f, "\n## {name}\n\n{}\n", req.prompt.trim())
                .map_err(|e| AppError::Internal(format!("追加失败: {e}")))?;
            Ok(Json(serde_json::json!({ "ok": true, "merged_into": t })))
        }
        None => {
            let dir = skills_root().join(&name);
            std::fs::create_dir_all(&dir)
                .map_err(|e| AppError::Internal(format!("创建技能目录失败: {e}")))?;
            let cat = req.category.as_deref().unwrap_or("").trim().to_string();
            let cat_line = if cat.is_empty() { String::new() } else { format!("category: {cat}\n") };
            let md = format!(
                "---\nname: {name}\ndescription: {}\n{cat_line}---\n\n{}\n",
                req.description.trim(),
                req.prompt.trim()
            );
            std::fs::write(dir.join("SKILL.md"), md)
                .map_err(|e| AppError::Internal(format!("写入 SKILL.md 失败: {e}")))?;
            Ok(Json(serde_json::json!({ "ok": true, "created": name })))
        }
    }
}
