//! 契约校验闸门（执行前）：validate(schema, args) 校验失败不执行，返回两层错误码

use regex;
use serde_json::{Map, Value};
use crate::mcp::envelope::ToolEnvelope;

/// 标准化工具错误（统一进 ToolEnvelope.meta + content）
#[derive(Debug, Clone)]
pub struct ToolError {
    pub code: &'static str,
    pub message: String,
    pub suggestion: Option<String>,
    /// 坑位 J54：是否可重试（业界 ToolError.retryable）——
    pub retryable: bool,
    /// 错误类型枚举（业界 ToolError.type 对齐）
    pub err_type: &'static str,
}

impl ToolError {
    pub fn missing(field: &str) -> Self {
        Self {
            code: "MISSING_PARAM",
            message: format!("缺少必填参数: {field}"),
            suggestion: Some(format!("请补充 {field} 参数")),
            retryable: false,
            err_type: "validation_error",
        }
    }

    pub fn invalid(field: &str, expected: &str) -> Self {
        Self {
            code: "INVALID_PARAM",
            message: format!("参数 {field} 不合法：应为 {expected}"),
            suggestion: Some(format!("请检查 {field} 的格式，应为 {expected}")),
            retryable: false,
            err_type: "validation_error",
        }
    }

    /// 契约确定性：类型错误必须带"实际收到了什么"——只说"应为 string"不说实收类型，
    pub fn invalid_with_actual(field: &str, expected: &str, actual: &serde_json::Value) -> Self {
        let (kind, hint) = describe_value(actual);
        Self {
            code: "INVALID_PARAM",
            message: format!("参数 {field} 不合法：应为 {expected}，实际收到 {kind}{hint}"),
            suggestion: Some(format!(
                "按 {expected} 重新构造 {field}。多行文本在 JSON 里换行写 \\n；若转义反复失败，改用 write 工具整文件覆写"
            )),
            retryable: false,
            err_type: "validation_error",
        }
    }

    /// 内容字段缺失/为空（write.content 等）：schema 的"长度 ≥ 1"模型看不懂，
    pub fn empty_content(field: &str) -> Self {
        Self {
            code: "EMPTY_CONTENT",
            message: format!("{field} 缺失或为空：必须是完整可写的最终文件内容"),
            suggestion: Some("先 read 目标文件拿到真实内容，再提交完整新版内容".to_string()),
            retryable: false,
            err_type: "validation_error",
        }
    }

    /// modify 意图模式：缺定位但后端已自动定位到目标（附真实代码段）
    pub fn modify_intent_needs_anchor(desc: &str, kind: &str, name: &str, line: usize, snippet: &str) -> Self {
        Self {
            code: "MODIFY_INTENT_NEEDS_ANCHOR",
            message: format!(
                "modify 收到意图「{desc}」但 change_spec 缺少定位（find/line/replace）。\n\
                 【后端已自动定位到目标】{kind} `{name}`（第 {line} 行起，无需再 read 找位置）：\n\
                 ```\n{snippet}\n```"
            ),
            suggestion: Some(
                "基于以上【真实代码】在 change_spec 填定位：block=find（逐字复制上述片段且全文唯一）+replace；\
                 line=行号+replace；write=replace（全新全文）。description 只是意图说明，不能替代定位".to_string(),
            ),
            retryable: true,
            err_type: "validation_error",
        }
    }

    /// modify 意图模式：识别为整文件覆写/重新生成 → 应改用 write
    pub fn modify_intent_whole_file(desc: &str, path: &str) -> Self {
        Self {
            code: "MODIFY_INTENT_WHOLE_FILE",
            message: format!(
                "modify 收到意图「{desc}」——这是**整文件覆写/重新生成**操作（含覆写/整文件/重新生成关键词）。\n\
                 modify 只能改文件中的**一段**（find/line 定位）。"
            ),
            suggestion: Some(format!(
                "改用 write {{path: {path}, content: <完整的新文件内容>}}——content 必须是完整的最终\
                 文件内容（整份 JSON/代码），不是改动意图描述；目标文件不存在时同样用 write 创建"
            )),
            retryable: true,
            err_type: "validation_error",
        }
    }

