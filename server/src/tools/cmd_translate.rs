//! 命令翻译层（从 cmd_tools.rs 独立：翻译/清洗归本文件，执行归 cmd_tools）

/// cmd/c 不提供 tail/head/grep/awk/sed/wc——模型训练语料里 Unix shell 形态多见，
pub fn translate_unix_pipeline(cmd: &str) -> Result<String, String> {
    //（`git stash list 2>&1; echo ---; git log` → git 收到 ';' 参数 → fatal: bad revision）。
    let cmd = translate_semicolons(cmd);
    if !cmd.contains('|') {
        return Ok(cmd.to_string());
    }
    // 无顶层 &（2>&1 的重定向 & 除外）也无括号 → 纯管道，走单管道翻译路径
    if !has_top_level_amp_or_paren(&cmd) {
        return translate_pipeline_only(&cmd);
    }
    // cmd 复合：顶层 & 分支切分（引号感知 + 括号深度 + 跳过 2>&1 重定向）→ 逐支翻译 → 原样重组
    let branches: Vec<(String, String)> = split_top_level_branches(&cmd)
        .into_iter()
        .filter(|(p, _)| !p.trim().is_empty())
        .collect();
    let mut out = String::new();
    for (i, (piece, sep)) in branches.iter().enumerate() {
        let piece = piece.trim();
        let t = if let Some(inner) = strip_one_outer_paren(piece) {
            // 整支包在单层括号里：翻译括号内，括号原样搬回（保平衡）
            format!("({})", translate_pipeline_only(&inner)?)
        } else {
            translate_pipeline_only(piece)?
        };
        out.push_str(&t);
        if i + 1 < branches.len() {
            out.push_str(sep);
        }
    }
    Ok(out)
}

/// 是否存在顶层 & 复合（跳过 2>&1 / 1>&2 重定向）或括号分组
pub(crate) fn has_top_level_amp_or_paren(cmd: &str) -> bool {
    let mut in_quote: Option<char> = None;
    let mut depth = 0i32;
    let chars: Vec<char> = cmd.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        match c {
            '"' | '\'' => {
                if in_quote == Some(c) {
                    in_quote = None;
                } else if in_quote.is_none() {
                    in_quote = Some(c);
                }
            }
            '(' if in_quote.is_none() => depth += 1,
            ')' if in_quote.is_none() => depth -= 1,
            '&' if in_quote.is_none() && depth == 0 => {
                let prev = if i > 0 { chars[i - 1] } else { '\0' };
                if prev != '>' {
                    return true;
                }
            }
            _ => {}
        }
    }
    depth != 0
}

/// 顶层分支切分：返回 (piece, 分支后的分隔符)，分隔符保持 && / & 原样
fn split_top_level_branches(cmd: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut cur = String::new();
    let mut in_quote: Option<char> = None;
    let mut depth = 0i32;
    let chars: Vec<char> = cmd.chars().collect();
    let mut i = 0usize;
    let mut last: Option<char> = None;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '"' | '\'' => {
                if in_quote == Some(c) {
                    in_quote = None;
                } else if in_quote.is_none() {
                    in_quote = Some(c);
                }
                cur.push(c);
            }
            '(' if in_quote.is_none() => {
                depth += 1;
                cur.push(c);
            }
            ')' if in_quote.is_none() => {
                depth -= 1;
                cur.push(c);
            }
            '&' if in_quote.is_none() && depth == 0 && last != Some('>') => {
                let dbl = i + 1 < chars.len() && chars[i + 1] == '&';
                let sep = if dbl { " && " } else { " & " };
                out.push((std::mem::take(&mut cur), sep.to_string()));
                if dbl {
                    i += 1;
                }
                last = Some('&');
                i += 1;
                continue;
            }
            _ => cur.push(c),
        }
        last = Some(c);
        i += 1;
    }
    out.push((cur, String::new()));
    out
}

