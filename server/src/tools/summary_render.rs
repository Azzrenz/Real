//! 工具结果 → 可读摘要（完整版）

use serde_json::Value;

const MAX_SUMMARY: usize = 20000;

/// 解析工具结果（信封 content[0].text 是工具返回 JSON 或纯文本）
fn parse_envelope_text(joined: &str) -> (Value, String) {
    if let Ok(v) = serde_json::from_str::<Value>(joined) {
        (v, String::new())
    } else {
        (Value::Null, joined.to_string())
    }
}

/// 主入口：按工具类型整理 result_summary。
pub fn clean(name: &str, env_text: &str) -> String {
    let (v, raw_text) = parse_envelope_text(env_text);
    // 纯文本（非 JSON，如 modify 失败诊断/finish 消息）→ 原样
    if v.is_null() {
        return truncate(&raw_text, MAX_SUMMARY);
    }
    // 信封层：content[0].text 内可能再包一层工具 JSON（read_result 等被 registry 包过一次）
    let payload = unwrap_envelope(&v);
    if let Some(err_text) = extract_error(&payload) {
        return err_text;
    }
    let rendered = render(name, &payload);
    if !rendered.is_empty() {
        return rendered;
    }
    // 兜底：data 里有个路径/文件名之类也想给
    let data = payload.get("data").cloned().unwrap_or(payload);
    if let Some(p) = data.get("path").and_then(|x| x.as_str()) {
        return truncate(&format!("路径: {p}"), MAX_SUMMARY);
    }
    truncate(env_text, MAX_SUMMARY)
}

/// 剥信封层：ToolEnvelope → 其 content[0].text 可能是内层 JSON
fn unwrap_envelope(v: &Value) -> Value {
    if let Some(arr) = v.get("content").and_then(|c| c.as_array()) {
        if let Some(first) = arr.first() {
            if let Some(t) = first.get("text").and_then(|x| x.as_str()) {
                if let Ok(inner) = serde_json::from_str::<Value>(t) {
                    return inner;
                }
            }
        }
    }
    v.clone()
}

/// 提取失败错误（信封或内层 error/meta.error_code/is_error）
fn extract_error(v: &Value) -> Option<String> {
    let is_err = v.get("is_error").and_then(|x| x.as_bool()).unwrap_or(false)
        || v.get("ok").and_then(|x| x.as_bool()) == Some(false)
        || v.get("error").is_some_and(|x| !x.is_null());
    if !is_err {
        return None;
    }
    // 完整错误文本（content[0].text 或 error.message）
    let mut msg = if let Some(text) = v
        .get("content")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
        .and_then(|p| p.get("text"))
        .and_then(|x| x.as_str())
    {
        text.to_string()
    } else if let Some(m) = v
        .get("error")
        .and_then(|e| e.get("message"))
        .and_then(|x| x.as_str())
    {
        m.to_string()
    } else if let Some(s) = v.get("error").and_then(|e| e.as_str()) {
        s.to_string()
    } else {
        return None;
    };
    // 附加失败证据（run 的 stdout/stderr、verify 的 errors 数组）——截断防撑爆
    if let Some(data) = v.get("data") {
        let mut evidence: Vec<String> = Vec::new();
        if let Some(so) = data
            .get("stdout")
            .and_then(|x| x.as_str())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            evidence.push(format!("stdout:\n{}", truncate(so, 20000)));
        }
        if let Some(se) = data
            .get("stderr")
            .and_then(|x| x.as_str())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            evidence.push(format!("stderr:\n{}", truncate(se, 20000)));
        }
        if let Some(errs) = data.get("errors").and_then(|x| x.as_array()) {
            let texts: Vec<String> = errs
                .iter()
                .filter_map(|e| e.as_str().map(|s| s.trim().to_string()))
                .filter(|s| !s.is_empty())
                .collect();
            if !texts.is_empty() {
                evidence.push(format!("errors:\n{}", truncate(&texts.join("\n"), 20000)));
            }
        }
        if !evidence.is_empty() {
            msg.push_str("\n\n");
            msg.push_str(&evidence.join("\n"));
        }
    }
    Some(msg)
}

