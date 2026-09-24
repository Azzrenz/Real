//! ask —— 模型主动向用户提问的决策门。

use serde_json::{json, Value};

use crate::mcp::registry::BuiltinTool;

#[allow(dead_code)]
pub struct AskTool;

#[async_trait::async_trait]
impl BuiltinTool for AskTool {
    fn name(&self) -> &'static str {
        "ask"
    }

    fn description(&self) -> &'static str {
        "向用户提一个需要他拍板的问题，然后挂起等他选。这不是聊天工具，是决策门：\
         只在岔路会影响你接下来怎么做、且自己推不出来时才用。\
         输入 {question, options:[{label, detail, recommended?}]}；options 2~4 项，\
         每项 detail 必填——写清选它会怎样、代价是什么；没说明的选项等于让用户猜。\
         用法：把主张标在 recommended 上（每问只允许一项），带着方案问——\
         只摆选项不作判断，等于把思考退回给用户。\
         不要用它：能从上下文推断的直接做；只想通报进展的走旁白，不要弹窗打断。一次只问一件事。\
         行为：调用后本轮挂起等待，最长 120 秒；超时按未作答处理。\
         返回 JSON 结构 {kind:\"user_choice\", data:{question, chosen, note?, timed_out}}。\
         读取规则按字段分三种：\
         (a) chosen 非空 → 用户选了你给的某一项，按它走；\
         (b) chosen 为空且 note 非空 → 用户否掉了你给的全部选项，note 里就是他的主张，\
         按 note 走，不要回头再推一次你原来的选项；\
         (c) timed_out=true → 没人答，按 recommended 那一项继续，\
         并在旁白里说明你按哪条走的。\
         note 是用户在弹窗里自己写的一句话（他也可以只写它、不选任何选项），\
         他没写时该字段不出现。"
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "要用户拍板的问题，一句话；一次只问一件事"
                },
                "options": {
                    "type": "array",
                    "minItems": 2,
                    "maxItems": 4,
                    "description": "2~4 个候选项（少于 2 项就别问，直接做）；每项必带 detail",
                    "items": {
                        "type": "object",
                        "properties": {
                            "label": {
                                "type": "string",
                                "description": "选项短标题，写清做什么；不要写「方案一」这类空名"
                            },
                            "detail": {
                                "type": "string",
                                "description": "选它会怎样、代价是什么"
                            },
                            "recommended": {
                                "type": "boolean",
                                "description": "你主张这一项；每问只允许一项为 true"
                            }
                        },
                        "required": ["label", "detail"]
                    }
                }
            },
            "required": ["question", "options"]
        })
    }

    fn annotations(&self) -> Value {
        json!({"read_only": true, "destructive": false, "idempotent": false})
    }

    /// 不会被执行：`ask` 在调工具前就被编排层拦截了。留着它是为了满足 trait——
    async fn run(&self, _args: Value) -> Result<String, String> {
        Err("ask 应由编排层拦截，不应到达工具层（检查 execute_one 的 ask 分支）".into())
    }
}

/// 把 ask 入参转成一次确认请求。参数不合规返回原因——那是模型看得懂的重试依据。
pub fn spec_from_args(args: &Value) -> Result<crate::confirm::ConfirmSpec, String> {
    let question = args
        .get("question")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if question.is_empty() {
        return Err("question 不能为空".into());
    }
    let raw = args
        .get("options")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if raw.len() < 2 {
        return Err("options 至少 2 项——只有一条路时不叫选择，直接做".into());
    }
    let mut options = Vec::new();
    for o in raw.iter().take(4) {
        let label = o.get("label").and_then(|v| v.as_str()).unwrap_or("").trim();
        let detail = o.get("detail").and_then(|v| v.as_str()).unwrap_or("").trim();
        if label.is_empty() || detail.is_empty() {
            return Err("每个选项都要有 label 与 detail（说明选它会发生什么）".into());
        }
        options.push(crate::confirm::ConfirmOption {
            label: label.to_string(),
            detail: detail.to_string(),
            recommended: o
                .get("recommended")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        });
    }
    Ok(crate::confirm::ConfirmSpec {
        action: "ask".into(),
        target: question.to_string(),
        impact: String::new(),
        risk_label: "info".into(),
        options,
    })
}