    /// modify 意图模式：后端自动定位失败（意图里的实体名文件里找不到）
    pub fn modify_intent_locate_failed(desc: &str, no_file: bool) -> Self {
        Self {
            code: "MODIFY_INTENT_LOCATE_FAILED",
            message: format!(
                "modify 收到意图「{desc}」但缺少**定位信息**——后端自动定位失败（意图描述里的类/方法名\
                 在文件中找不到），无法知道要改哪段。\n{}",
                if no_file {
                    "⚠️ 且你**连目标文件都没指定**（缺 path/file）——必须先在 write 的 path / modify 的 file 字段指定要改/生成哪个文件"
                } else {
                    "（已指定目标文件，但意图里的实体名在文件中找不到——先 read 确认真实函数/类名）"
                }
            ),
            suggestion: Some(
                "两步：① 先 read 目标文件（可引用前序 #En）；② 在 change_spec 填定位：\
                 block=find（来自 read 的真实原文片段）+replace；line=行号+replace；write=replace（全新全文）".to_string(),
            ),
            retryable: true,
            err_type: "validation_error",
        }
    }

    /// 工具报错 + 目标文件真实内容预览（错误包装：模型下一次直接抄真实片段，不再凭记忆拼）
    pub fn with_file_preview(err: &str, path: &str, preview: &str, chars: usize) -> Self {
        Self {
            code: "TOOL_ERROR_WITH_PREVIEW",
            message: format!(
                "{err}\n\n【后端已自动读取目标文件】{path} 当前内容（前 {chars} 字符，\
                 可直接复制其中真实片段作为 find，或数行号作为 line）：\n\n{preview}"
            ),
            suggestion: None,
            retryable: true,
            err_type: "validation_error",
        }
    }

    /// 同 `with_file_preview`，但**保留内层真实错误码**。
    pub fn with_file_preview_as(
        code: &'static str,
        err: &str,
        path: &str,
        preview: &str,
        chars: usize,
    ) -> Self {
        let mut s = Self::with_file_preview(err, path, preview, chars);
        s.code = code;
        // retryable 也跟码走：老包装一律写死 true，等于"路径不存在"也能无脑重试。
        s.retryable = code_retryable(code) || code_retryable_with_change(code);
        // err_type 同理：老包装一律写死 validation_error，
        s.err_type = if matches!(code, "MODIFY_PATH_NOT_FOUND") {
            "domain_error"
        } else {
            "validation_error"
        };
        s
    }

    /// modify：file 是占位符描述而非真实路径（文案归位）
    pub fn modify_file_placeholder() -> Self {
        Self {
            code: "MODIFY_FILE_PLACEHOLDER",
            message: "modify 的 file 是**占位符描述**（如「待定位的源码文件」），不是真实路径。".to_string(),
            suggestion: Some("先 read 定位真实文件（或用 list 按 basename 找），再用其绝对路径或 basename 发起 modify".to_string()),
            retryable: true,
            err_type: "validation_error",
        }
    }

    /// modify：find 是占位符/空文本（文案归位）
    pub fn modify_find_empty() -> Self {
        Self {
            code: "MODIFY_FIND_EMPTY",
            message: "modify 的 find 是占位符/空文本（不是文件真实内容）。".to_string(),
            suggestion: Some("先 read 目标文件确定真实改动点，再从读到的带行号原文里抄写 find/replace".to_string()),
            retryable: true,
            err_type: "validation_error",
        }
    }

