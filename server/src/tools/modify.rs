//! 意图式修改：模型给 file + change_spec（find/replace），后端编译为 read→edit→verify 并验证

use crate::mcp::registry::BuiltinTool;
use crate::tools::contract::validate;
use crate::tools::fs_common::extract_structure;
use crate::tools::fs_write::{EditTool, WriteTool};
use crate::tools::fs_common::{brace_diagnose, brace_guard_should_block};
use async_trait::async_trait;
use serde_json::{json, Value};

// ===== 文件索引缓存（用户方案 #1 落地：函数名→行号映射）=====
use std::sync::Mutex;

struct FileIndexEntry {
    modified: std::time::SystemTime,
    structure: Vec<(usize, String, String)>,
    content_len: usize,
}

static FILE_INDEX: Mutex<Option<std::collections::HashMap<String, FileIndexEntry>>> =
    Mutex::new(None);

/// 取文件 structure（带索引缓存）：mtime 未变 → 复用缓存；变了/首次 → 读全文重扫。
pub(crate) fn indexed_structure(file: &str) -> Option<Vec<(usize, String, String)>> {
    let path = std::path::Path::new(file);
    let meta = path.metadata().ok()?;
    let modified = meta.modified().ok()?;
    let content_len = meta.len() as usize;
    let mut guard = FILE_INDEX.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(map) = guard.as_mut() {
        if let Some(entry) = map.get(file) {
            if entry.modified == modified && entry.content_len == content_len {
                return Some(entry.structure.clone());
            }
        }
    }
    // 缓存未命中/失效 → 读全文 + 提取 structure，写入缓存
    let content = std::fs::read_to_string(file).ok()?;
    let structure = extract_structure(&content);
    if structure.is_empty() {
        return None;
    }
    let entry = FileIndexEntry {
        modified,
        structure: structure.clone(),
        content_len,
    };
    if guard.is_none() {
        *guard = Some(std::collections::HashMap::new());
    }
    if let Some(map) = guard.as_mut() {
        map.insert(file.to_string(), entry);
    }
    Some(structure)
}

/// 意图自动定位（设计决定："后端先做确定性脏活）
pub(crate) fn locate_intent_target(
    file: &str,
    desc: &str,
) -> Option<(String, String, usize, String)> {
    let content = std::fs::read_to_string(file).ok()?;
    // ===== 组合拳 P2 补充：行号定位优先 =====
    if let Some((start, end)) = extract_line_range(desc) {
        let total = content.lines().count();
        if start >= 1 && start <= total && end >= start {
            let end = end.min(total);
            // （FIND_AMBIGUOUS 根治）：行号区间前后各扩展 8 行上下文——
            let ctx = 8;
            let from = start.saturating_sub(ctx).max(1);
            let to = (end + ctx).min(total);
            let snippet: Vec<&str> = content.lines().skip(from - 1).take(to - from + 1).collect();
            return Some((
                "lines".to_string(),
                format!("L{start}-{end}"),
                start,
                snippet.join("\n"),
            ));
        }
    }
    // （用户方案 #1 落地）：用索引缓存（mtime 失效），不再每次读全文重扫
    let structure = match indexed_structure(file) {
        Some(s) => s,
        None => return None,
    };
    // 候选：desc 里出现过的结构名（忽略大小写、忽略引号/点号/括号装饰）
    let desc_lower = desc.to_ascii_lowercase();
    // 方法/类名匹配：名字作为子串出现（如 "WebSocketRoute.__init__" 含 "__init__"）
    let mut hits: Vec<&(usize, String, String)> = structure
        .iter()
        .filter(|(_, _, nm)| {
            let n = nm.to_ascii_lowercase();
            !n.is_empty() && (desc_lower.contains(&n) || n.contains(desc_lower.trim()))
        })
        .collect();
    if hits.is_empty() {
        return None;
    }
    // 优先"更具体"（名字更长 = 匹配更精确；方法名通常比类名短但更靠后=更内层）
    hits.sort_by_key(|(ln, _, nm)| (std::cmp::Reverse(nm.len()), *ln));
    let (line, kind, name) = hits[0].clone();
    // 代码段：定义行 → 下一个定义行（-1）或 +40 行封顶
    let next_line = structure
        .iter()
        .map(|(l, _, _)| *l)
        .filter(|l| *l > line)
        .min()
        .unwrap_or(line + 40);
    let end = (next_line - 1).min(line + 40);
    let snippet: Vec<&str> = content
        .lines()
        .skip(line - 1)
        .take(end - line + 1)
        .collect();
    Some((kind, name, line, snippet.join("\n")))
}

