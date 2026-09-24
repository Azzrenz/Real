//! 工具参数预处理管线：占位符解析 → 净化 → 残留检测 → 路径清单校验 → cwd 注入

use crate::path::{looks_like_placeholder, looks_like_placeholder_cmd};
use serde_json::{json, Value};
use std::collections::HashMap;

/// 参数净化层（设计决定："一进来就把路径问题解决"）
pub(crate) fn sanitize_path_args(args: &mut Value) {
    fn clean_str(s: &str, recover_control: bool) -> String {
        let mut t = s.trim().to_string();
        // ① 剥首尾匹配引号（ASCII 双/单引号 + 中文弯引号），循环剥多层
        loop {
            let chars: Vec<char> = t.trim().chars().collect();
            if chars.len() < 2 {
                break;
            }
            let (first, last) = (chars[0], *chars.last().unwrap());
            let is_quote = |c: char| {
                matches!(
                    c,
                    '"' | '\'' | '\u{201c}' | '\u{201d}' | '\u{2018}' | '\u{2019}'
                )
            };
            if is_quote(first) && is_quote(last) {
                t = chars[1..chars.len() - 1]
                    .iter()
                    .collect::<String>()
                    .trim()
                    .to_string();
            } else {
                break;
            }
        }
        // ② 还原 JSON 转义（os error 123 根源：\\ 与 \" 未还原）
        t = t
            .replace("\\\\", "\\")
            .replace("\\\"", "\"")
            .replace("\\/", "/");
        // ③ 剥尾部噪音（正则提取带进来的 `],)};` 等——**仅路径字段**：路径合法结尾
        if recover_control {
            while let Some(c) = t.chars().last() {
                if matches!(c, ']' | '}' | ')' | ',' | ';' | '"' | '\'') {
                    t.pop();
                } else {
                    break;
                }
            }
        }
        // ④ 控制字符处理（审理 N1 修复）
        t = t
            .chars()
            .map(|c| match c {
                '\t' if recover_control => 't',
                '\r' if recover_control => 'r',
                '\n' if recover_control => 'n',
                '\t' | '\r' | '\n' => c,
                c if c.is_control() => '\0',
                c => c,
            })
            .filter(|c| *c != '\0')
            .collect();
        t
    }
    fn walk(v: &mut Value, in_path_array: bool) {
        match v {
            Value::Object(map) => {
                let keys: Vec<String> = map.keys().cloned().collect();
                for k in keys {
                    let is_path_field =
                        matches!(k.as_str(), "paths" | "path" | "file" | "target" | "cwd");
                    match map.get_mut(&k) {
                        Some(Value::String(s)) => {
                            if is_path_field || k == "command" {
                                // 路径字段：\t\r\n 还原为字母（N1）；命令字段：保持剥离旧行为
                                let cleaned = clean_str(s, is_path_field);
                                *s = if is_path_field {
                                    // 路径字段
                                    cleaned.replace('\\', "/")
                                } else {
                                    // command 字段：**在这里不决定斜杠方向** ——
                                    cleaned
                                };
                            }
                        }
                        Some(val @ Value::Array(_)) if is_path_field => {
                            if let Some(arr) = val.as_array_mut() {
                                for item in arr.iter_mut() {
                                    if let Some(s) = item.as_str() {
                                        // 数组路径字段同样反斜杠→正斜杠（与 String 分支一致）
                                        *item =
                                            Value::String(clean_str(s, true).replace('\\', "/"));
                                    }
                                }
                            }
                        }
                        Some(other) => walk(other, in_path_array),
                        None => {}
                    }
                }
            }
            Value::Array(arr) => {
                for item in arr.iter_mut() {
                    walk(item, in_path_array);
                }
            }
            _ => {}
        }
    }
    walk(args, false);
}

// write 的 content 不再参与 #En 展开（见 prepare_tool_args 剥离逻辑），展开垃圾的种子