    /// modify：**参数没填实**（replace 缺失 / null / 空 / 纯占位符）—— 与文件系统无关。
    pub fn modify_param_unfilled(field: &str, got: &str) -> Self {
        Self {
            code: "MODIFY_PARAM_UNFILLED",
            message: format!(
                "modify 的 change_spec.{field} **没有填实**（{got}）。\n\
                 ⚠️ 这是**调用方漏填参数**，与文件系统无关 —— 目标文件存在、路径正确、\
                 目录内容也没问题，**不要去 list / read / 换路径**。"
            ),
            suggestion: Some(format!(
                "在 change_spec 里补齐 {field}：\
                 block 模式 = find（从 read 结果里逐字复制的原文片段）+ replace（改成什么）；\
                 line 模式 = line（行号）+ replace（该行的新内容）；\
                 write 模式 = replace（完整的新全文）。\
                 只想**追加**就给 find 配 insert_after，不用把 replace 留空。"
            )),
            retryable: true,
            err_type: "validation_error",
        }
    }

    /// modify：find 命中但替换无差异（changed:0）——定位/方案问题，非参数问题
    pub fn modify_no_change(path: &str, head: &str) -> Self {
        Self {
            code: "MODIFY_NO_CHANGE",
            message: format!(
                "modify 未产生实际修改（changed:0）：find 匹配到了，但替换后文件无差异。\n\
                 ⚠️ 这是**定位/方案问题**，不是参数问题——retry 同 find/replace 必然再次失败。\n\
                 可能原因：① 插入位置选错（该位置已有类似代码）；② find 匹配到**无关位置**\
                 （文件里有多个相似片段）；③ 你要的修改已被前序步骤做过。\n\
                 目标文件 {path} 前 2000 字符：\n{head}"
            ),
            suggestion: Some(
                "先 read 目标文件相关区域（可引用前序 #En）确认真实代码再换 find/line 定位；\
                 或改用 line 模式（明确行号 + replace 该行新内容）；\
                 或确认修改已存在后 skip（原因进最终报告）".to_string(),
            ),
            retryable: true,
            err_type: "validation_error",
        }
    }

    /// modify：读回校验未通过（verified=false）——已回滚
    pub fn modify_verify_fail(find_gone: bool, replace_present: bool, actual: Option<&str>) -> Self {
        let actual_block = match actual {
            Some(a) if !a.is_empty() => format!(
                "\n替换后该处实际内容（带行号）：\n{a}\n\
                 （拿它跟你的 replace 逐行对比——缩进、转义、多余空格都会导致 replace_present=false）"
            ),
            _ => String::new(),
        };
        Self {
            code: "MODIFY_VERIFY_FAIL",
            message: format!(
                "modify 内容校验未通过（verified=false）：替换后目标内容未按预期出现\
                 （find_gone={find_gone} / replace_present={replace_present}）——\
                 修改可能匹配到无关位置或替换未生效。已自动回滚到备份。{actual_block}"
            ),
            suggestion: Some(
                "对照上面给出的实际内容修正 replace（逐行照抄现况再改），或用 line 模式（明确行号）"
                    .to_string(),
            ),
            retryable: true,
            err_type: "validation_error",
        }
    }

    /// .rs 改动破坏花括号平衡（结构护栏）
    pub fn brace_unbalanced(path: &str, diag: &str) -> Self {
        Self {
            code: "MODIFY_BRACE_UNBALANCED",
            message: format!(
                "modify 结构护栏触发：修改后 {path} 的花括号不平衡（{diag}）——\
                 插入位置/替换内容破坏了代码结构。已自动回滚到备份。"
            ),
            suggestion: Some(
                "① read 该文件拿带行号原文，定位上面给出的行号；\
                 ② 改用 line 模式（明确行号 + replace 该行新内容），避免拼长 find；\
                 ③ 确需块级改写时，find 必须包含函数完整收尾上下文（含最后一个 }）"
                    .to_string(),
            ),
            retryable: true,
            err_type: "validation_error",
        }
    }

    pub fn domain(
        code: &'static str,
        message: impl Into<String>,
        suggestion: Option<&str>,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            suggestion: suggestion.map(|s| s.to_string()),
            // 领域错误默认不可重试（如 OLD_TEXT_MISMATCH 需要改参数而非重试）
            retryable: false,
            err_type: "domain_error",
        }
    }
}