/// 全文通道专用：把 `read` 结果 JSON **无损**摊平成纯正文（去掉 JSON 壳）。
pub fn read_full_text(raw: &str) -> Option<String> {
    let v: Value = serde_json::from_str(raw).ok()?;
    let files = v.pointer("/data/files").and_then(|f| f.as_array())?;
    if files.is_empty() {
        return None;
    }
    let mut out = String::new();
    for f in files {
        let path = f.get("path").and_then(|x| x.as_str()).unwrap_or("");
        let total = f.get("total_lines").and_then(|x| x.as_u64());
        let mode = f.get("mode").and_then(|x| x.as_str()).unwrap_or("");
        let trunc = f.get("truncated").and_then(|x| x.as_bool()).unwrap_or(false);
        // 定位头：路径 · 行数 · 模式 · 是否截断 —— 模型据此决定"还要不要再读一段"
        if !path.is_empty() {
            let mut head = format!("【{path}");
            if let Some(t) = total {
                head.push_str(&format!(" · {t} 行"));
            }
            if !mode.is_empty() && mode != "full" {
                head.push_str(&format!(" · mode={mode}"));
            }
            if trunc {
                head.push_str(" · 已截断");
            }
            head.push('】');
            out.push_str(&head);
            out.push('\n');
        }
        if let Some(map) = f.get("structure_map").and_then(|x| x.as_str()) {
            if !map.trim().is_empty() {
                out.push_str("【结构地图】按行号定位：read 带 start_line/end_line\n");
                out.push_str(map.trim_end());
                out.push('\n');
            }
        }
        let content = f.get("content").and_then(|x| x.as_str()).unwrap_or("");
        out.push_str(content);
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
    }
    // read 顶层 error（部分文件失败时会有）——留着，别丢
    if let Some(e) = v.get("error").filter(|e| !e.is_null()) {
        out.push_str(&format!("[error] {e}\n"));
    }
    if out.trim().is_empty() {
        None
    } else {
        Some(out)
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let t: String = s.chars().take(max).collect();
        format!("{t}…（已截断，共 {} 字符）", s.chars().count())
    }
}

/// data.get(路径) 或 v.get
fn data_get<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    v.get("data")
        .and_then(|d| d.get(key))
        .or_else(|| v.get(key))
}