/// 组合拳 P2：从意图描述提取行号区间。
fn extract_line_range(desc: &str) -> Option<(usize, usize)> {
    let bytes: Vec<char> = desc.chars().collect();
    let mut nums: Vec<usize> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        // 可选 l/L 前缀（L307 / l307 / line 307）
        let mut j = i;
        if bytes[j] == 'l' || bytes[j] == 'L' {
            j += 1;
        }
        // 收集数字
        let start_digit = j;
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            j += 1;
        }
        if j > start_digit {
            if let Ok(n) = bytes[start_digit..j]
                .iter()
                .collect::<String>()
                .parse::<usize>()
            {
                if n >= 1 && n <= 10_000_000 {
                    nums.push(n);
                    // 区间分隔符（- ~ 至 到 行）后可能跟第二个数字
                    if nums.len() < 2 {
                        let mut k = j;
                        // 跳过空白
                        while k < bytes.len() && bytes[k].is_whitespace() {
                            k += 1;
                        }
                        if k < bytes.len() && (bytes[k] == '-' || bytes[k] == '~') {
                            let mut m = k + 1;
                            while m < bytes.len() && bytes[m].is_whitespace() {
                                m += 1;
                            }
                            let sd = m;
                            while m < bytes.len() && bytes[m].is_ascii_digit() {
                                m += 1;
                            }
                            if m > sd {
                                if let Ok(n2) =
                                    bytes[sd..m].iter().collect::<String>().parse::<usize>()
                                {
                                    if n2 >= 1 && n2 <= 10_000_000 {
                                        nums.push(n2);
                                    }
                                }
                            }
                        }
                    }
                    if nums.len() >= 2 {
                        break;
                    }
                }
            }
        }
        if j == i {
            i += 1;
        } else {
            i = j;
        }
    }
    match nums.as_slice() {
        [a, b] if b >= a => Some((*a, *b)),
        [a] => Some((*a, *a)),
        _ => None,
    }
}