/// 从工具错误文本里解出**内层真实错误码**。
pub fn inner_code(err: &str) -> Option<String> {
    let s = err.trim_start();
    let s = s.strip_prefix('[')?;
    let end = s.find(']')?;
    let code = s[..end].trim();
    if code.is_empty()
        || !code
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
    {
        return None;
    }
    Some(code.to_string())
}

/// 错误 → 信封文本（错误码进 meta 由 registry 填充）。
pub fn err_text(e: &ToolError) -> String {
    let mut msg = format!("[{}] {}", e.code, e.message);
    if let Some(s) = &e.suggestion {
        msg.push_str(&format!("\n建议: {s}"));
    }
    msg
}

/// 校验结果
pub type ValidateResult = Result<Value, ToolError>;

/// 执行前闸门：校验 args 是否符合 inputSchema（JSON Schema 子集）
pub fn validate(schema: &Value, args: &Value) -> ValidateResult {
    let schema_obj = match schema.as_object() {
        Some(o) => o,
        None => return Ok(args.clone()),
    };

    // 顶层类型
    if let Some(t) = schema_obj.get("type").and_then(|v| v.as_str()) {
        if t != "object" {
            return Err(ToolError::invalid("(顶层)", &format!("{t}")));
        }
    }

    let args_obj = match args {
        Value::Object(o) => o,
        _ => return Err(ToolError::invalid("(顶层)", "对象")),
    };

    let mut cleaned = Map::new();
    let props = schema_obj.get("properties").and_then(|v| v.as_object());

    // 1) 必填存在性
    if let Some(required) = schema_obj.get("required").and_then(|v| v.as_array()) {
        for f in required {
            let name = f.as_str().unwrap_or_default();
            if !args_obj.contains_key(name) {
                return Err(ToolError::missing(name));
            }
        }
    }

    // 1.5) anyOf 条件必填（契约边界下沉）：声明"至少满足 oneOf 分支"——
    if let Some(any_of) = schema_obj.get("anyOf").and_then(|v| v.as_array()) {
        let mut satisfied = false;
        let mut branch_summary = Vec::new();
        for branch in any_of {
            if let Some(br) = branch.get("required").and_then(|x| x.as_array()) {
                let all_present = br.iter().all(|f| {
                    let n = f.as_str().unwrap_or_default();
                    args_obj.contains_key(n)
                });
                if all_present {
                    satisfied = true;
                    break;
                }
                branch_summary.push(
                    br.iter()
                        .map(|f| f.as_str().unwrap_or_default().to_string())
                        .collect::<Vec<_>>()
                        .join("+"),
                );
            }
        }
        if !satisfied {
            return Err(ToolError::missing(&format!(
                "（anyOf）需满足任一分支: {}",
                branch_summary.join(" 或 ")
            )));
        }
    }

    // 2) 逐字段校验 + 清洗
    if let Some(props) = props {
        for (name, field_schema) in props {
            match args_obj.get(name) {
                None => {
                    // 有默认值则注入
                    if let Some(d) = field_schema.get("default") {
                        cleaned.insert(name.clone(), d.clone());
                    }
                }
                Some(v) => {
                    // 数值越界 → 夹到边界（宽容执行的值维度，与上面 enum 宽容化同源）
                    let v = clamp_numeric(field_schema, v.clone());
                    check_field(name, field_schema, &v)?;
                    cleaned.insert(name.clone(), v);
                }
            }
        }
    }

    // 3) 额外属性（additionalProperties: false）
    if schema_obj
        .get("additionalProperties")
        .and_then(|v| v.as_bool())
        == Some(false)
    {
        for k in args_obj.keys() {
            if k == "reason" || cleaned.contains_key(k) {
                continue;
            }
            return Err(ToolError::invalid(
                k,
                "额外字段未在 schema 中声明（additionalProperties=false）",
            ));
        }
    }

    Ok(Value::Object(cleaned))
}