/// 整支包在单层括号里 → 剥出括号内内容（"（…）" 整体，非首尾各一个散括号）
fn strip_one_outer_paren(piece: &str) -> Option<String> {
    let p = piece.trim();
    if !p.starts_with('(') || !p.ends_with(')') {
        return None;
    }
    let mut depth = 0i32;
    let mut first_close: Option<usize> = None;
    for (i, c) in p.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    first_close = Some(i);
                    break;
                }
            }
            _ => {}
        }
    }
    let close = first_close?;
    if close == p.len() - 1 {
        Some(p[1..close].to_string())
    } else {
        None
    }
}

/// 纯管道翻译（无顶层 & / 括号；或复合分支剥括号后的内芯）
fn translate_pipeline_only(cmd: &str) -> Result<String, String> {
    let segments = split_pipes_respecting_quotes(cmd);
    let mut out: Vec<String> = Vec::with_capacity(segments.len());
    let mut translated_any = false;
    for (i, seg) in segments.iter().enumerate() {
        let seg = seg.trim();
        let first = seg.split_whitespace().next().unwrap_or("");
        match first {
            // `tail`/`head` **只在真管道位置（前有 `|`）**才能翻成"读上游输出"。
            "tail" | "head" => {
                let op = if first == "tail" { "Last" } else { "First" };
                if i == 0 {
                    return Err(format!(
                        "{first} 出现在管道首段，没有上游输入可读，翻译后必然为空。\
                         要读文件请用 read 工具（它给行号，便于定位）；\
                         要看某个命令输出的尾巴，把它接在 `|` 之后。原段：{seg}"
                    ));
                }
                if seg.split_whitespace().any(|t| t == "-c") {
                    return Err(format!(
                        "{first} -c N（按字节截取）在 Windows 侧没有等价 —— Select-Object 只按行。\
                         要读日志尾部请用 read 工具（可给 offset / limit）。原段：{seg}"
                    ));
                }
                if has_file_arg(seg) {
                    return Err(format!(
                        "{first} 带文件参数：Windows 侧没有等价写法，\
                         旧实现会翻成「读标准输入」而丢掉文件名（拿到空结果）。\
                         要读该文件请用 read 工具；要按行看尾部可用 run 的 script 参数跑 python。\
                         原段：{seg}"
                    ));
                }
                let n = extract_tail_n(seg);
                out.push(format!(
                    "powershell -NoProfile -Command \"$input | Select-Object -{op} {n}\""
                ));
                translated_any = true;
            }
            "grep" => {
                // flag **先过白名单**（只放行 -i / -n）—— 其余一律拒绝，
                let (ci, numbered, pat) = grep_flags_and_pat(seg)?;
                if pat.contains('|') {
                    return Err(format!(
                        "grep 正则含 `|`（交替）无法安全翻译为 Windows findstr —— \
                         请改用 run 的 script 参数跑 python，或分开多次 grep（段 {i}: {seg}）"
                    ));
                }
                let mut flags = String::new();
                if ci {
                    flags.push_str(" /i");
                }
                if numbered {
                    flags.push_str(" /n");
                }
                if pat.is_empty() {
                    out.push("findstr .".to_string());
                } else {
                    out.push(format!("findstr{flags} \"{}\"", pat.replace('"', "\"\"")));
                }
                translated_any = true;
            }
            "Select-String" => {
                // 经 cmd /C 执行时报「'Select-String' 不是内部或外部命令」→ 255 → 模型换形态
                let pat = extract_select_string_pat(seg);
                if pat.is_empty() {
                    out.push("findstr .".to_string());
                } else {
                    let alts: Vec<String> = pat
                        .split('|')
                        .filter(|p| !p.trim().is_empty())
                        .map(|p| format!("/C:\"{}\"", p.trim()))
                        .collect();
                    out.push(format!("findstr {}", alts.join(" ")));
                }
                translated_any = true;
            }
            "wc" => {
                // 只放行 `wc -l`（行数 → `find /c /v ""`）。`-w` 词数 / `-c` 字节数 / 无参数
                let toks: Vec<&str> = seg.split_whitespace().collect();
                let only_l = toks
                    .iter()
                    .skip(1)
                    .filter(|t| t.starts_with('-'))
                    .all(|t| t.trim_start_matches('-').chars().all(|c| c == 'l'))
                    && toks.iter().skip(1).any(|t| t.starts_with('-') && t.contains('l'));
                if !only_l {
                    return Err(format!(
                        "wc 只支持 `wc -l`（行数）—— `-w` 词数 / `-c` 字节数 / 无参数在 Windows 无等价。\
                         改用 run 的 script 参数跑 python。原段：{seg}"
                    ));
                }
                let files: Vec<&str> = toks
                    .iter()
                    .skip(1)
                    .filter(|t| !t.starts_with('-'))
                    .copied()
                    .collect();
                out.push(if files.is_empty() {
                    "find /c /v \"\"".to_string()
                } else {
                    format!("find /c /v \"\" {}", files.join(" "))
                });
                translated_any = true;
            }
            "cat" => {
                // `cat file` → `type file`（cmd 内置；Windows 无 cat）。
                if seg.split_whitespace().skip(1).any(|t| t.starts_with('-')) {
                    return Err(format!(
                        "cat 带 flag（如 `-n` 显示行号）在 Windows 的 type 里没有等价 —— \
                         要看带行号的正文请用 read 工具（它天然带行号）。原段：{seg}"
                    ));
                }
                out.push(seg.replacen("cat", "type", 1));
                translated_any = true;
            }
            _ => out.push(seg.to_string()),
        }
    }
    if translated_any {
        let joined = out.join(" | ");
        tracing::debug!(original = %cmd, translated = %joined, "run 命令 Unix→Windows 清洗");
        Ok(joined)
    } else {
        Ok(cmd.to_string())
    }
}