/// 路径归一（后端物化层）：模型给的 file/target 若不在磁盘，但能在项目文件清单中
pub fn resolve_known_path(raw: &str, session_id: &str) -> Option<String> {
    let norm = |s: &str| s.replace('\\', "/").to_ascii_lowercase();
    let raw_n = norm(raw);
    // 1) 真实存在即放行
    if std::path::Path::new(raw).is_file() {
        return Some(raw.to_string());
    }
    // 2) 收集项目文件清单（与 read 路径守卫同源）
    let mut originals: Vec<String> = Vec::new();
    if let Some(ws) = crate::agent::workspace::current(session_id) {
        let cache = crate::agent::project_profile::profile_cache_file(&ws);
        if let Ok(c) = std::fs::read_to_string(&cache) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&c) {
                if let Some(files) = v.get("files").and_then(|f| f.as_array()) {
                    originals.extend(
                        files
                            .iter()
                            .filter_map(|f| f.as_str().map(|s| s.to_string())),
                    );
                }
            }
        }
    }
    // 合并本会话已确认文件清单
    originals.extend(crate::agent::workspace::known_files(session_id));
    if originals.is_empty() {
        return None;
    }
    let known_norm: std::collections::HashSet<String> = originals.iter().map(|s| norm(s)).collect();
    // 3) basename 唯一匹配
    let base = std::path::Path::new(raw)
        .file_name()
        .and_then(|s| s.to_str())
        .map(norm);
    if let Some(b) = base {
        let suffix = format!("/{b}");
        let mut hits: Vec<String> = originals
            .iter()
            .filter(|o| {
                let n = norm(o);
                n == raw_n || n.ends_with(&suffix)
            })
            .cloned()
            .collect();
        if hits.len() == 1 {
            return Some(hits.remove(0));
        }
        // 4) 去掉误入的 /core/ 段（poetry-core 误判）
        let stripped = raw_n.replacen("/core/", "/", 1);
        if known_norm.contains(&stripped) {
            if let Some(o) = originals.iter().find(|o| norm(o) == stripped) {
                return Some(o.clone());
            }
        }
    }
    // 5) FileIndex fallback（治"模型第一次猜路径无清单可补全"）
    if let Some(ws) = crate::agent::workspace::current(session_id) {
        let idx = crate::backbone::file_index::global_index(&ws);
        match idx.lookup(raw) {
            crate::backbone::file_index::LookupResult::Exact(p) => return Some(p),
            crate::backbone::file_index::LookupResult::Ambiguous(list) if list.len() == 1 => {
                return Some(list[0].clone());
            }
            _ => {}
        }
    }
    None
}

/// tools 默认用全局 project_root()=D:\proj 解析与校验，但任务工作区是会话 WS；
fn anchor_to_workspace(path: &str, session_id: &str) -> String {
    let p = path.trim();
    let is_abs = p.len() >= 2 && p.as_bytes()[1] == b':';
    if is_abs || p.is_empty() {
        return path.to_string();
    }
    if let Some(ws) = crate::agent::workspace::current(session_id) {
        if p == "." || p == ".." || p.contains('*') || p.contains('?') {
            return ws;
        }
        return std::path::Path::new(&ws)
            .join(p)
            .to_string_lossy()
            .to_string();
    }
    path.to_string()
}