/// 共享编译器：change_spec → (具体工具名, 工具参数)。
pub fn compile_change_spec(file: &str, cs: &Value) -> Result<(String, Value), String> {
    let mode = cs
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("block")
        .to_lowercase();
    let replace: String = match cs.get("replace").filter(|v| !v.is_null()).and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => ["replace2", "new", "content", "text", "insert", "append", "add"]
            .iter()
            .find_map(|k| {
                cs.get(*k)
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .filter(|s| !s.trim().is_empty())
            })
            .unwrap_or_default(),
    };
    // （意图模式·教学引导）：change_spec 只有 description（意图）没有定位信息
    let has_find = cs
        .get("find")
        .and_then(|v| v.as_str())
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    let has_line = cs.get("line").is_some();
    if !has_find && !has_line && replace.trim().is_empty() {
        let desc = cs
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        // （边界）：description 也是空 → 文案不能"收到意图「」"——指向"至少给意图或定位"
        if desc.is_empty() {
            return Err(
                "modify 的 change_spec 为空：既没有**定位信息**（find/line/replace），也没有**意图描述**（description）。\n\
                 后端无法凭空知道要改哪个文件哪一段。\n\
                 正确姿势：至少给一项——\n\
                 ① description：说明想改成什么（如「把 target_path 改成先 unquote 再校验」）；\n\
                 ② 或直接给定位：block 的 find+replace / line 的行号+新内容 / write 的全新全文。\n\
                 建议：先 read 目标文件，基于真实内容填 find 或 line。"
                    .to_string(),
            );
        }
        // （意图自动定位注入）：纯 change 意图无定位时，后端先尝试自动定位
        if let Some((kind, name, line, snippet)) = locate_intent_target(file, desc) {
            // 闭环审查修复 #3：snippet 从 1200 提到 4000——模型生成 find/replace
            let clipped: String = snippet.chars().take(4000).collect();
            return Err(crate::tools::contract::err_text(
                &crate::tools::contract::ToolError::modify_intent_needs_anchor(
                    desc, &kind, &name, line, &clipped,
                ),
            ));
        }
        // （覆写意图识别）：模型想"整文件覆写/重新生成"（如 analysis.json 报告、
        let desc_lower = desc.to_lowercase();
        let rewrite_hint = [
            "覆写",
            "整文件",
            "全部重写",
            "重新生成",
            "overwrite",
            "rewrite whole",
            "regenerate",
        ]
        .iter()
        .any(|k| desc_lower.contains(k));
        if rewrite_hint {
            return Err(crate::tools::contract::err_text(
                &crate::tools::contract::ToolError::modify_intent_whole_file(desc, &file),
            ));
        }
            return Err(crate::tools::contract::err_text(
                &crate::tools::contract::ToolError::modify_intent_locate_failed(desc, file.is_empty()),
            ));
    }
    match mode.as_str() {
        "line" | "single" => {
            let line = cs.get("line").and_then(|v| v.as_u64()).ok_or_else(|| {
                let got = cs
                    .get("line")
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "未提供（change_spec 里没有 line 键）".into());
                format!(
                    "modify mode=line 需要 line（目标行号，整数）+ replace（新行内容）；old 省略后端自动取该行原文。当前收到 line = {got}"
                )
            })?;
            Ok((
                "edit".into(),
                json!({
                    "file": file,
                    "replacements": [{"line": line, "new": replace}],
                }),
            ))
        }
        "write" => Ok(("write".into(), json!({"path": file, "content": replace}))),
        _ => {
            // block（默认）：find + replace，全文唯一匹配
            let find = cs.get("find").and_then(|v| v.as_str())
                .ok_or_else(|| "modify block 模式需要 find（要替换的文本片段）+ replace（替换后的内容）；或改 mode=line（行号替换）".to_string())?;
            // 追加正道：insert_after = 在 find 匹配原文之后插入的内容（模型表达"追加"的一等字段；
            let insert_after = cs.get("insert_after").and_then(|v| v.as_str()).filter(|s| !s.is_empty());
            let replace_owned;
            let replace: &str = if replace.is_empty() {
                match insert_after {
                    Some(ins) => {
                        // 语义是"在 find 那一段**之后另起一行**插入"——find 末尾若无换行符，
                        let sep = if find.ends_with('\n') { "" } else { "\n" };
                        replace_owned = format!("{find}{sep}{ins}");
                        replace_owned.as_str()
                    }
                    None => replace.as_str(),
                }
            } else {
                replace.as_str()
            };
            if find.trim().is_empty() || find.trim().eq_ignore_ascii_case("placeholder") {
                return Err(crate::tools::contract::err_text(
                    &crate::tools::contract::ToolError::modify_find_empty(),
                ));
            }
            Ok((
                "edit".into(),
                json!({
                    "file": file,
                    "replacements": [{"find": find, "replace": replace}],
                }),
            ))
        }
    }
}

/// 内容验收核心（三处合一：verify 工具的 run_intent_acceptance 与
pub fn acceptance_check(content: &str, find: &str, replace: &str) -> (usize, usize, bool) {
    // find 为空视为"已消失"（line 模式无 find，仅验 replace 出现）；matches("") 会误计非零
    let find_count = if find.is_empty() {
        0
    } else {
        content.matches(find).count()
    };
    let replace_count = if replace.is_empty() {
        0
    } else {
        content.matches(replace).count()
    };
    // （机制修复）：追加型修改（replace 含 find——如"插入代码块且保留 find 原文"）
    let matched = if replace.is_empty() {
        find_count == 0
    } else if !find.is_empty() && replace.contains(find) {
        replace_count >= 1
    } else {
        replace_count >= 1 && find_count == 0
    };
    (find_count, replace_count, matched)
}