/// 整条命令改写成 **PowerShell 原生管道**形态（跨行内联代码专用）。
pub fn to_ps_pipeline(cmd: &str) -> Option<String> {
    if !cmd.contains('|') {
        return None;
    }
    let segs = split_pipes_respecting_quotes(cmd);
    if segs.len() < 2 {
        return None;
    }
    let mut out: Vec<String> = Vec::with_capacity(segs.len());
    for (i, raw) in segs.iter().enumerate() {
        let seg = raw.trim();
        if i == 0 {
            out.push(seg.to_string());
            continue;
        }
        let first = seg.split_whitespace().next().unwrap_or("");
        match first {
            "head" => out.push(format!("Select-Object -First {}", extract_tail_n(seg))),
            "tail" => out.push(format!("Select-Object -Last {}", extract_tail_n(seg))),
            "grep" => {
                let (ci, pat) = extract_grep_pat(seg);
                if pat.is_empty() {
                    out.push("Select-String -Pattern .".to_string());
                } else {
                    let simple = !pat.contains(['|', '(', ')', '[', ']', '*', '+', '\\', '^', '$']);
                    let flag = if ci { " -SimpleMatch" } else { "" };
                    if simple && !ci {
                        out.push(format!("Select-String -SimpleMatch -Pattern \"{pat}\""));
                    } else {
                        out.push(format!("Select-String{flag} -Pattern \"{pat}\""));
                    }
                }
            }
            "wc" if seg.contains("-l") => out.push("Measure-Object -Line".to_string()),
            "cat" => out.push(seg.replacen("cat", "Get-Content", 1)),
            _ => return None,
        }
    }
    Some(out.join(" | "))
}

/// 摘取 `-c` / `-e` 之后的内联代码段（归翻译域：翻译/清洗归本文件）。
pub fn extract_inline_code(cmd: &str, flag: &str) -> String {
    let seg = split_pipes_respecting_quotes(cmd)
        .first()
        .cloned()
        .unwrap_or_else(|| cmd.to_string());
    let rest = seg.splitn(2, flag).nth(1).unwrap_or("").trim();
    let q = rest.chars().next().unwrap_or('\0');
    if (q == '"' || q == '\'') && rest.len() >= 2 && rest.ends_with(q) {
        rest[1..rest.len() - 1].to_string()
    } else {
        rest.trim_matches('"').trim_matches('\'').trim().to_string()
    }
}