/// 值的类型与摘要（错误信封用：给模型"实际收到什么"的自诊断锚点）
fn describe_value(v: &serde_json::Value) -> (String, String) {
    match v {
        serde_json::Value::Null => (
            "null".into(),
            "（很可能字段被漏填或 JSON 转义断裂——检查该参数是否真的写进了请求）".into(),
        ),
        serde_json::Value::Object(o) => {
            let keys: Vec<String> = o.keys().take(6).cloned().collect();
            let more = if o.len() > 6 { "…" } else { "" };
            (
                "object".into(),
                format!("（键: {}{more}——字符串内容应直接作为值，不要再包一层结构）", keys.join(", ")),
            )
        }
        serde_json::Value::Array(a) => ("array".into(), format!("（{} 项——应为单个字符串，不是数组）", a.len())),
        serde_json::Value::String(s) => {
            let prev: String = s.chars().take(60).collect();
            let ell = if s.chars().count() > 60 { "…" } else { "" };
            ("string".into(), format!("（值开头: {prev}{ell}）"))
        }
        serde_json::Value::Number(n) => ("number".into(), format!("（值 {n}——应为字符串，数字要加引号）")),
        serde_json::Value::Bool(b) => ("boolean".into(), format!("（值 {b}——应为字符串）")),
    }
}

/// 数值越界 → **夹到边界**（架构决策·宽容执行的值维度；与 `check_field` 里的 enum 宽容化同源）。
fn clamp_numeric(fs: &Value, v: Value) -> Value {
    let Some(n) = v.as_i64() else {
        return v;
    };
    let lo = fs.get("minimum").and_then(|x| x.as_i64());
    let hi = fs.get("maximum").and_then(|x| x.as_i64());
    if lo.is_none() && hi.is_none() {
        return v;
    }
    let mut want = n;
    if let Some(l) = lo {
        if want < l {
            want = l;
        }
    }
    if let Some(h) = hi {
        if want > h {
            want = h;
        }
    }
    if want == n {
        return v;
    }
    tracing::info!(from = n, to = want, "参数越界已夹紧（宽容执行·值维度）");
    Value::Number(want.into())
}

fn check_field(name: &str, fs: &Value, v: &Value) -> Result<(), ToolError> {
    let fs = match fs.as_object() {
        Some(o) => o,
        None => return Err(ToolError::invalid(name, "对象")),
    };
    // type
    if let Some(t) = fs.get("type").and_then(|x| x.as_str()) {
        let ok = match t {
            "string" => v.is_string(),
            "integer" => v.as_i64().is_some(),
            "number" => v.is_number(),
            "boolean" => v.is_boolean(),
            "array" => v.is_array(),
            "object" => v.is_object(),
            _ => true,
        };
        if !ok {
            return Err(ToolError::invalid_with_actual(name, t, v));
        }
    }

    // enum（架构决策·枚举宽容化）：**enum 不匹配不再拒绝**——保留原始值，
    if let Some(enum_arr) = fs.get("enum").and_then(|x| x.as_array()) {
        if !enum_arr.contains(v) {
            tracing::debug!(field = name, value = ?v, "enum 宽容：非枚举值放行，由工具层解释");
        }
    }

    // 字符串约束
    if let Some(s) = v.as_str() {
        if let Some(min) = fs.get("minLength").and_then(|x| x.as_u64()) {
            if (s.chars().count() as u64) < min {
                return Err(ToolError::invalid(name, &format!("长度 ≥ {min}")));
            }
        }
        if let Some(max) = fs.get("maxLength").and_then(|x| x.as_u64()) {
            if (s.chars().count() as u64) > max {
                return Err(ToolError::invalid(name, &format!("长度 ≤ {max}")));
            }
        }
        // （契约边界下沉）：pattern 格式校验（如 url 的 ^https?://）。
        if let Some(pat) = fs.get("pattern").and_then(|x| x.as_str()) {
            if let Ok(re) = regex::Regex::new(pat) {
                if !re.is_match(s) {
                    return Err(ToolError::invalid(name, &format!("格式应匹配 {pat}")));
                }
            }
        }
    }

    // 数值约束
    if let Some(n) = v.as_f64() {
        if let Some(min) = fs.get("minimum").and_then(|x| x.as_f64()) {
            if n < min {
                return Err(ToolError::invalid(name, &format!("≥ {min}")));
            }
        }
        if let Some(max) = fs.get("maximum").and_then(|x| x.as_f64()) {
            if n > max {
                return Err(ToolError::invalid(name, &format!("≤ {max}")));
            }
        }
    }

    // 数组约束
    if let Some(arr) = v.as_array() {
        if let Some(min) = fs.get("minItems").and_then(|x| x.as_u64()) {
            if (arr.len() as u64) < min {
                return Err(ToolError::invalid(name, &format!("至少 {min} 项")));
            }
        }
        if let Some(max) = fs.get("maxItems").and_then(|x| x.as_u64()) {
            if (arr.len() as u64) > max {
                return Err(ToolError::invalid(name, &format!("至多 {max} 项")));
            }
        }
        if let Some(item_schema) = fs.get("items") {
            for (i, item) in arr.iter().enumerate() {
                check_field(&format!("{name}[{i}]"), item_schema, item)?;
            }
        }
    }

    // 嵌套对象
    if let (Some(o), Some(inner)) = (
        v.as_object(),
        fs.get("properties").and_then(|x| x.as_object()),
    ) {
        if let Some(required) = fs.get("required").and_then(|x| x.as_array()) {
            for f in required {
                if !o.contains_key(f.as_str().unwrap_or_default()) {
                    return Err(ToolError::missing(&format!(
                        "{name}.{}",
                        f.as_str().unwrap_or_default()
                    )));
                }
            }
        }
        for (k, sub) in inner {
            if let Some(sv) = o.get(k) {
                check_field(&format!("{name}.{k}"), sub, sv)?;
            }
        }
    }

    Ok(())
}