fn render(name: &str, v: &Value) -> String {
    let data = v.get("data").unwrap_or(v);
    match name {
        // ── 读文件：单文件精读全文 / 多文件每段智能预览 / 超多文件清单 ──
        "read" => {
            let files = data
                .get("files")
                .and_then(|f| f.as_array())
                .cloned()
                .unwrap_or_default();
            if files.is_empty() {
                return String::new();
            }
            // （v11·用户"并行读取怎么排"）：**多文件（2-5 个）每个文件一段**——
            if files.len() > 1 && files.len() <= 5 {
                let mut parts: Vec<String> = Vec::new();
                for f in files.iter() {
                    let one = read_with_linenums(std::slice::from_ref(f));
                    if !one.is_empty() {
                        parts.push(one);
                    }
                }
                if !parts.is_empty() {
                    return parts.join("\n\n");
                }
            }
            let mut out = Vec::new();
            for f in files.iter() {
                let path = f.get("path").and_then(|x| x.as_str()).unwrap_or("?");
                let total = f.get("total_lines").and_then(|x| x.as_u64()).unwrap_or(0);
                let mode = f.get("mode").and_then(|x| x.as_str()).unwrap_or("");
                let trunc = f
                    .get("truncated")
                    .and_then(|x| x.as_bool())
                    .unwrap_or(false);
                let mut line = format!("{path} · {total} 行");
                // 分段读取（lines）带区间——需求"这次读多少到多少行"全程可见
                if mode == "lines" {
                    let sl = f.get("start_line").and_then(|x| x.as_u64());
                    let el = f.get("end_line").and_then(|x| x.as_u64());
                    if let (Some(s), Some(e)) = (sl, el) {
                        line.push_str(&format!(" · 本次读 {s}–{e} 行"));
                    }
                }
                if !mode.is_empty() && mode != "lines" {
                    line.push_str(&format!("（mode={mode}）"));
                }
                if trunc {
                    line.push_str(" · 截断");
                }
                out.push(line);
            }
            let more = String::new();
            // （上下文投喂 P0 修复）：渲染尊重 read 的模式语义——
            let body = if files.len() == 1 {
                let first = &files[0];
                let mode = first.get("mode").and_then(|x| x.as_str()).unwrap_or("full");
                let content = first.get("content").and_then(|x| x.as_str()).unwrap_or("");
                // （读多少行较劲根治）：**结构地图渲染到头部**——模型先看目录
                let map = first
                    .get("structure_map")
                    .and_then(|x| x.as_str())
                    .unwrap_or("");
                let mut b = String::new();
                if !map.is_empty() {
                    b.push_str(&format!(
                        "【结构地图】此文件过大已截断，以下是文件结构（函数/结构体 → 起始行号）——\
                         要读某个目标：用 read 带 start_line/end_line（lines 分段）定位该行号区间。\n{map}\n"
                    ));
                }
                if matches!(mode, "full" | "lines") && !content.is_empty() {
                    b.push_str(content)
                } else if content.chars().count() < 4000 {
                    // （auto 语义修正·不二次截断）：auto 的 content 是 read_one
                    b.push_str(content)
                } else {
                    b.push_str(&read_with_linenums(&files))
                }
                b
            } else {
                String::new()
            };
            let mut result = out.join("\n");
            if !body.is_empty() {
                result.push_str("\n\n");
                result.push_str(&body);
            }
            if !more.is_empty() {
                result.push('\n');
                result.push_str(&more);
            }
            result
        }
        // ── 写：路径 + 字节 + 备份 ──
        "write" => {
            let path = data_get(v, "path").and_then(|x| x.as_str()).unwrap_or("");
            let bytes = data_get(v, "bytes_written")
                .or_else(|| data_get(v, "written"))
                .and_then(|x| x.as_u64())
                .unwrap_or(0);
            let bak = data_get(v, "backup_path")
                .and_then(|x| x.as_str())
                .unwrap_or("");
            format!(
                "{path}\n写入 {} 字节 {}",
                bytes,
                if bak.is_empty() {
                    String::new()
                } else {
                    format!("\n备份: {bak}")
                }
            )
            .trim()
            .to_string()
        }
        // ── 修改（modify）：文件 + 每处 find→replace diff（± 行级）──
        "modify" => {
            let file = data_get(v, "file").and_then(|x| x.as_str()).unwrap_or("");
            let changes = data_get(v, "changes")
                .and_then(|c| c.as_array())
                .cloned()
                .unwrap_or_default();
            let changed = changes.len();
            // 统计增/删行数（按 find/replace 拆行）——供前端/用户一眼看净变更
            let mut added = 0usize;
            let mut removed = 0usize;
            for c in changes.iter() {
                let old = c.get("find").and_then(|x| x.as_str()).unwrap_or("");
                let new = c.get("replace").and_then(|x| x.as_str()).unwrap_or("");
                if !old.is_empty() {
                    removed += old.split('\n').filter(|l| !l.is_empty()).count();
                }
                if !new.is_empty() {
                    added += new.split('\n').filter(|l| !l.is_empty()).count();
                }
            }
            let mut out = format!("{file}\n修改 {changed} 处  +{added} −{removed}");
            for c in changes.iter() {
                let old = c.get("find").and_then(|x| x.as_str()).unwrap_or("");
                let new = c.get("replace").and_then(|x| x.as_str()).unwrap_or("");
                out.push('\n');
                // 修改：旧块行红、新块行绿（find/replace 上下紧邻 = 视觉"改了这段"）
                if !old.is_empty() {
                    for l in old.split('\n') {
                        if !l.is_empty() {
                            out.push_str(&format!("\n- {}", l));
                        }
                    }
                }
                if !new.is_empty() {
                    for l in new.split('\n') {
                        if !l.is_empty() {
                            out.push_str(&format!("\n+ {}", l));
                        }
                    }
                }
            }
            // modify 特有的确定性校验（find 已消失、replace 已出现）
            if let Some(note) = v
                .get("data")
                .and_then(|d| d.get("verified"))
                .and_then(|x| x.get("note"))
                .and_then(|x| x.as_str())
            {
                out.push_str(&format!("\n校验: {note}"));
            }
            out
        }
        // ── 命令执行：命令 + 退出码 + stdout/stderr 前几行 ──
        "run" => {
            let cmd = data_get(v, "command")
                .and_then(|x| x.as_str())
                .or_else(|| data_get(v, "command_asked").and_then(|x| x.as_str()))
                .unwrap_or("");
            let exit = data_get(v, "exit_code")
                .and_then(|x| x.as_i64())
                .unwrap_or(-1);
            let stdout = data_get(v, "stdout")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            let stderr = data_get(v, "stderr")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            // 首行是"信封"：命令 + 退出码。缺命令时**不留空行**，直接给退出码行。
            let mut s = if cmd.is_empty() {
                format!("exit {exit}")
            } else {
                format!("{cmd}\nexit {exit}")
            };
            if let Some(p) = data_get(v, "spill_path").and_then(|x| x.as_str()) {
                s.push_str(&format!("\n（输出已截断，全文: {p}）"));
            }
            if !stdout.is_empty() {
                s.push_str(&format!("\n{}", truncate(&stdout, 20000)));
            }
            if !stderr.is_empty() {
                s.push_str(&format!("\n[stderr]\n{}", truncate(&stderr, 20000)));
            }
            s
        }
        // ── 验证 ──
        "verify" => {
            let cmd = data_get(v, "command")
                .or_else(|| data_get(v, "target"))
                .and_then(|x| x.as_str())
                .unwrap_or("");
            let exit = data_get(v, "exit_code")
                .and_then(|x| x.as_i64())
                .unwrap_or(-1);
            let passed = data_get(v, "passed")
                .and_then(|x| x.as_bool())
                .unwrap_or(exit == 0);
            let out = data_get(v, "output")
                .or_else(|| data_get(v, "stdout"))
                .and_then(|x| x.as_str())
                .unwrap_or("");
            format!(
                "验证{}: {}\nexit {}\n{}",
                if passed { "通过" } else { "未过" },
                cmd,
                exit,
                truncate(out, 20000)
            )
        }
        // ── 搜索 ──
        "search" => {
            // 结构化排版：汇总行 + 每条「文件:行号」作标签、下方缩进展示匹配文本（放宽截断，避免长内容被砍）
            let matches = data
                .get("matches")
                .and_then(|m| m.as_array())
                .cloned()
                .unwrap_or_default();
            let total = data
                .get("total")
                .and_then(|x| x.as_u64())
                .unwrap_or(matches.len() as u64);
            let has_more = data
                .get("has_more")
                .and_then(|x| x.as_bool())
                .unwrap_or(false);
            // 截断自解释：has_more 时必须给 next_offset（否则模型看不到断点 → 重发同 pattern
            let next_off = data
                .get("next_offset")
                .and_then(|x| x.as_u64())
                .unwrap_or(0);
            let more_hint = if has_more {
                if next_off > 0 {
                    format!(
                        "（共 {} 条，已显示前 {} 条；scan_offset={next_off} 续扫剩余）",
                        total,
                        matches.len()
                    )
                } else {
                    format!("（共 {} 条，已显示前 {} 条）", total, matches.len())
                }
            } else {
                String::new()
            };
            let mut out = format!("{} 处匹配{}", total, more_hint);
            // 已带窗口的命中数（默认窗口只附前 15 处；超出则提示剩余可 read 指定行号）
            let mut windowed = 0usize;
            for m in matches.iter() {
                let f = m.get("file").and_then(|x| x.as_str()).unwrap_or("?");
                let l = m.get("line").and_then(|x| x.as_u64()).unwrap_or(0);
                let t = m.get("text").and_then(|x| x.as_str()).unwrap_or("");
                // 标签行：路径:行号；内容行：缩进 2 空格，全文给出（仅在病态超长单行才截断）
                out.push_str(&format!("\n\n{}:{}", f, l));
                // 上下文窗口（智能投喂）：工具已附命中行前后文（带行号+命中标记）
                if let Some(ctx) = m.get("context").and_then(|c| c.as_array()) {
                    if !ctx.is_empty() {
                        windowed += 1;
                        for c in ctx {
                            let ln = c.get("line").and_then(|x| x.as_u64()).unwrap_or(0);
                            let tx = c.get("text").and_then(|x| x.as_str()).unwrap_or("");
                            let is_match =
                                c.get("match").and_then(|x| x.as_bool()).unwrap_or(false);
                            out.push_str(&format!(
                                "\n{:>6} {}{}",
                                ln,
                                if is_match { "▶ " } else { "  " },
                                truncate(tx, 8000)
                            ));
                        }
                        continue;
                    }
                }
                out.push_str(&format!("\n  {}", truncate(t, 8000)));
            }
            if windowed < matches.len() {
                out.push_str(&format!(
                    "\n…还有 {} 处命中未附窗口（命中过多，默认仅前 {} 处带前后文）——需要细节时用 read 传 start_line/end_line 精确读取指定行。",
                    matches.len() - windowed,
                    windowed
                ));
            }
            out
        }
        // ── 找文件 ──
        "find_files" => {
            let files = data
                .get("files")
                .and_then(|f| f.as_array())
                .cloned()
                .unwrap_or_default();
            let total = data
                .get("total")
                .and_then(|x| x.as_u64())
                .unwrap_or(files.len() as u64);
            let truncated = data
                .get("truncated")
                .and_then(|x| x.as_bool())
                .unwrap_or(false);
            let mut out = format!("{} 个文件", total);
            for f in files.iter() {
                let p = f.get("path").and_then(|x| x.as_str()).unwrap_or("");
                let sz = f.get("size").and_then(|x| x.as_u64()).unwrap_or(0);
                out.push_str(&format!("\n- {p} · {sz}B"));
            }
            if truncated {
                out.push_str(&format!("\n…已显示 {total} 条（达上限，后续未列出）"));
            } else if total > files.len() as u64 {
                out.push_str(&format!(
                    "\n…还有 {} 个未显示（共 {total} 个）",
                    total - files.len() as u64
                ));
            }
            out
        }
        // ── 列目录 ──
        "list" => {
            let entries = data
                .get("entries")
                .and_then(|e| e.as_array())
                .cloned()
                .unwrap_or_default();
            let dirs = data
                .get("dirs")
                .and_then(|x| x.as_u64())
                .unwrap_or_else(|| {
                    entries
                        .iter()
                        .filter(|e| e.get("type").and_then(|t| t.as_str()) == Some("dir"))
                        .count() as u64
                });
            let files = data
                .get("files")
                .and_then(|x| x.as_u64())
                .unwrap_or_else(|| {
                    entries
                        .iter()
                        .filter(|e| e.get("type").and_then(|t| t.as_str()) == Some("file"))
                .count() as u64
            });
            let truncated = data
                .get("truncated")
                .and_then(|x| x.as_bool())
                .unwrap_or(false);
            let mut out = format!("{dirs} 目录 · {files} 文件");
            for e in entries.iter() {
                // （十次）：列目录详情窗改显完整路径（与 audit 一致），不再只显文件名
                let p = e.get("path").and_then(|x| x.as_str()).unwrap_or("");
                out.push_str(&format!("\n- {p}"));
            }
            // 仅当工具自身触发 max_entries 上限（truncated=true）才诚实提示；
            if truncated {
                out.push_str("\n…结果被 max_entries 上限截断——请缩小 path 或传 recursive=false 聚焦，或用 find_files 按 pattern 收窄");
            }
            out
        }
        // ── 盘点 ──
        "audit" => {
            let file_count = data.get("file_count").and_then(|x| x.as_u64()).unwrap_or(0);
            let files = data
                .get("files")
                .and_then(|f| f.as_array())
                .cloned()
                .unwrap_or_default();
            let mut out = format!("扫描 {} 个文件", file_count.max(files.len() as u64));
            for f in files.iter() {
                let p = f.get("path").and_then(|x| x.as_str()).unwrap_or("");
                out.push_str(&format!("\n- {p}"));
            }
            out
        }
        // ── 环境 ──
        "env" => {
            let os = data_get(v, "os").and_then(|x| x.as_str()).unwrap_or("");
            let cwd = data_get(v, "cwd").and_then(|x| x.as_str()).unwrap_or("");
            let env = data
                .get("env")
                .and_then(|e| e.as_object())
                .cloned()
                .unwrap_or_default();
            let mut out = format!("OS: {os}\ncwd: {cwd}");
            for (k, val) in env.iter() {
                out.push_str(&format!("\n{} = {}", k, val.as_str().unwrap_or("")));
            }
            out
        }
        // ── 数据库查询 ──
        "db_query" => {
            let rows = data
                .get("rows")
                .and_then(|r| r.as_array())
                .cloned()
                .unwrap_or_default();
            let cols = data
                .get("columns")
                .and_then(|c| c.as_array())
                .cloned()
                .unwrap_or_default();
            let header: Vec<String> = cols
                .iter()
                .filter_map(|c| c.as_str().map(String::from))
                .collect();
            let mut out = String::from("查询结果");
            if !header.is_empty() {
                out.push_str(&format!("\n{}", header.join(" | ")));
            }
            for r in rows.iter() {
                let vals: Vec<String> = r
                    .as_array()
                    .map(|arr| arr.iter().map(|x| x.to_string()).collect())
                    .unwrap_or_default();
                out.push_str(&format!("\n{}", vals.join(" | ")));
            }
            out
        }
        // ── 自我诊断 ──
        "self_heal" => {
            let issues = data
                .get("issues")
                .and_then(|i| i.as_array())
                .cloned()
                .unwrap_or_default();
            let mut out = String::from("自我诊断");
            if issues.is_empty() {
                out.push_str("\n无已知重复失败模式");
            }
            for i in issues.iter() {
                let code = i.get("error_code").and_then(|x| x.as_str()).unwrap_or("?");
                let cnt = i.get("count").and_then(|x| x.as_u64()).unwrap_or(0);
                out.push_str(&format!("\n[{code}] × {cnt}"));
            }
            out
        }
        // ── 上网 ──
        "web_fetch" => {
            // 契约：web_fetch 输出 data.url / data.status / data.content（无 title/text）。
            let url = data_get(v, "url").and_then(|x| x.as_str()).unwrap_or("");
            let text = truncate(
                &data
                    .get("content")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                20000,
            );
            if text.is_empty() {
                url.to_string()
            } else {
                format!("{url}\n{text}")
            }
        }
        // ── 诊断（T2T 侦察：线索定位结论 + 命中文件清单）──
        "diagnose" => {
            let conclusion = data_get(v, "conclusion")
                .and_then(|c| c.as_str())
                .unwrap_or("")
                .to_string();
            let candidates = data_get(v, "candidates")
                .and_then(|c| c.as_array())
                .cloned()
                .unwrap_or_default();
            let file_names: Vec<String> = candidates
                .iter()
                .filter_map(|c| c.get("file").and_then(|f| f.as_str()).map(String::from))
                .collect();
            let mut out = String::new();
            if !conclusion.is_empty() {
                out.push_str(&format!("诊断结论：{conclusion}\n"));
            }
            out.push_str(&format!("命中 {} 个文件：", file_names.len()));
            if file_names.is_empty() {
                out.push_str("无");
            } else {
                for (i, f) in file_names.iter().enumerate() {
                    if i > 0 {
                        out.push_str(" / ");
                    }
                    out.push_str(f);
                }
            }
            out
        }
        _ => String::new(),
    }
}