pub fn prepare_tool_args(
    name: &str,
    args: &Value,
    evidence: &HashMap<String, String>,
    session_id: &str,
    _alloc: &std::sync::Mutex<HashMap<String, usize>>,
) -> Result<Value, String> {
    //（content 本就原样写盘，备份恢复成纯 no-op）。残留 #En 由下方 find_unresolved_placeholder
    let mut resolved = args.clone();
    // ===== 消灭红灯（架构定调）：参数自动归一 =====
    let arg_aliases: &[(&str, &[&str], &str)] = &[
        // modify 的 file 别名：模型写 file_path/path/target 都归一到 file
        ("modify", &["file_path", "path", "target", "file"], "file"),
        // write 的 path 别名：模型写 file/target/file_path 都归一到 path（write 契约是 path）
        ("write", &["file", "target", "file_path", "path"], "path"),
        // audit 的 path 别名
        ("audit", &["target", "dir", "file", "path"], "path"),
    ];
    for (tool, aliases, canon) in arg_aliases {
        if name != *tool {
            continue;
        }
        if resolved.get(*canon).is_none() {
            for a in *aliases {
                if a == canon {
                    continue;
                }
                if let Some(v) = resolved.get(*a) {
                    if v.is_string() {
                        resolved[*canon] = v.clone();
                        break;
                    }
                }
            }
        }
    }
    // read 的 paths 别名特殊处理：模型写 path（单数字符串）/file → 包装成 paths 数组
    if name == "read" && resolved.get("paths").is_none() {
        for a in ["path", "file"] {
            if let Some(v) = resolved.get(a).and_then(|v| v.as_str()) {
                if !v.trim().is_empty() {
                    resolved["paths"] = json!([v]);
                    break;
                }
            }
        }
    }
    // 模型语义明显是"要上下文"，布尔 true 应归一为默认行数而非报错）
    if name == "search" {
        if let Some(v) = resolved.get("context") {
            if let Some(b) = v.as_bool() {
                resolved["context"] = json!(if b { 3 } else { 0 });
                tracing::debug!(
                    from = b,
                    to = if b { 3 } else { 0 },
                    "search.context 布尔归一为整数"
                );
            }
        }
    }
    // modify 缺 file 时 schema 报裸
    if name == "modify" {
        let has_file = resolved
            .get("file")
            .and_then(|v| v.as_str())
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        if !has_file {
            let spec = resolved.get("change_spec").cloned().unwrap_or(json!({}));
            let has_find = spec
                .get("find")
                .and_then(|v| v.as_str())
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false);
            if has_find {
                return Err(
                    "modify.file 缺失——修改必须指明目标文件（绝对路径）。\
                     change_spec.find 已给出，但缺 **file 字段**。"
                        .into(),
                );
            }
        }
    }
    // 工具契约净化（：剥离编排控制字段（depends_on/#NEW/target_step）。
    if let Some(obj) = resolved.as_object_mut() {
        obj.remove("depends_on");
        obj.remove("#NEW");
        obj.remove("target_step");
    }
    // 参数净化层：统一清洗路径/命令字段的脏（引号/转义/尾巴），再走检测——
    sanitize_path_args(&mut resolved);
    // 模型在计划/补计划阶段没读文件时，
    let placeholder_check = |fields: &[&str]| -> Option<String> {
        for f in fields {
            if let Some(v) = resolved.get(*f).and_then(|v| v.as_str()) {
                // command 字段可含中文（echo 审计完成），
                let is_ph = if *f == "command" {
                    looks_like_placeholder_cmd(v)
                } else {
                    looks_like_placeholder(v)
                };
                if is_ph {
                    return Some(format!(
                        "参数 {f} 的值是**占位符描述**「{v}」，不是真实路径/命令，已拒绝执行。\
                         \n真实路径/命令来自工具结果（list/read 的返回）。"
                    ));
                }
            }
            if let Some(arr) = resolved.get(*f).and_then(|v| v.as_array()) {
                for item in arr {
                    if let Some(v) = item.as_str() {
                        if looks_like_placeholder(v) {
                            return Some(format!(
                                "参数 {f} 含**占位符描述**「{v}」，不是真实路径，已拒绝执行。"
                            ));
                        }
                    }
                }
            }
        }
        None
    };
    if let Some(err) = placeholder_check(match name {
        "read" => &["paths", "path", "file"],
        "write" => &["file", "target", "path"],
        // 模型写 {"path": "#E6 定位到的文件"} 时绕过教学拦截，路由到 modify
        "modify" => &["file", "path", "target"],
        "run" => &["command", "target", "path"],
        "list" | "search" | "find_files" => &["path"],
        _ => &[],
    }) {
        return Err(err);
    }
    // 残留占位符检测：resolve 后参数里仍有 #En/#NEW（依赖缺失 / 模型自造），显式检出。
    if let Some(ph) = crate::agent::plan::find_unresolved_placeholder(&resolved) {
        return Err(format!(
            "参数里出现了占位符 `{ph}`，它不是路径。占位符只有一种合法用法：在**内容字段**\
             （find / replace / old / new）里用 `#E<数字>` 引用本会话前序 read 步骤的正文；\
             **路径与命令字段必须写字面值**，例：\"file\": \"D:/proj/server/src/tools/modify.rs\"。\
             （`#NEW` 这种写法不存在，不要使用。）"
        ));
    }
    // Fix 2：相对/默认/通配路径锚定到会话工作区，避免 RELATIVE_PATH 拒绝与搜错目录
    let scalar_path_fields: &[&str] = match name {
        "search" | "find_files" => &["path"],
        // write 契约字段是 **path**
        "write" => &["file", "target", "path"],
        "modify" => &["file"],
        // run 的 cwd/target 显式传**相对路径**
        "run" => &["cwd", "target"],
        // 显式传**相对路径**（如 path="src"）时不锚定 → audit 按 project_root() 拼 → 扫错
        "audit" => &["path"],
        _ => &[],
    };
    for f in scalar_path_fields {
        if let Some(v) = resolved.get(f).and_then(|v| v.as_str()) {
            let a = anchor_to_workspace(v, session_id);
            if a != v {
                resolved[f] = serde_json::json!(a);
            }
        }
    }
    // audit 的 path 是 schema required——
    if name == "audit" {
        let has_path = resolved
            .get("path")
            .and_then(|v| v.as_str())
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        if !has_path {
            if let Some(ws) = crate::agent::workspace::current(session_id) {
                resolved["path"] = serde_json::json!(ws);
            }
            // 仍无（无会话工作区）→ 保持缺失，契约闸门报 MISSING_PARAM 教学兜底
        }
        // audit.scope 未知值确定性归一——模型可见契约不含
        if let Some(s) = resolved.get("scope").and_then(|v| v.as_str()) {
            if !matches!(s, "project" | "dir" | "file") {
                let p = resolved.get("path").and_then(|v| v.as_str()).unwrap_or("");
                let derived = if !p.is_empty() {
                    let path = std::path::Path::new(p);
                    if path.is_file() {
                        "file"
                    } else if path.is_dir() {
                        "dir"
                    } else {
                        "project"
                    }
                } else {
                    "project"
                };
                tracing::debug!(from = s, to = derived, "audit.scope 未知值归一（语义推导）");
                resolved["scope"] = serde_json::json!(derived);
            }
        }
        // 合法枚举值
        if name == "audit" {
            let p = resolved
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let sc = resolved
                .get("scope")
                .and_then(|v| v.as_str())
                .unwrap_or("project")
                .to_string();
            if !p.is_empty() {
                let path = std::path::Path::new(&p);
                if path.is_dir() && sc == "file" {
                    tracing::warn!(from = sc, to = "dir", path = %p, "audit.scope 组合归一：file + 目录 path → dir（模型意图盘点目录+focus 深挖）");
                    resolved["scope"] = serde_json::json!("dir");
                } else if path.is_file() && (sc == "dir" || sc == "project") {
                    tracing::warn!(from = sc, to = "file", path = %p, "audit.scope 组合归一：dir/project + 文件 path → file");
                    resolved["scope"] = serde_json::json!("file");
                }
            }
        }
    }
    // search/find_files/list 的 path 缺省
    if matches!(name, "search" | "find_files" | "list") {
        // 模型按 read 的
        let paths_arr = resolved
            .get("paths")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>())
            .unwrap_or_default();
        let has_path = resolved
            .get("path")
            .and_then(|v| v.as_str())
            .map(|s| !s.trim().is_empty() && s.trim() != ".")
            .unwrap_or(false);
        if !has_path && !paths_arr.is_empty() {
            tracing::debug!(tool = name, paths = ?paths_arr, "paths 复数归一 → path 取首项");
            resolved["path"] = serde_json::json!(paths_arr[0]);
        }
        // search/find_files/list 的 path 缺省
        let has_path2 = resolved
            .get("path")
            .and_then(|v| v.as_str())
            .map(|s| !s.trim().is_empty() && s.trim() != ".")
            .unwrap_or(false);
        if !has_path2 {
            if let Some(ws) = crate::agent::workspace::current(session_id) {
                tracing::debug!(tool = name, ws = %ws, "默认根注入会话工作区（path 缺省/为 . 时）");
                resolved["path"] = serde_json::json!(ws);
            }
        }
    }
    // search.output_mode 未知值 → content（默认行内容）
    if name == "search" {
        if let Some(m) = resolved.get("output_mode").and_then(|v| v.as_str()) {
            if !matches!(m, "content" | "files" | "count") {
                resolved["output_mode"] = serde_json::json!("content");
            }
        }
        if let Some(p) = resolved.get("path").and_then(|v| v.as_str()) {
            let p = p.trim().trim_end_matches(['/', '\\']);
            if !p.is_empty() && !std::path::Path::new(p).exists() {
                if let Some(real) = resolve_known_path(p, session_id) {
                    if real != p {
                        tracing::debug!(from = p, to = %real, "search.path 层级纠正（basename 清单唯一命中）");
                        resolved["path"] = serde_json::json!(real);
                    }
                }
            }
        }
    }
    // 模型多轮后
    if name == "modify" {
        let spec = resolved.get("change_spec").cloned().unwrap_or(json!({}));
        let has_find = spec
            .get("find")
            .and_then(|v| v.as_str())
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        let has_replace = spec
            .get("replace")
            .and_then(|v| v.as_str())
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        let has_desc = spec
            .get("description")
            .and_then(|v| v.as_str())
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        let has_line = spec.get("line").is_some();
        // 兼容旧形态：change_spec 缺失时检查顶层 find/change/content（直传）
        let top_find = resolved
            .get("find")
            .and_then(|v| v.as_str())
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        let top_change = resolved
            .get("change")
            .and_then(|v| v.as_str())
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        let top_content = resolved
            .get("content")
            .and_then(|v| v.as_str())
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        if !(has_find
            || has_replace
            || has_desc
            || has_line
            || top_find
            || top_change
            || top_content)
        {
            return Err(
                "modify 缺少修改内容：change_spec 里 find/replace/description 全空（或顶层 find/change/content 全空）。\
                 请提供：①find（要替换的**原文**，必须来自前序 read 的真实内容）+ replace，\
                 或 ②change 意图描述（如'把 target_path 改成解码 %2f 后校验'），或 ③content 整文件新内容。"
                    .into(),
            );
        }
    }
    if name == "read" {
        // mode 归一兜底（覆盖原子 read 直调）——
        if let Some(m) = resolved.get("mode").and_then(|v| v.as_str()) {
            if !matches!(m, "auto" | "full" | "lines") {
                let has_range =
                    resolved.get("start_line").is_some() || resolved.get("end_line").is_some();
                resolved["mode"] = serde_json::json!(if has_range { "lines" } else { "auto" });
            }
        }
        // Tool Result Grounding（用户契约 ：read 必须带行号——
        resolved["numbered"] = serde_json::json!(true);
        if let Some(arr) = resolved.get("paths").and_then(|v| v.as_array()) {
            let new: Vec<serde_json::Value> = arr
                .iter()
                .map(|v| {
                    if let Some(s) = v.as_str() {
                        // 模型说"读 engine.rs"（文件名/
                        if let Some(real) = resolve_known_path(s, session_id) {
                            if real != s {
                                return serde_json::json!(real);
                            }
                        }
                        let a = anchor_to_workspace(s, session_id);
                        if a != s {
                            return serde_json::json!(a);
                        }
                    }
                    v.clone()
                })
                .collect();
            resolved["paths"] = serde_json::Value::Array(new);
        }
    }
    // paths 健康检查（修复"#E10 读取失败：paths 应为至少 1 项"）
    if name == "read" {
        if let Some(paths) = resolved.get("paths") {
            let bad = match paths {
                serde_json::Value::Array(a) => a.is_empty() || !a.iter().all(|p| p.is_string()),
                _ => true,
            };
            if bad {
                // evidence 无内容 → #En 引用失败直接报错引导，
                return Err(
                    "read.paths 解析后不含有效路径（#En 引用的工具结果为空或失败）。\
                     请改用 list 枚举出真实文件路径后重新引用，或直接用绝对路径。"
                        .into(),
                );
            }
        }
        // audit 结果（全量 files 清单）——read 的每个路径必须在清单中，否则是模型猜测
        if let Some(paths) = resolved.get("paths").and_then(|p| p.as_array()) {
            let norm = |s: &str| s.replace('\\', "/").to_ascii_lowercase();
            // 从 evidence 收集所有 audit 结果的 files 路径（规范化：正斜杠+小写）
            let audit_paths: std::collections::HashSet<String> = evidence
                .values()
                .filter_map(|v| serde_json::from_str::<serde_json::Value>(v).ok())
                .filter(|v| v.get("name").and_then(|n| n.as_str()) == Some("audit"))
                .filter_map(|v| {
                    let text = v.pointer("/content/0/text").and_then(|t| t.as_str())?;
                    let inner: serde_json::Value = serde_json::from_str(text).ok()?;
                    let files = inner.pointer("/data/files")?.as_array()?;
                    Some(
                        files
                            .iter()
                            .filter_map(|f| f.get("path").and_then(|p| p.as_str()).map(norm))
                            .collect::<Vec<_>>(),
                    )
                })
                .flatten()
                .collect();
            // 修复任务兜底（设计决定："长时间任务路径错一次就堵住"）
            let mut known_paths = audit_paths;
            if known_paths.is_empty() {
                if let Some(ws) = crate::agent::workspace::current(session_id) {
                    let cache = crate::agent::project_profile::profile_cache_file(&ws);
                    if let Ok(content) = std::fs::read_to_string(&cache) {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                            if let Some(files) = v.get("files").and_then(|f| f.as_array()) {
                                known_paths
                                    .extend(files.iter().filter_map(|f| f.as_str().map(norm)));
                            }
                        }
                    }
                }
            }
            if !known_paths.is_empty() {
                let unknown: Vec<String> = paths
                    .iter()
                    .filter_map(|p| p.as_str())
                    .filter(|p| !known_paths.contains(&norm(p)))
                    // 文件真实存在即放行——任务描述（instruction）
                    .filter(|p| !std::path::Path::new(p).is_file())
                    .map(|s| s.to_string())
                    .collect();
                if !unknown.is_empty() {
                    // 模型猜错目录（tests/batch.jsonl，
                    let mut hints: Vec<String> = Vec::new();
                    for p in &unknown {
                        let base = std::path::Path::new(p)
                            .file_name()
                            .and_then(|b| b.to_str())
                            .map(|b| b.to_ascii_lowercase())
                            .unwrap_or_default();
                        if !base.is_empty() {
                            let matches: Vec<&String> = known_paths
                                .iter()
                                .filter(|k| {
                                    std::path::Path::new(k)
                                        .file_name()
                                        .and_then(|b| b.to_str())
                                        .map(|b| b.to_ascii_lowercase() == base)
                                        .unwrap_or(false)
                                })
                                .collect();
                            if !matches.is_empty() {
                                let joined = matches
                                    .iter()
                                    .map(|s| s.as_str())
                                    .collect::<Vec<_>>()
                                    .join("; ");
                                hints.push(format!("「{p}」→ 清单中有同名文件: {joined}"));
                            }
                        }
                    }
                    let hint_block = if hints.is_empty() {
                        format!(
                            "项目真实文件见画像 🔑 文件清单（候选示例: {}）。",
                            {
                                let mut sample: Vec<String> =
                                    known_paths.iter().take(15).cloned().collect();
                                sample.sort();
                                sample.join("; ")
                            }
                        )
                    } else {
                        format!("按文件名匹配到真实路径：\n{}", hints.join("\n"))
                    };
                    return Err(format!(
                        "read.paths 含不在项目文件清单中的路径（疑似模型猜测）: {}。\n{hint_block}\
                         \n请用清单中的真实路径（basename 匹配已给出），或先用 list 枚举确认路径后 #En 引用。",
                        unknown.join(", "),
                    ));
                }
            }
        }
        if name == "read" {
            let req_paths: Vec<&str> = resolved
                .get("paths")
                .and_then(|x| x.as_array())
                .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
                .unwrap_or_default();
            let known_all: Vec<String> = crate::agent::workspace::known_files(session_id);
            let dir_hits: Vec<String> = req_paths
                .iter()
                .filter(|p| {
                    std::path::Path::new(p).is_dir()
                        && !std::path::Path::new(p).is_file()
                        && !p.trim_end_matches(['/', '\\']).is_empty()
                })
                .map(|s| s.to_string())
                .collect();
            if !dir_hits.is_empty() {
                let mut inside: Vec<String> = known_all
                    .iter()
                    .filter(|k| {
                        std::path::Path::new(k).parent().map(|d| d == std::path::Path::new(&dir_hits[0])).unwrap_or(false)
                    })
                    .cloned()
                    .collect();
                inside.sort();
                inside.truncate(8);
                let hint = if inside.is_empty() {
                    "\n（该目录内尚无已登记文件——先用 list 枚举该目录确认文件名，或按编译报错里的相对路径 + 项目根拼绝对路径）".to_string()
                } else {
                    format!("\n该目录下本会话已知的真实文件（直接挑一个改读绝对路径）：\n  {}", inside.join("\n  "))
                };
                return Err(format!(
                    "read 路径是**目录**不是文件: {}。目录本身不能 read。{}",
                    dir_hits.join("、"),
                    hint,
                ));
            }
        }
    }
    // write content 占位符拦截（write 占位符复制 bug，方案二：Worker 执行层护栏）
    if name == "write" {
        // 模型写临时脚本/新文件时偶发只给
        let has_path = args
            .get("path")
            .and_then(|v| v.as_str())
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false)
            || args
                .get("file")
                .and_then(|v| v.as_str())
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false);
        if !has_path {
            return Err(
                "write.path 缺失——创建/写入文件必须带 **path（绝对路径）**。\
                例：{\"path\":\"D:/proj/app/schema_audit.py\",\"content\":\"...完整文件内容...\"}。\
                临时验证脚本也请写明确路径（如 workspace 下的 _verify_tmp.py），不要省略 path。\
                若意图是**修改已有文件**，用 modify（file + find/replace 或 change）。"
                    .into(),
            );
        }
        // 模型在计划/补计划阶段没读文件时，
        let content_missing_or_empty = match args.get("content") {
            None => true,
            Some(Value::String(v)) => v.trim().is_empty(),
            _ => false,
        };
        if content_missing_or_empty {
            return Err(crate::tools::contract::err_text(
                &crate::tools::contract::ToolError::empty_content("write.content"),
            ));
        }

        // 原 content #En 检测（whole_placeholder_ref/
    }

    // Tool Result Grounding：write 的目标路径必须非空
    let target_fields: &[&str] = match name {
        "write" => &["target", "file"],
        _ => &[],
    };
    for f in target_fields {
        if let Some(v) = resolved.get(f).and_then(|v| v.as_str()) {
            let t = v.trim();
            if t.is_empty() {
                return Err(format!(
                    "{name}.{f} 为空（#En 引用的工具结果为空或失败）。\
                     请先 read 或 list 枚举出真实文件路径后再引用。"
                ));
            }
        }
    }

    // session 工作区注入（审计 A1）：run 缺省 cwd 用本 session 的工作区，不读全局
    let is_run_like = name == "run";
    if is_run_like {
        if let Some(c) = resolved
            .get("cwd")
            .or_else(|| resolved.get("target"))
            .and_then(|v| v.as_str())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            crate::agent::workspace::note_cwd(session_id, c);
        }
    }
    if is_run_like
        && resolved
            .get("cwd")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().is_empty())
            .unwrap_or(true)
    {
        if let Some(ws) = crate::agent::workspace::current(session_id) {
            if std::path::Path::new(&ws).is_dir() {
                resolved["cwd"] = serde_json::json!(ws);
            } else {
                tracing::warn!(session = %session_id, ws = %ws, "会话工作区目录已不存在，跳过 cwd 注入（模型需重选工作区）");
            }
        }
    }
    // 模型把脚本路径放 args 数组
    if is_run_like {
        for bad in ["args", "arguments", "argv", "params"] {
            if let Some(arr) = resolved.get(bad).and_then(|v| v.as_array()) {
                let cmd = resolved
                    .get("command")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if cmd.is_empty() {
                    return Err(format!(
                        "run 的 command 为空，且 {bad} 数组也不能替代 command——请直接给完整命令字符串，\
                         \n如 {{\"command\": \"python D:/x/tests/verifier.py -v\"}}。"
                    ));
                }
                // 字形规则归命令部门（tools/cmd_translate）：装配层只把 args 拆成 token，
                let mut tokens: Vec<String> = Vec::with_capacity(arr.len());
                for item in arr {
                    match item.as_str() {
                        Some(a) if !a.trim().is_empty() => tokens.push(a.trim().to_string()),
                        _ => {
                            return Err(format!(
                                "run 的 {bad} 数组含非字符串元素，无法自动合并进 command。\
                                 \n请直接给完整命令字符串，如 {{\"command\": \"python D:/x/tests/verifier.py -v\"}}。"
                            ));
                        }
                    }
                }
                let merged = crate::tools::cmd_translate::merge_argv_tokens(&cmd, &tokens);
                tracing::debug!(session = session_id, from = %cmd, to = %merged, "run args 数组已归一化合并进 command");
                resolved["command"] = serde_json::json!(merged);

                if let Some(obj) = resolved.as_object_mut() {
                    obj.remove(bad);
                }
            }
        }
    }
    // 命令斜杠归一下沉到**执行层**（cmd_tools::run）：只有那里知道这条命令最终进哪个壳。
    Ok(resolved)
}

#[cfg(test)]
#[path = "prepare_args_tests.rs"]
mod prepare_args_tests;