/// 从工具错误文本中提取错误码 meta（registry 错误信封用）
pub fn extract_error_meta(err_text: &str) -> Option<serde_json::Value> {
    let t = err_text.trim();
    if t.starts_with('[') {
        if let Some(end) = t.find(']') {
            let code = &t[1..end];
            if code.chars().all(|c| c.is_ascii_uppercase() || c == '_') {
                return Some(serde_json::json!({"error_code": code}));
            }
        }
    }
    None
}

/// 领域错误码 → 是否可重试（坑位 J54：Replanner 决策依据）
pub fn code_retryable(code: &str) -> bool {
    matches!(
        code,
        // A. 瞬态可重试：**原样重跑有意义的**——超时 / 文件 IO 瞬态 / 网络瞬断 / 进程启动。
        "TIMEOUT"
            | "READ_FAILED"
            | "WRITE_FAILED"
            | "NETWORK_ERROR"
            | "DB_CONNECT_FAIL"
            | "EXEC_FAILED"
            | "TEST_FAILED"
    )
}

/// 错误码 → 建议 next_action（信号契约 v1 定稿，§五）。
pub fn code_next_action(code: &str) -> &'static str {
    if code_retryable(code) {
        return "retry";
    }
    match code {
        // 写操作失败需先验证再继续（改完没生效，重试前先确认现状）
        "INTENT_MISMATCH" | "VERIFY_FAILED" | "VERIFY_TARGET_MISSING" | "POST_EDIT_INVALID"
        | "WRITE_VERIFY_FAILED" | "WRITE_VERIFY_READ_FAILED" | "POST_EDIT_VERIFY_READ_FAILED"
        | "SECURITY_PATTERN" => "verify",
        // 需要用户确认的（弹窗待批 / 权限 / 敏感）
        "WRITE_REQUIRES_CONFIRM" | "EDIT_REQUIRES_CONFIRM" | "PERMISSION_DENIED"
        | "SENSITIVE_FILE" => "ask_user",
        // ── modify 专属：模型**自己就能修好**的三种，不必惊动用户 ──────────
        "MODIFY_PARAM_UNFILLED" | "MODIFY_ANCHOR_NOT_FOUND" | "MODIFY_ANCHOR_AMBIGUOUS" => {
            "retry"
        }
        // 路径不存在：模型无从凭空得知正确路径，与既有 NOT_FOUND 保持同处置
        "MODIFY_PATH_NOT_FOUND" => "ask_user",
        // run 命令失败：模型读 stdout/stderr 后改参数重试（原样重跑无意义）。
        "RUN_FAILED" => "retry",
        // 其余领域错误默认 ask_user（改参数/换工具/问用户）
        _ => "ask_user",
    }
}