/// （Read 预览 v5）：read 工具智能截断预览——
fn read_with_linenums(files: &[Value]) -> String {
    let Some(first) = files.first() else {
        return String::new();
    };
    let content = first.get("content").and_then(|x| x.as_str()).unwrap_or("");
    if content.is_empty() {
        return String::new();
    }

    let raw_lines: Vec<&str> = content.split('\n').collect();
    let total_disp = raw_lines.len();
    let total = first
        .get("total_lines")
        .and_then(|x| x.as_u64())
        .unwrap_or(total_disp as u64);
    let path = first.get("path").and_then(|x| x.as_str()).unwrap_or("");
    let ext = path
        .rsplit_once('.')
        .map(|(_, e)| e.to_string())
        .unwrap_or_default();

    // 剥 "N\t" 行号前缀（read 带 numbered=true 自动加），保留真实代码。
    let strip = |line: &str| -> String {
        if let Some(idx) = line.find('\t') {
            let prefix = &line[..idx];
            let has_digit = prefix.chars().any(|c| c.is_ascii_digit());
            let all_ok = prefix.chars().all(|c| c.is_ascii_digit() || c == ' ');
            if has_digit && all_ok {
                return line[idx + 1..].to_string();
            }
        }
        line.to_string()
    };

    // 阈值分级——≤60 行全显示；大文件前 16/后 10；超大文件（>500 行）前 26/后 18
    let (head, tail) = if total_disp <= 60 {
        (usize::MAX, 0usize)
    } else if total_disp <= 500 {
        (16, 10)
    } else {
        (26, 18)
    };
    let show_all = head == usize::MAX || total_disp <= head + tail + 1;

    // 头带 path——多文件每段可独立标识（单文件时也自我完整）
    let mut out = format!("{path} · {total} 行");
    if !ext.is_empty() && ext.len() <= 6 {
        out.push_str(&format!(" · {}", ext));
    }
    out.push_str("\n─────────\n");

    if show_all {
        for (i, line) in raw_lines.iter().enumerate() {
            out.push_str(&format!("{:>4}  {}\n", i + 1, strip(line)));
        }
    } else {
        // 前 head 行（行号 1..head 连续）
        for i in 0..head {
            let line = raw_lines.get(i).copied().unwrap_or("");
            out.push_str(&format!("{:>4}  {}\n", i + 1, strip(line)));
        }
        // 中间省略——明确标注"第 X–Y 行省略 N 行"（行号连续区间，不跳变）
        let mid_start = head + 1;
        let mid_end = total_disp - tail;
        out.push_str(&format!(
            "… 第 {mid_start}–{mid_end} 行省略（共 {} 行） …\n",
            mid_end - mid_start + 1
        ));
        // 后 tail 行（行号 (total_disp-tail+1)..total_disp 连续）
        for i in (total_disp - tail)..total_disp {
            let line = raw_lines.get(i).copied().unwrap_or("");
            out.push_str(&format!("{:>4}  {}\n", i + 1, strip(line)));
        }
    }
    out
}

#[cfg(test)]
#[path = "summary_render_tests.rs"]
mod summary_render_tests;
