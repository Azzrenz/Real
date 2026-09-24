//! 项目画像预扫描：Planner 规划前预热文件清单与结构，减少探索浪费

use std::path::PathBuf;

/// 画像缓存落到工作区**之外**的全局目录（坑位 G8）：Real 内部的 project_profile
pub fn profile_cache_file(anchor: &str) -> PathBuf {
    let safe = anchor.replace(['\\', '/', ':'], "_");
    let dir = std::env::temp_dir().join("real_profile_cache");
    let _ = std::fs::create_dir_all(&dir);
    dir.join(format!("{safe}.json"))
}

/// 锚行输入（拆开摆：来源如实标注，不再"本轮已如此处理"式空口承诺）
pub struct AnchorInput<'a> {
    /// 本轮生效项目根（会话工作区解析后的值）
    pub workspace: Option<&'a str>,
    /// 名录命中：(路径, 命中的口语名)
    pub named: Option<(&'a str, &'a str)>,
    /// 上一轮的项目根（与本轮不同 → 标注"本轮切换"）
    pub prev: Option<&'a str>,
}

/// 本轮任务一句（原话压行 + 截断，CJK 安全）——
fn task_excerpt(user_input: &str) -> String {
    let line: String = user_input
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("  ");
    let mut out: String = line.chars().take(80).collect();
    if line.chars().count() > 80 {
        out.push('…');
    }
    if out.is_empty() {
        "（本轮未给文字指令）".to_string()
    } else {
        out
    }
}

/// 换项目标注：上一轮根与本轮根不同才写（同根/空值/脏值一律不写）
fn switch_note(prev: Option<&str>, now: &str) -> String {
    let Some(p) = prev
        .map(str::trim)
        .filter(|p| !p.is_empty() && !p.contains("://"))
    else {
        return String::new();
    };
    if super::project_registry::same_root(p, now) {
        String::new()
    } else {
        format!("；本轮切换：{p} → {now}")
    }
}

pub fn target_anchor_full(user_input: &str, ctx: AnchorInput<'_>) -> Option<String> {
    // ① 名录命中（口语项目名，如 "查 Real 的问题"）—— 优先于盘符抽取之外的一切
    let named_path = crate::agent::plan::extract_paths(user_input)
        .into_iter()
        .filter(|p| p.len() > 3 && p.as_bytes().get(1) == Some(&b':'))
        .filter_map(|p| {
            let pb = std::path::PathBuf::from(&p);
            if pb.is_dir() {
                Some(pb)
            } else if pb.is_file() {
                pb.parent().map(|d| d.to_path_buf())
            } else {
                None
            }
        })
        .max_by_key(|p| p.as_os_str().len());
    let (root, src) = match ctx.named {
        Some((p, alias)) => (p.to_string(), format!("项目名 \"{alias}\" 命中项目名录")),
        None => match named_path {
            Some(dir) => (dir.display().to_string(), "用户本轮点名".to_string()),
            None => match ctx.workspace.map(str::trim).filter(|w| !w.is_empty()) {
                Some(w) => (w.to_string(), "会话工作区".to_string()),
                None => return None,
            },
        },
    };
    let switched = switch_note(ctx.prev, &root);
    let task = task_excerpt(user_input);
    // 锚行拆两条：本轮任务（问什么）+ 项目根（在哪儿）。
    Some(format!(
        "【本轮任务】{task}\n\
         【项目根】{root}（来源：{src}{switched}）\n\
         - 用户口中的\"这个系统/这个项目/这个文件/你自己的…\"等指代，默认指**该根下的文件**；\n\
         - 同源仓内容相似度极高，未经点名不要互相取材；\n\
         - 用户点名了别的项目就按给的方式切换（本行来源已注明依据）；来源写\"会话工作区\"= 本轮未点名，沿用上一轮根。**问的边界**：只有“目标含糊到无法执行”（连对象都指不出）才问一句；只要能从本轮输入+本行项目根推断出对象，就直接执行，**不得用“你要哪条，我照办：1/2/3”这类选项菜单代替执行**。\n\
         - 动手前先核对一次：这一步要碰的项目/文件/模型属不属于上面那个根；对不上就停下确认——**证据不能反过来改目标**。"
    ))
}

pub fn tree_view(paths: &[String]) -> Option<String> {
    use std::collections::BTreeMap;
    let mut by_dir: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for p in paths {
        let path = std::path::Path::new(p);
        let dir = path.parent().map(|d| d.display().to_string()).unwrap_or_default();
        let name = path.file_name().map(|n| n.to_string_lossy().to_string())?;
        by_dir.entry(dir).or_default().push(name);
    }
    if by_dir.is_empty() {
        return None;
    }
    let mut out = String::from("【项目结构树】（本会话已确认的真实文件，按目录分组——规划路径时以此为准，勿凭记忆猜目录/层级）：\n");
    for (dir, mut files) in by_dir {
        files.sort();
        files.truncate(12);
        out.push_str(&format!("  {dir}/\n"));
        for f in &files {
            out.push_str(&format!("    {f}\n"));
        }
    }
    Some(out)
}

#[cfg(test)]
#[path = "project_profile_tests.rs"]
mod target_anchor_tests;