/// 错误码是否属于"模型可自行调整后重试"（供教学/Replanner 区分"换姿势"与"问用户"）。
pub fn code_retryable_with_change(code: &str) -> bool {
    // modify 透传出来的专属码：模型改参数 / 换锚点后即可重试，无需用户介入。
    if matches!(
        code,
        "MODIFY_PARAM_UNFILLED"
            | "MODIFY_ANCHOR_NOT_FOUND"
            | "MODIFY_ANCHOR_AMBIGUOUS"
            | "MODIFY_PATH_NOT_FOUND"
            // run：命令退出非零——**改命令/改代码**后重试，不是原样重跑
            | "RUN_FAILED"
    ) {
        return true;
    }
    matches!(
        code,
        // 路径/参数/命令形态错误：模型改参数/换工具后可重试，无需用户介入
        "MISSING_PARAM"
            | "INVALID_PARAM"
            | "NOT_FOUND"
            | "PATH_NOT_FOUND"
            | "PARENT_NOT_FOUND"
            | "RELATIVE_PATH"
            | "IS_DIRECTORY"
            | "NOT_DIRECTORY"
            | "COMMAND_NOT_ALLOWED"
            | "UNIX_CMD_NOT_TRANSLATABLE"
            | "COMMAND_PLACEHOLDER"
            | "OLD_TEXT_MISMATCH"
            | "EDIT_CONFLICT"
            | "LINE_NOT_FOUND"
            | "MULTI_LINE_REPLACE_SINGLE"
            | "EMPTY_REPLACEMENT"
            | "REPLACEMENT_REQUIRED"
            | "REPLACEMENTS_REQUIRED"
            | "FIND_NOT_FOUND"
            | "SEARCH_NO_FILES"
            | "NO_MATCH"
            | "FIND_AMBIGUOUS"
            | "WEB_INVALID_URL"
            | "INVALID_REGEX"
            | "ARGS_REQUIRED"
            | "NO_KEYWORDS"
            |         "OUTPUT_INVALID"
    )
}

// 工具行为契约（发动机效率升级）

// 失败归因分类器（从 workflow.rs 迁入——错误契约部门=错误文案唯一住所）

/// 失败归因结果（category/evidence/advice 三段，供范本填充）
#[derive(Debug)]
pub struct FailureAttribution {
    pub category: String,
    pub evidence: String,
    pub advice: String,
}

/// 码 → 归因（**精确相等**，不走子串）。
fn classify_by_code(code: &str, first: &str) -> Option<FailureAttribution> {
    let cat = |c: &str, a: &str| {
        Some(FailureAttribution {
            category: c.into(),
            evidence: first.to_string(),
            advice: a.into(),
        })
    };
    match code {
        // ── 锚点类：必须来这里拦。"FIND_NOT_FOUND" 含子串 "not_found"，
        "FIND_NOT_FOUND" | "MODIFY_ANCHOR_NOT_FOUND" => cat(
            "锚点未命中",
            "find 片段在文件当前内容里找不到——多半是凭记忆写的。用 read 重读目标区域，\
             从结果里**逐字复制**原文当 find；文件若已被改过，先看它现在长什么样。",
        ),
        "FIND_AMBIGUOUS" | "MODIFY_ANCHOR_AMBIGUOUS" => cat(
            "锚点多义",
            "find 在文件里出现多次、无法唯一定位——**不要**把 find 改短，要**加长**：\
             把相邻几行一并抄进去，让它全文唯一。",
        ),
        // ── 参数类：与文件系统无关，最怕被引去 list/read
        "MODIFY_PARAM_UNFILLED" | "REPLACEMENT_REQUIRED" | "REPLACEMENTS_REQUIRED" => cat(
            "参数未填实",
            "调用方漏填了参数（如 change_spec.replace）——**补齐即可**，与文件系统无关，\
             别去 list / read / 换路径。",
        ),
        // ── run 类：命令退出码非零。
        "RUN_FAILED" => cat(
            "命令失败",
            "命令退出码非零——**先读返回里的 stdout/stderr**（首个报错行已提到 message），\
             照着它的第一条错误改命令或改代码；同一条命令原样重跑通常不会变好。",
        ),
        // ── 路径类：文本分支也认得出，但显式登记可固定 category 名与建议
        "NOT_FOUND" | "PATH_NOT_FOUND" | "MODIFY_PATH_NOT_FOUND" => cat(
            "路径",
            "文件/目录不存在——照着错误里列出的**父目录实际内容**核对名字，\
             或确认该文件是否本该先由 write 创建。",
        ),
        _ => None,
    }
}