/// 校验未通过时，把**文件里那一带的实际内容**（带行号）揪出来。
pub fn nearest_actual(content: &str, replace: &str) -> Option<String> {
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return None;
    }
    let anchor = replace.lines().find(|l| !l.trim().is_empty())?.trim().to_string();
    if anchor.is_empty() {
        return None;
    }
    let key: String = anchor.chars().take(40).collect();
    let hit = lines
        .iter()
        .position(|l| l.trim() == anchor)
        .or_else(|| lines.iter().position(|l| l.contains(&key)))?;
    let lo = hit.saturating_sub(3);
    let hi = (hit + 4).min(lines.len());
    Some(
        (lo..hi)
            .map(|n| format!("{}: {}", n + 1, lines[n]))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

/// 从**编译后**的工具参数反推"校验用的 change_spec"。
fn verify_spec_from(tool_args: &Value) -> Value {
    let mut m = serde_json::Map::new();
    match tool_args
        .get("replacements")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
    {
        Some(rep) => {
            if let Some(f) = rep.get("find") {
                m.insert("find".to_string(), f.clone());
            }
            // block 模式用 replace、line 模式用 new —— 都落到 replace（verify 只认它）
            if let Some(rp) = rep.get("replace").or_else(|| rep.get("new")) {
                m.insert("replace".to_string(), rp.clone());
            }
        }
        None => {
            // write 模式：整文件覆写
            if let Some(c) = tool_args.get("content") {
                m.insert("replace".to_string(), c.clone());
                m.insert("mode".to_string(), Value::String("write".into()));
            }
        }
    }
    Value::Object(m)
}

/// 读回验证：执行后检查修改是否落盘（find 消失 / replace 出现 / 全文一致）。
async fn verify_applied(file: &str, cs: &Value) -> Value {
    let mode = cs
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("block")
        .to_lowercase();
    let content = tokio::fs::read_to_string(file).await.unwrap_or_default();
    if mode == "write" {
        let expect = cs.get("replace").and_then(|v| v.as_str()).unwrap_or("");
        json!({
            "mode": "write",
            "verified": content == expect,
            "note": if content == expect { "整文件已覆写为目标内容" } else { "整文件内容与目标不一致" },
        })
    } else {
        let find = cs.get("find").and_then(|v| v.as_str()).unwrap_or("");
    let replace: String = match cs.get("replace").filter(|v| !v.is_null()).and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => ["replace2", "new", "content", "text", "insert", "append", "add"]
            .iter()
            .find_map(|k| {
                cs.get(*k)
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .filter(|s| !s.trim().is_empty())
            })
            .unwrap_or_default(),
    };
        let norm = |s: &str| s.replace("\r\n", "\n").replace('\r', "\n");
        let (find_count, replace_count, matched) =
            acceptance_check(&norm(&content), &norm(find), &norm(&replace));
        let actual = if matched {
            None
        } else {
            nearest_actual(&content, &replace)
        };
        json!({
            "mode": mode,
            "verified": matched,
            "find_gone": find_count == 0,
            "replace_present": replace_count >= 1,
            "actual": actual,
            "note": if matched { "find 已消失、replace 已出现" } else { "内容校验未通过（见 find_gone/replace_present）" },
        })
    }
}

/// 意图式修改工具（模型只说改什么，后端编译执行+验证）
pub struct ModifyTool;