/// 管道切分（引号感知）：`|` 在引号内不切分——`grep -E "^error|-->|E0425"` 必须作为
pub fn split_pipes_respecting_quotes(cmd: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quote: Option<char> = None;
    for c in cmd.chars() {
        match c {
            '"' | '\'' => {
                if in_quote == Some(c) {
                    in_quote = None;
                } else if in_quote.is_none() {
                    in_quote = Some(c);
                }
                cur.push(c);
            }
            '|' if in_quote.is_none() => {
                out.push(std::mem::take(&mut cur));
            }
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

/// 语句切分（引号感知）：按**语句分隔符**切开 —— `;`、`&`、`&&`、`||`。
pub fn split_statements(cmd: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let chars: Vec<char> = cmd.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if c == '"' || c == '\'' {
            if quote == Some(c) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(c);
            }
            cur.push(c);
            i += 1;
            continue;
        }
        if quote.is_none() {
            let prev_is_redirect = i > 0 && chars[i - 1] == '>';
            // `|` **不算**语句边界（那是管道、同一句话内部的数据流）—— 见函数头说明。
            let is_sep = c == ';' || (c == '&' && !prev_is_redirect);
            if is_sep {
                out.push(std::mem::take(&mut cur));
                // 吞掉成对分隔符（`&&` / `||`），避免留下空段
                if (c == '&' || c == '|') && i + 1 < chars.len() && chars[i + 1] == c {
                    i += 1;
                }
                i += 1;
                continue;
            }
        }
        cur.push(c);
        i += 1;
    }
    out.push(cur);
    out
}

/// 命令里是否含**转义引号**（`\"`）—— 这是 Unix / PowerShell 母语的写法。
pub fn has_escaped_quote(cmd: &str) -> bool {
    let ch: Vec<char> = cmd.chars().collect();
    let mut i = 0usize;
    while i + 1 < ch.len() {
        if ch[i] == '\\' && ch[i + 1] == '"' {
            return true;
        }
        i += 1;
    }
    false
}

/// 命令里是否含 **bash 风格盘符路径**（`/c/Users/...`）。
pub fn has_posix_drive_path(cmd: &str) -> bool {
    let ch: Vec<char> = cmd.chars().collect();
    let mut i = 0usize;
    while i + 2 < ch.len() {
        if ch[i] == '/' && ch[i + 1].is_ascii_alphabetic() && ch[i + 2] == '/' {
            let prev_ok = i == 0
                || matches!(
                    ch[i - 1],
                    ' ' | '\t' | '"' | '\'' | '&' | ';' | '(' | '|' | '='
                );
            if prev_ok {
                return true;
            }
        }
        i += 1;
    }
    false
}

/// 段里是否带**文件参数**（自由参数：非 flag，也不是 flag 的值）。
fn has_file_arg(seg: &str) -> bool {
    let toks: Vec<&str> = seg.split_whitespace().collect();
    let mut i = 1usize;
    while i < toks.len() {
        let t = toks[i];
        if t == "-n" || t == "-c" {
            i += 2;
            continue;
        }
        if is_redirect_token(t) {
            i += if t == ">" || t == ">>" { 2 } else { 1 };
            continue;
        }
        if t.starts_with('-') {
            i += 1;
            continue;
        }
        return true;
    }
    false
}

/// 是否是**重定向记号**（不是要读的文件）。
fn is_redirect_token(t: &str) -> bool {
    let s = t.trim_start_matches(|c: char| c.is_ascii_digit());
    s.starts_with('>')
}

/// 解析 `tail -40` / `tail -n 40` / `tail -5` 的行数（默认 10）。
fn extract_tail_n(seg: &str) -> usize {
    let toks: Vec<&str> = seg.split_whitespace().collect();
    let lead_num = |t: &str| -> Option<usize> {
        let digits: String = t.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() {
            None
        } else {
            digits.parse().ok()
        }
    };
    let mut i = 1;
    while i < toks.len() {
        match toks[i] {
            "-n" => {
                if let Some(v) = toks.get(i + 1).and_then(|t| lead_num(t)) {
                    return v;
                }
                i += 2;
            }
            t if t.starts_with('-') && t.len() > 1 => {
                if let Some(v) = lead_num(&t[1..]) {
                    return v;
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    10
}

/// 单命令 Unix 形态 → Windows 等价（**非管道**场景，命令层职责的一部分）。
fn split_chain(cmd: &str) -> Vec<(String, String)> {
    let mut parts: Vec<(String, String)> = Vec::new();
    let mut pending_op = String::new();
    let mut cur = String::new();
    let mut in_d = false;
    let mut in_s = false;
    let chars: Vec<char> = cmd.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        // `2>&1` / `1>&2` 的重定向 `&` **不是**链分隔符，必须原样保留。
        let prev_is_redirect = i > 0 && chars[i - 1] == '>';
        if c == '"' && !in_s {
            in_d = !in_d;
            cur.push(c);
        } else if c == '\'' && !in_d {
            in_s = !in_s;
            cur.push(c);
        } else if !in_d && !in_s && c == '&' && !prev_is_redirect {
            if i + 1 < chars.len() && chars[i + 1] == '&' {
                parts.push((std::mem::take(&mut pending_op), std::mem::take(&mut cur)));
                pending_op = "&&".into();
                i += 1;
            } else {
                parts.push((std::mem::take(&mut pending_op), std::mem::take(&mut cur)));
                pending_op = "&".into();
            }
        } else {
            cur.push(c);
        }
        i += 1;
    }
    parts.push((std::mem::take(&mut pending_op), cur));
    parts
}

pub fn translate_single_unix_command(cmd: &str) -> Result<Option<String>, String> {
    // &&/& 链：逐段递归翻译（段内已无 &），未识别的段原样保留；任一段翻译拒绝则整链拒绝
    if cmd.contains('&') {
        let parts = split_chain(cmd);
        if parts.len() > 1 {
            let mut out = String::new();
            for (op, seg) in parts {
                let seg_t = seg.trim();
                if seg_t.is_empty() {
                    continue;
                }
                let piece = match translate_single_unix_command(seg_t)? {
                    Some(t) => t,
                    None => seg_t.to_string(),
                };
                if !out.is_empty() {
                    out.push_str(&format!(" {op} "));
                }
                out.push_str(&piece);
            }
            return Ok(Some(out));
        }
    }
    let base = cmd.split_whitespace().next().unwrap_or("");
    match base {
        // cat file → type file（cmd 内置；cat 无文件参数时保留原样由 cmd 报错）
        "cat" => Ok(Some(cmd.replacen("cat", "type", 1))),
        // grep "pat" file → findstr /i "pat" file（单命令形态，复用管道翻译的同一解析）
        "grep" => {
            // flag 白名单（-i / -n）—— **与管道翻译路径共用同一实现**
            let (ci, numbered, pat) = grep_flags_and_pat(cmd)?;
            if pat.contains('|') {
                return Err("grep 正则含 `|`（交替）无法安全翻译为 Windows findstr —— \
                     请改用 run 的 script 参数跑 python，或分开多次 grep"
                    .into());
            }
            if pat.is_empty() {
                return Ok(Some("findstr .".into()));
            }
            let mut flags = if ci { " /i".to_string() } else { String::new() };
            if numbered {
                flags.push_str(" /n");
            }
            let rest = cmd
                .split_whitespace()
                .skip(1)
                .skip_while(|t| t.starts_with('-'))
                .collect::<Vec<_>>();
            // findstr 模式在前、文件在后：grep pat file → findstr "pat" file
            let files = rest.iter().skip(1).copied().collect::<Vec<_>>().join(" ");
            let mut out = format!("findstr{flags} \"{}\"", pat.replace('"', "\"\""));
            if !files.is_empty() {
                out.push_str(&format!(" {files}"));
            }
            Ok(Some(out))
        }
        // find . -name "*.rs"：Unix find 语义与 Windows find（/c 计数）完全不同——
        "find" => Err(
            "find 是 Unix 命令，Windows 的 find 语义完全不同（/c 是计数不是查找）。\
             查找文件请改用：dir /s /b \"*.rs\"（递归列出），或用 search 工具搜内容。"
                .into(),
        ),
        // sed/awk：无 Windows 等价，确定性拒绝 + 教学
        "sed" | "awk" => Err("sed/awk 是 Unix 文本处理命令，Windows 无等价。\
             请改用 python（如 python -c \"...\"）或 powershell Select-String/ForEach-Object。"
            .into()),
        _ => Ok(None),
    }
}

/// 解析并**校验** grep 的 flag —— 只放行能**无损**翻成 findstr 的两个
fn grep_flags_and_pat(seg: &str) -> Result<(bool, bool, String), String> {
    let mut ci = false;
    let mut numbered = false;
    let mut pat = String::new();
    for t in seg.split_whitespace().skip(1) {
        if t.starts_with('-') {
            for c in t.trim_start_matches('-').chars() {
                match c {
                    'i' => ci = true,
                    'n' => numbered = true,
                    'A' | 'B' | 'C' => {
                        return Err(format!(
                            "grep -{c}（上下文行）在 findstr 无等价，静默丢弃会改变语义 —— \
                             改用 read 工具读该文件的相关行段，或 run 的 script 参数跑 python。\
                             原段：{seg}"
                        ))
                    }
                    'r' | 'R' => {
                        return Err(format!(
                            "grep -{c}（递归搜索）在 findstr 无等价 —— 丢掉它只会搜**当前目录**、\
                             给出残缺结果。改用 run 的 script 参数（python 的 rglob）或 search 工具。\
                             原段：{seg}"
                        ))
                    }
                    'c' => {
                        return Err(format!(
                            "grep -c（计数）在 findstr 无等价 —— findstr 的 `/c:` 是「字面串」、\
                             同名不同义。改用 run 的 script 参数。原段：{seg}"
                        ))
                    }
                    'l' => {
                        return Err(format!(
                            "grep -l（只列文件名）在 findstr 无等价 —— 改用 run 的 script 参数。\
                             原段：{seg}"
                        ))
                    }
                    other => {
                        return Err(format!(
                            "grep 的 `-{other}` 未登记（本层只放行 -i / -n）—— \
                             要更多检索能力请用 run 的 script 参数跑 python，或 search 工具。\
                             原段：{seg}"
                        ))
                    }
                }
            }
            continue;
        }
        pat = t.trim_matches('"').trim_matches('\'').to_string();
        break;
    }
    Ok((ci, numbered, pat))
}

/// 解析 `grep -i "pat"` / `grep -in pat` → (忽略大小写?, 模式)
pub fn extract_grep_pat(seg: &str) -> (bool, String) {
    let mut ci = false;
    let mut pat = String::new();
    for t in seg.split_whitespace().skip(1) {
        if t.starts_with('-') {
            if t.contains('i') {
                ci = true;
            }
            continue;
        }
        pat = t.trim_matches('"').trim_matches('\'').to_string();
        break;
    }
    (ci, pat)
}

/// Unix 分号 `;` → cmd 的 `&`（引号外才翻译；`;` 在 cmd 下不是分隔符，会被当参数——
pub fn translate_semicolons(cmd: &str) -> String {
    let mut out = String::with_capacity(cmd.len());
    let mut quote: Option<char> = None;
    for c in cmd.chars() {
        match c {
            q @ ('"' | '\'') => {
                if quote == Some(q) {
                    quote = None;
                } else if quote.is_none() {
                    quote = Some(q);
                }
                out.push(c);
            }
            ';' if quote.is_none() => out.push_str(" & "),
            _ => out.push(c),
        }
    }
    out
}

/// 提取 Select-String 的匹配模式：优先 `-Pattern "a|b"` 的值，回退第一个引号内容。
pub fn extract_select_string_pat(seg: &str) -> String {
    if let Some(idx) = seg.find("-Pattern") {
        let rest = &seg[idx + "-Pattern".len()..];
        for t in rest.split_whitespace() {
            let t = t.trim_matches('"').trim_matches('\'');
            if !t.is_empty() && !t.starts_with('-') {
                return t.to_string();
            }
        }
    }
    if let Some(start) = seg.find('"') {
        if let Some(end) = seg[start + 1..].find('"') {
            return seg[start + 1..start + 1 + end].to_string();
        }
    }
    String::new()
}

/// 把命令里**看起来是 Windows 路径**的反斜杠片段改成正斜杠，供 **Git Bash** 执行。
pub fn normalize_backslash_paths_for_bash(cmd: &str) -> String {
    fn is_sep(c: char) -> bool {
        matches!(c, ' ' | '\t' | ';' | '&' | '|')
    }
    fn is_path_char(c: char) -> bool {
        c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '~' | '$' | '+' | '@')
    }
    // 前缀形态：① 盘符 ② UNC ③ 相对 `.\` `..\` ④ 根相对 `\`
    fn has_prefix(w: &str) -> bool {
        let b = w.as_bytes();
        if b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\' {
            return true;
        }
        if w.starts_with("\\\\") {
            return true;
        }
        if w.starts_with(".\\") || w.starts_with("..\\") {
            return true;
        }
        if w.starts_with('\\') && !w.starts_with("\\\\") {
            if let Some(c) = w[1..].chars().next() {
                return is_path_char(c);
            }
        }
        false
    }
    // ⑤ 裸相对：整词由 ≥2 段路径字符用 `\` 连接（且不含正斜杠，避免误伤 sed 的 s/a\/b/）
    fn is_bare_relative(w: &str) -> bool {
        if !w.contains('\\') || w.contains('/') {
            return false;
        }
        let segs: Vec<&str> = w.split('\\').collect();
        segs.len() >= 2 && segs.iter().all(|s| !s.is_empty() && s.chars().all(is_path_char))
    }
    // `prefix_only`（双引号内）：只认明确前缀 —— 双引号内 bash 仅对 `\$ \` \" \\ \换行` 转义，
    fn fix_word(w: &str, prefix_only: bool) -> String {
        if let Some((lhs, rhs)) = w.split_once('=') {
            if has_prefix(rhs) || (!prefix_only && is_bare_relative(rhs)) {
                return format!("{lhs}={}", rhs.replace('\\', "/"));
            }
        }
        if has_prefix(w) || (!prefix_only && is_bare_relative(w)) {
            return w.replace('\\', "/");
        }
        w.to_string()
    }

    let chars: Vec<char> = cmd.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(cmd.len() + 8);
    let mut i = 0;
    while i < n {
        let c = chars[i];
        // 单引号：bash 不解析其中的反斜杠（常是 grep/sed 正则），整段原样保留
        if c == '\'' {
            out.push(c);
            i += 1;
            while i < n && chars[i] != '\'' {
                out.push(chars[i]);
                i += 1;
            }
            if i < n {
                out.push(chars[i]);
                i += 1;
            }
            continue;
        }
        // 双引号：逐词做**前缀**归一（见 prefix_only 说明）
        if c == '"' {
            let mut inner = String::new();
            let mut j = i + 1;
            while j < n && chars[j] != '"' {
                inner.push(chars[j]);
                j += 1;
            }
            let fixed: String = inner
                .split(' ')
                .map(|w| fix_word(w, true))
                .collect::<Vec<_>>()
                .join(" ");
            out.push('"');
            out.push_str(&fixed);
            if j < n {
                out.push('"');
                j += 1;
            }
            i = j;
            continue;
        }
        // 裸词：到分隔符止
        if !is_sep(c) {
            let mut word = String::new();
            while i < n && !is_sep(chars[i]) && chars[i] != '"' && chars[i] != '\'' {
                word.push(chars[i]);
                i += 1;
            }
            out.push_str(&fix_word(&word, false));
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// argv 数组 token 合并进 command 的字形规则（引号 + 字形合并；斜杠归一下沉执行层）。
pub fn merge_argv_tokens(cmd: &str, tokens: &[String]) -> String {
    let mut merged = cmd.to_string();
    for tok in tokens {
        let t = tok.trim();
        if t.is_empty() {
            continue;
        }
        let need_quote =
            t.contains('/') || t.contains(':') || t.contains(char::is_whitespace);
        if need_quote && !(t.starts_with('"') && t.ends_with('"')) {
            merged.push_str(&format!(" \"{}\"", t.replace('"', "\\\"")));
        } else {
            merged.push_str(&format!(" {t}"));
        }
    }
    // 斜杠归一**下沉到执行层**（cmd_tools::run 按通道决定方向）：这里只做字形合并，
    merged
}

pub fn normalize_drive_slash(cmd: &str) -> String {
    let mut out = String::with_capacity(cmd.len() + 8);
    let chars: Vec<char> = cmd.chars().collect();
    let mut i = 0;
    // in_quote: 双引号内；pattern_quote: 该引号是 findstr /c: 或 /g: 的模式串；
    let mut in_quote = false;
    let mut pattern_quote = false;
    let mut in_backtick = false;
    let mut in_single = false;
    while i < chars.len() {
        let ch = chars[i];
        if ch == '"' {
            if !in_quote {
                // 开引号：向前看（跳过空白）是否紧跟 /c: 或 /g: → 模式串
                let mut j = i;
                let mut prev = String::new();
                while j > 0 {
                    j -= 1;
                    let c = chars[j];
                    if c == ' ' || c == '\t' {
                        break;
                    }
                    prev.insert(0, c);
                }
                let lower = prev.to_ascii_lowercase();
                pattern_quote = lower.ends_with("/c:") || lower.ends_with("/g:");
            } else {
                // 闭引号：模式串豁免随之结束（否则后面整段都被当成模式串跳过归一）
                pattern_quote = false;
            }
            in_quote = !in_quote;
            out.push(ch);
            i += 1;
            continue;
        }
        if ch == '`' && !in_single {
            in_backtick = !in_backtick;
            out.push(ch);
            i += 1;
            continue;
        }
        if ch == '\'' && !in_backtick {
            in_single = !in_single;
            out.push(ch);
            i += 1;
            continue;
        }
        let protected = pattern_quote || in_backtick || in_single;
        // 协议豁免：当前位置前两字符是 "//"（即 http:// 之类）→ 不动
        let in_url = i >= 2
            && (chars[i - 1] == '/')
            && (chars[i - 2] == ':' || chars[i - 2] == '/');
        // 检测盘符正斜杠 X:/ 模式（前一个字符须为分隔符/引号/行首）
        let at_token_start = i == 0
            || chars[i - 1] == ' '
            || chars[i - 1] == '"'
            || chars[i - 1] == '&'
            || chars[i - 1] == '\''
            || chars[i - 1] == ';';
        if !protected
            && !in_url
            && ch.is_ascii_alphabetic()
            && i + 2 < chars.len()
            && chars[i + 1] == ':'
            && chars[i + 2] == '/'
            && at_token_start
        {
            // 前推写出 "X:\"，后续 '/' 全部转 '\' 直到分隔符
            out.push(ch);
            out.push(':');
            out.push('\\');
            i += 3;
            while i < chars.len()
                && chars[i] != ' '
                && chars[i] != '"'
                && chars[i] != '&'
                && chars[i] != ';'
            {
                if chars[i] == '/' {
                    out.push('\\');
                } else {
                    out.push(chars[i]);
                }
                i += 1;
            }
            continue;
        }
        out.push(ch);
        i += 1;
    }
    out
}

#[cfg(test)]
#[path = "cmd_translate_tests.rs"]
mod cmd_translate_tests;

#[cfg(test)]
#[path = "cmd_translate_shape_tests.rs"]
mod cmd_shape_tests;