/// 按错误文本特征分类 → 类别（证据 + 建议按类别给）
pub fn classify_failure(error_text: &str) -> FailureAttribution {
    let low = error_text.to_lowercase();
    let first = error_text.lines().next().unwrap_or("").chars().take(80).collect::<String>();
    if let Some(code) = inner_code(error_text) {
        if let Some(attr) = classify_by_code(&code, &first) {
            return attr;
        }
    }
    // 分类规则：错误文本特征 → 类别（证据 + 建议按类别给）
    if low.contains("not_found") || low.contains("path_not_found") || low.contains("no such file")
        || low.contains("不存在") || low.contains("path escapes")
    {
        FailureAttribution {
            category: "路径".into(),
            evidence: first,
            advice: "检查路径拼写（连字符/下划线、大小写、盘符）——先用 list 列目录确认真实文件名，再重试。".into(),
        }
    } else if low.contains("import_error") || low.contains("modulenotfound") || low.contains("import error") {
        FailureAttribution {
            category: "导入".into(),
            evidence: first,
            advice: "修改后 import 失败——可能是循环导入或符号缺失，检查 import 链与模块名。".into(),
        }
    } else if low.contains("permissionerror") || low.contains("access is denied") || low.contains("拒绝访问") || low.contains("denied") {
        FailureAttribution {
            category: "权限".into(),
            evidence: first,
            advice: "权限不足——换可写路径或调整目标目录权限。".into(),
        }
    } else if low.contains("unterminated") || low.contains("unmatched") || low.contains("引号") {
        FailureAttribution {
            category: "引号".into(),
            evidence: first,
            advice: "命令含未配对引号或括号——检查转义与配对。".into(),
        }
    } else if low.contains("syntaxerror") || low.contains("syntax error") || low.contains("compile") {
        FailureAttribution {
            category: "语法".into(),
            evidence: first,
            advice: "语法/编译错误——检查括号、缩进与语法。".into(),
        }
    } else if low.contains("refused") || low.contains("timeout") || low.contains("无法连接") || low.contains("connection") {
        FailureAttribution {
            category: "连接".into(),
            evidence: first,
            advice: "连接失败——检查地址/端口/网络可达性。".into(),
        }
    } else {
        FailureAttribution {
            category: "未识别".into(),
            evidence: first,
            advice: "检查工具参数与用法——必要时换一种方式完成目标。".into(),
        }
    }
}

/// 从工具信封提取错误类型（方括号内类型名优先，如 [Errno 2] → "Errno 2"）
pub fn extract_error_type(env: &ToolEnvelope) -> String {
    let text = env
        .content
        .iter()
        .filter_map(|c| c.text.clone())
        .collect::<Vec<_>>()
        .join("
");
    if let Some(pos) = text.find('[') {
        if let Some(end) = text[pos..].find(']') {
            return text[pos + 1..pos + end].to_string();
        }
    }
    text.lines().next().unwrap_or("").chars().take(24).collect()
}

#[cfg(test)]
#[path = "contract_tests.rs"]
mod contract_tests;