#[async_trait]
impl BuiltinTool for ModifyTool {
    fn name(&self) -> &'static str {
        "modify"
    }

    fn description(&self) -> &'static str {
        include_str!("../../prompts/tools/modify.md")
    }

    fn input_schema(&self) -> Value {
        json!({
            "type":"object",
            "properties":{
                "file":{"type":"string","description":"文件绝对路径（可用 #En 引用前序 read/find_files 结果）"},
                "change_spec":{"type":"object","properties":{
                    "description":{"type":"string","description":"[可选] 意图描述（说明想改成什么，便于理解；**不能替代定位**——模型义务见 find/line/replace）"},
                    "mode":{"type":"string","enum":["block","line","write"],"default":"block","description":"[自动] 默认 block；block=find/replace 结构化改写 / line=行号替换 / write=整文件覆写"},
                    "find":{"type":"string","description":"[必须]（block 模式）要被替换的原文片段（可多行，全文唯一匹配；**必须来自 read 真实内容**，严禁编造）"},
                    "replace":{"description":"[必须·永不可省/null/占位——可省略的只有 line 模式的 old] 替换后的内容（block=新片段 / line=新行 / write=新全文；**必须与 find 不同**）。多行文本换行写 \\n；转义反复失败改用 write 整文件覆写"},
                    "insert_after":{"type":"string","description":"[可选·追加正道] 在 find 匹配的原文**之后插入**此内容（find+insert_after 自动合并为 replace）——追加/新增场景用这个，不要把 replace 留空或写 null"},
                    "line":{"type":"integer","minimum":1,"description":"[必须]（line 模式）目标行号（从 1 开始；old 由后端自动取该行原文）"}
                },"required":[],"additionalProperties":true}
            },
            "required":["file","change_spec"],
            "additionalProperties":true
        })
    }

    fn annotations(&self) -> Value {
        json!({"read_only": false, "destructive": true, "idempotent": false})
    }

    fn output_schema(&self) -> Option<Value> {
        // （契约基础层）：声明 modify 输出契约，registry 强制校验——
        Some(json!({
            "type": "object", "additionalProperties": false,
            "properties": {
                "ok": {"type": "boolean"},
                "kind": {"type": "string"},
                "data": {"type": "object", "additionalProperties": false, "properties": {
                    "file": {"type": "string"},
                    "mode": {"type": "string"},
                    "changed": {"type": "integer"},
                    "verified": {"type": "object"},
                    "changes": {}
                }},
                "warnings": {"type": "array"},
                "error": {}
            }
        }))
    }

    async fn run(&self, raw_args: Value) -> Result<String, String> {
        let args = validate(&self.input_schema(), &raw_args)
            .map_err(|e| crate::tools::contract::err_text(&e))?;
        let file_raw = args["file"].as_str().unwrap();
        // 占位符教学（对齐 prepare_args）：file 是"待定位的源码文件"类占位描述
        if crate::path::looks_like_placeholder(file_raw) {
            return Err(crate::tools::contract::err_text(
                &crate::tools::contract::ToolError::modify_file_placeholder(),
            ));
        }
        // 兜底路径归一：相对路径基于项目根（正式路径走计划期 expand_intents 的会话级归一）
        let file = {
            let p = std::path::Path::new(file_raw);
            if p.is_absolute() {
                file_raw.to_string()
            } else {
                crate::tools::fs_read::project_root()
                    .join(file_raw)
                    .to_string_lossy()
                    .to_string()
            }
        };
        let cs = args.get("change_spec").cloned().unwrap_or(json!({}));
        // ══ 闸门：change_spec 的「新内容」位有没有填实 ═════════════════════════
        {
            let raw_replace_present = cs.get("replace").is_some();
            let replace_is_null = cs.get("replace").map(|v| v.is_null()).unwrap_or(false);
            let replace_probe: String = match cs
                .get("replace")
                .filter(|v| !v.is_null())
                .and_then(|v| v.as_str())
            {
                Some(s) => s.to_string(),
                // 别名兜底：与 compile_change_spec 同一张表，漏一个就会出现裂缝
                None => ["replace2", "new", "content", "text", "insert", "append", "add"]
                    .iter()
                    .find_map(|k| {
                        cs.get(*k)
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.trim().is_empty())
                            .map(|s| s.to_string())
                    })
                    .unwrap_or_default(),
            };
            // 唯一合法的"不给 replace"用法：find + insert_after（追加场景）
            let has_after = cs
                .get("insert_after")
                .and_then(|v| v.as_str())
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false);
            if replace_probe.trim().is_empty() && !has_after {
                let got = if !raw_replace_present {
                    "change_spec 里**没有 replace 键**"
                } else if replace_is_null {
                    "replace 的值是 **null**"
                } else {
                    "replace 是**空字符串**（或只含空白）"
                };
                return Err(crate::tools::contract::err_text(
                    &crate::tools::contract::ToolError::modify_param_unfilled("replace", got),
                ));
            }
        }
        // 编译：change_spec → 具体工具调用（mode=block/line/write）
        let (tool_name, mut tool_args) = match compile_change_spec(&file, &cs) {
            Ok(c) => c,
            Err(e) => {
                // （信任协议 v2 · 兜底补全）：教学错误附上目标文件真实内容——
                let preview: String = tokio::fs::read_to_string(&file)
                    .await
                    .map(|c| c.chars().take(1500).collect())
                    .unwrap_or_default();
                if preview.trim().is_empty() {
                    return Err(e);
                }
                return Err(crate::tools::contract::err_text(
                    &crate::tools::contract::ToolError::with_file_preview(&e, &file, &preview, 1500),
                ));
            }
        };
        // 校验用的 spec 要在这里先算好：`tool_args` 随后会被 move 进执行调用。
        let verify_cs = verify_spec_from(&tool_args);
        // 转发人工确认令牌（executor 在敏感写入确认后签发）给底层 edit/write，使其凭 token 放行。
        if let Some(tok) = raw_args
            .get("__real_self_edit_token")
            .and_then(|v| v.as_str())
        {
            tool_args["__real_self_edit_token"] = json!(tok);
        }
        if let Some(sid) = raw_args.get("__session").and_then(|v| v.as_str()) {
            tool_args["__session"] = json!(sid);
        }

        // 执行（读→改由 EditTool/WriteTool 完成）
        let exec_result = match tool_name.as_str() {
            "edit" => EditTool.run(tool_args).await,
            "write" => WriteTool.run(tool_args).await,
            _ => return Err(format!("modify 编译器产出未知工具 {tool_name}")),
        };
        let exec_out: Value = match exec_result {
            Ok(s) => serde_json::from_str(&s).unwrap_or(json!({"ok": false, "error": s})),
            Err(e) => {
                // 失败：返回带文件真实内容的引导错误（与 REPLACEMENTS_REQUIRED 同构）
                let content = tokio::fs::read_to_string(&file)
                    .await
                    .unwrap_or_default();
                // ── 内层码透传：三类错因各自带着自己的码出去 ──────────────
                let inner = crate::tools::contract::inner_code(&e);
                let out_code: &'static str = match inner.as_deref() {
                    Some("NOT_FOUND") => "MODIFY_PATH_NOT_FOUND",
                    Some("FIND_NOT_FOUND") => "MODIFY_ANCHOR_NOT_FOUND",
                    Some("FIND_AMBIGUOUS") => "MODIFY_ANCHOR_AMBIGUOUS",
                    Some("EDIT_NEEDS_READ") => "EDIT_NEEDS_READ",
                    // 没解出内层码时**不猜**：落到一个中性码，别冒充任何一类
                    _ => "MODIFY_EXEC_FAILED",
                };
                let preview: String = content.chars().take(2000).collect();
                return Err(crate::tools::contract::err_text(
                    &crate::tools::contract::ToolError::with_file_preview_as(
                        out_code, &e, &file, &preview, 2000,
                    ),
                ));
            }
        };
        // （机制 bug 根治）：fs_write EditTool 返回的字段是 `data.changes`（数组），
        let changed = exec_out
            .pointer("/data/changes")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        // 读回验证（编译器最后一步）：**用编译后的 find/replace**，与执行共用同一真值源
        let verified = verify_applied(&file, &verify_cs).await;
        // ===== 失败一次就回来=====
        let mode = cs
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("block")
            .to_lowercase();
        if changed == 0 && mode == "block" {
            let head = tokio::fs::read_to_string(&file)
                .await
                .map(|c| c.chars().take(2000).collect::<String>())
                .unwrap_or_default();
            return Err(crate::tools::contract::err_text(
                &crate::tools::contract::ToolError::modify_no_change(&file, &head),
            ));
        }
        // ===== 结构护栏：verified=false 或 .rs 括号不平衡 → 回滚 + 报错 =====
        let verified_ok = verified
            .get("verified")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        if !verified_ok && mode == "block" {
            if let Some(bak) = exec_out
                .pointer("/data/backup_path")
                .and_then(|v| v.as_str())
            {
                let _ = tokio::fs::copy(bak, &file).await;
            }
            return Err(crate::tools::contract::err_text(
                &crate::tools::contract::ToolError::modify_verify_fail(
                    verified.get("find_gone").and_then(|v| v.as_bool()).unwrap_or(false),
                    verified.get("replace_present").and_then(|v| v.as_bool()).unwrap_or(false),
                    verified.get("actual").and_then(|v| v.as_str()),
                ),
            ));
        }
        if file.ends_with(".rs") && changed > 0 {
            let after = tokio::fs::read_to_string(&file).await.unwrap_or_default();
            // 修改前原文（备份）：护栏只在"这次改动把平衡改坏"时拦（brace_guard_should_block）——
            let before = match exec_out
                .pointer("/data/backup_path")
                .and_then(|v| v.as_str())
            {
                Some(bak) => tokio::fs::read_to_string(bak).await.unwrap_or_default(),
                None => String::new(),
            };
            if !after.is_empty() && brace_guard_should_block(&before, &after) {
                if let Some(bak) = exec_out
                    .pointer("/data/backup_path")
                    .and_then(|v| v.as_str())
                {
                    let _ = tokio::fs::copy(bak, &file).await;
                }
                // 教学文案与错误码归契约部门（contract.rs），工具层只判拦不拦
                return Err(crate::tools::contract::err_text(
                    &crate::tools::contract::ToolError::brace_unbalanced(&file, &brace_diagnose(&after)),
                ));
            }
        }
        Ok(json!({
            "ok": true,
            "kind": "modify_result",
            "data": {
                "file": file,
                "mode": cs.get("mode").and_then(|v| v.as_str()).unwrap_or("block"),
                "changed": changed,
                "verified": verified,
                "changes": exec_out.pointer("/data/changes").cloned().unwrap_or(Value::Null),
            },
            "warnings": [], "error": null,
        })
        .to_string())
    }
}

#[cfg(test)]
#[path = "modify_tests.rs"]
mod modify_tests;
