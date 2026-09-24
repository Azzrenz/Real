//! 基座：消息进 → 答案出（三相自旋工作流）

use crate::agent::context::CONVERGENT_SYSTEM_PROMPT;
use crate::error::{AppError, AppResult};
use crate::model::types::{
    to_wire_items, FunctionCall, InputItem, ResponsesRequest, StreamResult, ToolChoice,
};
use crate::mcp::envelope::ToolEnvelope;
use crate::state::AppState;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::Arc;

// ── 调参集中地（全部有默认值，环境变量见注释）──
const MIRROR_MIN_INTERVAL: u32 = 10;
/// 工具默认超时（秒，REAL_TOOL_TIMEOUT_SECS 可覆盖）
const TOOL_TIMEOUT_DEFAULT: u64 = 60;

/// 信封超时（秒）：域地板上浮 + 参数 timeout+15s 余量——工具自身超时先触发，信封只兜死锁。
fn verify_auto_enabled() -> bool {
    match std::env::var("REAL_VERIFY_AUTO") {
        Ok(v) => !matches!(v.trim().to_lowercase().as_str(), "off" | "0" | "false" | "no"),
        Err(_) => true,
    }
}

fn tool_envelope_secs(env_default: u64, tool: &str, args: &serde_json::Value) -> u64 {
    let mut t = env_default;
    // 域工具超时地板：内核不认识具体域，遍历注册表询问——
    for d in crate::domains::registered() {
        if let Some(floor) = d.tool_timeout_floor(tool) {
            t = t.max(floor);
        }
    }
    if let Some(n) = args.get("timeout").and_then(|v| v.as_i64()) {
        if n > 0 {
            t = t.max(n as u64 + 15);
        }
    }
    t
}

// ── 话术资产（prompts 外置）──

/// 域系统提示：classify 命中的域工作流话术；零域命中返回空列表。
fn domain_prompts(goal: &str) -> Vec<&'static str> {
    crate::domains::registered()
        .iter()
        .filter(|d| d.classify(goal))
        .filter_map(|d| d.system_prompt())
        .collect()
}

/// 组合契约：命中域的工作流话术 + 命中的工作规范（内核零域知识）。
fn contract_for(goal: &str) -> String {
    let mut out = domain_prompts(goal).join("\n\n");
    let specs = crate::agent::specs::match_specs(goal);
    if specs.is_empty() {
        return out;
    }
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str(&specs);
    out
}

/// 运行结果（对外只暴露答案；事实走事件流）
pub struct AgentOutcome {
    pub answer: String,
}

// 入口：工作平台唯一的 agent 启动点（chat 路由 / 定时任务共用）

/// 注入物封顶（字符数；env 可调，0=不封顶）——控上下文体积、护前缀缓存。
fn cap_inject(s: String, cap: usize, label: &str) -> String {
    if cap == 0 || s.chars().count() <= cap {
        return s;
    }
    let mut out: String = s.chars().take(cap).collect();
    out.push_str(&format!("\n（{label}超过 {cap} 字已截断：需要完整内容时明确说明具体条目，后端可单独回灌）"));
    tracing::info!(label, cap, "注入物超限已截断");
    out
}

fn env_cap(name: &str, default: usize) -> usize {
    std::env::var(name).ok().and_then(|v| v.parse::<usize>().ok()).unwrap_or(default)
}

fn inject_weight(title: &str) -> usize {
    /// 轻：认得你是谁、有什么凭证、有哪些技能 —— 有就行，不必多占。
    const LIGHT: [&str; 4] = ["用户偏好", "用户画像", "用户凭证", "可用技能"];
    /// 重：**别再犯的错**、**已经定过的结论**、**做过的活** —— 少一句就白跑一轮。
    const HEAVY: [&str; 6] = [
        "失败教训",
        "上轮结论",
        "历史任务记忆",
        "对话脉络",
        "长期记忆",
        "工作日志",
    ];
    if LIGHT.iter().any(|k| title.contains(k)) {
        1
    } else if HEAVY.iter().any(|k| title.contains(k)) {
        3
    } else {
        2
    }
}

/// **按段分额度的注入封顶** —— 替代「从尾部一刀切」。
fn cap_inject_weighted(s: String, total: usize, label: &str) -> String {
    if total == 0 || s.chars().count() <= total {
        return s;
    }
    // 分段：以「【」开头的行起新段
    let mut sections: Vec<(String, String)> = Vec::new();
    let mut title = String::new();
    let mut body = String::new();
    for line in s.split_inclusive('\n') {
        if line.trim_start().starts_with('【') {
            if !title.is_empty() || !body.trim().is_empty() {
                sections.push((title.clone(), body.clone()));
            }
            title = line.trim().to_string();
            body.clear();
        } else {
            body.push_str(line);
        }
    }
    if !title.is_empty() || !body.trim().is_empty() {
        sections.push((title, body));
    }
    if sections.len() <= 1 {
        // 单段（或没分段）：退回原有的一刀切，行为不变
        return cap_inject(s, total, label);
    }
    let wsum: usize = sections.iter().map(|(t, _)| inject_weight(t)).sum::<usize>().max(1);
    let mut out = String::new();
    let mut cut = 0usize;
    for (t, b) in &sections {
        let full = format!("{t}{b}");
        let n = full.chars().count();
        let quota = total * inject_weight(t) / wsum;
        if n <= quota {
            out.push_str(&full);
        } else {
            out.extend(full.chars().take(quota));
            out.push_str(&format!(
                "\n…（本段 {n} 字超额度 {quota}，余下 {} 字已略——需要时 read 记忆目录下对应文件）\n",
                n - quota
            ));
            cut += 1;
        }
    }
    tracing::info!(label, sections = sections.len(), cut, total, "注入物按段分额度");
    out
}

pub async fn run_agent(
    ctx: &AppState,
    session_id: &str,
    user_input: &str,
    image_blocks: Vec<Value>,
    run: crate::state::RunHandle,
) -> AppResult<AgentOutcome> {
    let run_id = run.run_id.clone();
    let user_input = crate::routes::chat::sanitize_user_input(user_input).trim().to_string();

    // ── 事件封顶：会话事件超水位删最旧（REAL_EVENTS_CAP，默认 15 万）──
    match std::env::var("REAL_EVENTS_CAP")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(150_000)
    {
        cap if cap > 0 => {
            match crate::db::repos::trim_events_to_cap(&ctx.pool, session_id, cap).await {
                Ok(n) if n > 0 => {
                    tracing::info!(session = session_id, trimmed = n, cap, "events 超水位已裁剪");
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(session = session_id, error = %e, "events 裁剪失败（不影响主流程）"),
            }
        }
        _ => {}
    }

    // ── 技能调用："/技能名 参数" → SKILL.md 全文挂载为本轮执行框架 ──
    let user_input = if let Some(rest) = user_input.strip_prefix('/') {
        let rest = rest.trim_start();
        let (nm, args) = match rest.find(char::is_whitespace) {
            Some(i) => (&rest[..i], rest[i..].trim()),
            None => (rest, ""),
        };
        match crate::routes::skills::load_skill_text(nm) {
            Some(skill) => {
                if args.is_empty() {
                    format!("{skill}\n\n【本轮任务】按以上技能工作流执行。")
                } else {
                    format!("{skill}\n\n【本轮任务】按以上技能工作流执行，任务参数：{args}")
                }
            }
            None => user_input,
        }
    } else {
        user_input
    };

    // 记忆写入三连（主题/偏好/凭证）——写入失败不影响主流程
    let _ = crate::agent::memory::theme::store_theme(&ctx.pool, session_id, &user_input, None).await;
    let _ = crate::agent::memory::theme::store_preferences(&ctx.pool, session_id, &user_input).await;
    let _ = crate::agent::memory::theme::store_credentials(&ctx.pool, session_id, &user_input).await;

    // run 的身份与取消令牌由路由层创建（routes/chat.rs open_run）——
    let cancel = run.cancel.clone();
    let _guard = crate::state::CancelGuard::new(ctx, session_id, &run_id);

    let (model, effort) = {
        let model = ctx.effective_model(session_id).await;
        let s = ctx.settings.read().await;
        // 档位决策的**唯一实现** = `config::settings::resolve_effort`（含按任务复杂度自适应）。
        let effort = crate::config::settings::resolve_effort(
            &s.thinking_mode,
            &s.thinking_effort,
            &user_input,
        );
        (model, effort)
    };
    let cost = Arc::new(std::sync::Mutex::new(crate::cost::CostTracker::with_model(&model)));
    let billing_model = model.clone();

    crate::db::repos::update_session_status(&ctx.pool, session_id, "planning").await?;

    // 历史（多轮会话）与记忆上下文（插槽附件：没有也能跑）
    let history_build = crate::agent::history::build_history(&ctx.pool, session_id).await?;
    let (history, history_last_user_id) = (history_build.items, history_build.last_user_id);
    let ws_before = crate::db::repos::get_session_workspace(&ctx.pool, session_id)
        .await
        .ok()
        .flatten();
    let memory_ctx = crate::agent::memory::build_memory_prompt(&ctx.pool, session_id, &user_input)
        .await?
        .unwrap_or_default();
    // 轮任务关键词激活：命中任务要点自动回灌（后端行为）
    let turn_hints = crate::agent::memory::turn_log::activate(&ctx.pool, session_id, &user_input)
        .await
        .unwrap_or_default();
    let target_anchor = {
        let ws = crate::db::repos::get_session_workspace(&ctx.pool, session_id)
            .await
            .ok()
            .flatten();
        // 口语项目名命中名录（"查 Real 的问题"这类不带盘符的说法，路径抽取抽不到）
        let named = crate::agent::memory::project_registry::lookup(&ctx.pool, &user_input).await;
        let named_ref = named.as_ref().map(|(p, a)| (p.as_str(), a.as_str()));
        crate::agent::memory::project_profile::target_anchor_full(
            &user_input,
            crate::agent::memory::project_profile::AnchorInput {
                workspace: ws.as_deref(),
                named: named_ref,
                prev: ws_before.as_deref(),
            },
        )
        .unwrap_or_default()
    };
    let path_facts = {
        let args_list = crate::db::repos::recent_tool_args(&ctx.pool, session_id, 12)
            .await
            .unwrap_or_default();
        let mut paths: Vec<String> = Vec::new();
        for a in args_list {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&a) {
                paths.extend(crate::path::structured_paths_from_args(&v));
            }
        }
        crate::path::live_paths_block(&paths, 8)
    };
    let struct_tree = {
        let known = crate::agent::workspace::known_files(session_id);
        crate::agent::memory::project_profile::tree_view(&known).unwrap_or_default()
    };
    let mut memory_ctx = if turn_hints.is_empty() {
        cap_inject_weighted(memory_ctx, env_cap("REAL_MEMORY_CTX_CAP", crate::agent::history::th::INJECT_MEMORY_CHARS), "记忆上下文")
    } else if memory_ctx.is_empty() {
        cap_inject(turn_hints, env_cap("REAL_TURN_HINTS_CAP", crate::agent::history::th::INJECT_TURN_HINTS_CHARS), "轮任务回灌")
    } else {
        let m = cap_inject_weighted(memory_ctx, env_cap("REAL_MEMORY_CTX_CAP", crate::agent::history::th::INJECT_MEMORY_CHARS), "记忆上下文");
        let t = cap_inject(turn_hints, env_cap("REAL_TURN_HINTS_CAP", crate::agent::history::th::INJECT_TURN_HINTS_CHARS), "轮任务回灌");
        format!("{m}\n\n{t}")
    };
    // 技能薄索引（0028）：只注入 name+description（目录扫描，薄）；用户以 /技能名 调用全文
    if let Ok(rd) = std::fs::read_dir(crate::routes::skills::skills_root()) {
        let mut idx = String::from("【可用技能】（输入 /技能名 参数 调用）：");
        let mut n = 0;
        for e in rd.flatten() {
            if let Ok(raw) = std::fs::read_to_string(e.path().join("SKILL.md")) {
                // 技能索引只取 name+description（分类是给人看列表用的，不占 prompt 预算）
                let (name, description, _category, _) = crate::routes::skills::parse_skill_md(&raw);
                if !name.is_empty() {
                    idx.push_str(&format!("\n- /{name}：{description}"));
                    n += 1;
                }
            }
        }
        if n > 0 {
            let idx = cap_inject(idx, env_cap("REAL_SKILL_IDX_CAP", crate::agent::history::th::INJECT_SKILL_IDX_CHARS), "技能索引");
            memory_ctx = if memory_ctx.is_empty() { idx } else { format!("{memory_ctx}\n\n{idx}") };
        }
    }
    if !path_facts.is_empty() {
        memory_ctx = if memory_ctx.is_empty() {
            path_facts
        } else {
            format!("{path_facts}\n{memory_ctx}")
        };
    }
    if !target_anchor.is_empty() {
        memory_ctx = if memory_ctx.is_empty() {
            target_anchor
        } else {
            format!("{target_anchor}\n\n{memory_ctx}")
        };
    }
    // 记忆作用域：`shared`（按工作区跨会话共享）| `session`（只在本任务可见）。
    let memory_shared = crate::db::repos::memory_scope_shared(&ctx.pool).await;
    if memory_shared {
        let dir = crate::path::data_root::scripts_dir().display().to_string();
        let idx = crate::path::data_root::scripts_index(10);
        // 索引**直接列出**（不让模型花一次调用去查——那正是"纪律写了却不生效"的形态）。
        let hint = if idx.is_empty() {
            format!(
                "【常用脚本库】{dir}（暂无现成脚本）\n要写脚本时，把**通用**的存进这个目录\
（首行 docstring 写「名字 · 用途」），后续所有会话/项目共享。"
            )
        } else {
            format!(
                "【常用脚本库】{dir}\n现有：{idx}\n用法 `python {dir}/<名>.py --help`；\
先用现成的、没有再写——写完把**通用**脚本存回该目录（首行 docstring 写「名字 · 用途」）。"
            )
        };
        memory_ctx = if memory_ctx.is_empty() { hint } else { format!("{memory_ctx}\n\n{hint}") };
    }
    // 结构树并入（任务启动即见真实目录树，杜绝凭记忆猜层级）
    if !struct_tree.is_empty() {
        memory_ctx = if memory_ctx.is_empty() {
            struct_tree
        } else {
            format!("{struct_tree}\n\n{memory_ctx}")
        };
    }
    // 长期知识现状（`longterm/` 三类）—— 同【常用脚本库】口径：内容直接列出，
    if memory_shared {
        if let Ok(Some(ws)) = crate::db::repos::get_session_workspace(&ctx.pool, session_id).await {
            let lt = crate::agent::memory::journal::longterm_block(
                &ws,
                crate::agent::memory::journal::LONGTERM_INJECT_CHARS,
            );
            if !lt.is_empty() {
                memory_ctx = if memory_ctx.is_empty() {
                    lt
                } else {
                    format!("{lt}\n\n{memory_ctx}")
                };
            }
        }
    } else {
        // 隔离模式（`memory_scope=session`）：longterm 文件仍在磁盘上（它按**项目**存，
        let off = "【长期记忆】本任务**已停用**（隔离模式）：不要读写 `longterm/` 下的文件，\
                   也不要引用其它任务的结论 —— 本次从零开始。";
        memory_ctx = if memory_ctx.is_empty() {
            off.to_string()
        } else {
            format!("{off}\n\n{memory_ctx}")
        };
    }

    // 流式转发器：模型增量 mpsc → 按序 emit；排空后才发 EV_COMPLETE。
    let (stream_tx, mut stream_rx) =
        tokio::sync::mpsc::unbounded_channel::<(&'static str, serde_json::Value)>();
    let fwd_ctx = ctx.clone();
    let fwd_sid = session_id.to_string();
    let fwd_run = run_id.clone();
    let forwarder = tokio::spawn(async move {
        while let Some((kind, payload)) = stream_rx.recv().await {
            let _ = crate::sse::emit(&fwd_ctx, &fwd_sid, kind, crate::sse::with_run(payload, &fwd_run))
                .await;
        }
    });

    let loop_ = Loop {
        ctx,
        session_id: session_id.to_string(),
        run_id: run_id.clone(),
        model,
        effort,
        image_blocks,
        tools: ctx.registry.tools(),
        history,
        history_last_user_id,
        memory_ctx,
        goal_line: user_input.split("\n\n").next().unwrap_or(&user_input).trim().to_string(),
        cancel: cancel.clone(),
        cost: cost.clone(),
        reasoning_emitter: crate::sse::gate::StreamEmitter::new(crate::sse::EV_REASONING, stream_tx.clone()),
        message_emitter: crate::sse::gate::StreamEmitter::new(crate::sse::EV_MESSAGE, stream_tx.clone()),
        tx: stream_tx,
    };
    // 流式发射器自己做时间窗合并（见 sse/gate.rs），需要定时器把"停顿期的滞留缓冲"冲出去。
    loop_.reasoning_emitter.arm_ticker();
    loop_.message_emitter.arm_ticker();

    let outcome = loop_.spin().await;
    // ★ 先停两路定时冲刷表，再 drop 循环体 —— 顺序是硬性的。
    loop_.reasoning_emitter.shutdown().await;
    loop_.message_emitter.shutdown().await;
    drop(loop_);
    let _ = forwarder.await;

    match outcome {
        Ok((answer, _rounds, tool_calls, changed)) => {
            let (llm_calls, cost_yuan, ratio, peak_tag) = {
                let c = cost.lock().unwrap_or_else(|e| e.into_inner());
                // 计价标签：一口价=flat；峰谷按倍率标 peak/offpeak。
                let tag = if crate::pricing::is_flat_pricing(&billing_model) {
                    "flat".to_string()
                } else {
                    let m = crate::pricing::current_multiplier();
                    if m > 1.0 {
                        format!("peak:{m:.0}")
                    } else {
                        "offpeak".to_string()
                    }
                };
                (c.call_count, crate::cost::CostTracker::round_to_fen(c.total_yuan()), c.cache_hit_ratio(), tag)
            };
            let _ = crate::sse::emit(ctx, session_id, crate::sse::EV_COMPLETE, crate::sse::with_run(json!({
                "model": billing_model,
                "answer": answer,
                "tool_count": tool_calls,
                "llm_calls": llm_calls,
                "cost_yuan": cost_yuan,
                "peak_tag": peak_tag,
                "cache_hit_ratio": ratio,
                "changed_files": changed,
                "plan_steps": [],
            }), &run_id)).await;
            let _ = crate::agent::memory::store_conversation_roll(&ctx.pool, session_id, &user_input, &answer).await;
            // 轮任务日志：结论要点 + 激活关键词入 turn_logs，
            let _ = crate::agent::memory::turn_log::record_turn(&ctx.pool, session_id, &user_input, &answer, &changed).await;
            // 真动过文件才叫任务：闲聊与纯问答不占用任务名与板块标签
            if !changed.is_empty() {
                crate::agent::naming::apply(&ctx.pool, session_id, &user_input).await;
                // 板块标签：从本轮改过的路径推断「这件事属于哪一块」（聊天面板/Agent 内核/话术…），
                if let Some(area) = crate::agent::naming::area_from_paths(&changed) {
                    let _ = crate::db::repos::set_session_area(&ctx.pool, session_id, &area).await;
                }
            }
            // 自动沉淀（只读版）：后台跑复盘，候选经验推给前端看，不落盘、不阻塞本轮结束。
            {
                let st = ctx.clone();
                let sid = session_id.to_string();
                let ui = user_input.to_string();
                let ans = answer.clone();
                // 用 billing_model（= 本轮模型的克隆，见 153 行）——`model` 自身已 move 进 Runner
                let mdl = billing_model.clone();
                let aux_model = {
                    let s = ctx.settings.read().await;
                    crate::config::settings::cheap_model(&s, &mdl)
                };
                tokio::spawn(async move {
                    crate::agent::evolution::maybe_suggest(&st, &sid, &ui, &ans, tool_calls as usize, &aux_model).await;
                });
            }
            {
                let st = ctx.clone();
                let sid = session_id.to_string();
                let ui = user_input.to_string();
                let ans = answer.clone();
                let mdl = billing_model.clone();
                let aux_model = {
                    let s = ctx.settings.read().await;
                    crate::config::settings::cheap_model(&s, &mdl)
                };
                tokio::spawn(async move {
                    crate::agent::session_title::maybe_update_title(&st, &sid, &ui, &ans, &aux_model).await;
                });
            }
            Ok(AgentOutcome { answer })
        }
        Err(e) => {
            // 取消类错误保持静默（终态由路由层 EV_CANCELLED 独占），只上抛防双卡。
            if !cancel.is_cancelled() {
                let _ = crate::sse::emit(ctx, session_id, crate::sse::EV_ERROR, crate::sse::with_run(json!({
                    "message": e.to_string(),
                }), &run_id)).await;
            }
            Err(e)
        }
    }
}

// 循环本体（三相自旋）

/// 传输类错误判定：SSE 读取/流式网络错误可重试；
fn is_transport_error(e: &AppError) -> bool {
    let s = e.to_string();
    // "流异常早停"判为可重试：偶发抖动自愈，持续早停（重试 3 次）才上抛
    s.contains("SSE 读取失败") || s.contains("流式网络请求失败") || s.contains("SSE 解析失败")
        || s.contains("流异常早停")
        || is_upstream_unavailable(e)
}

/// 上游不可用（HTTP 5xx / 408 / 网络层失败）——与"模型答错了"不是一回事
fn is_upstream_unavailable(e: &AppError) -> bool {
    let s = e.to_string();
    if s.contains("LLM 流式 HTTP 5") || s.contains("LLM 流式 HTTP 408") || s.contains("上游繁忙") {
        return true;
    }
    matches!(e, AppError::Http(_)) || s.contains("流式网络请求失败")
}
/// 事实账本：循环全程只记录，不判定
struct Ledger {
    goal: String,
    items: Vec<InputItem>,
    rounds: u32,
    changed_files: Vec<String>,
    pub tool_trace: Vec<(String, bool)>,
    #[allow(dead_code)]
    plan_steps: Vec<Value>,
    bad_rounds: u32,
    /// 空转熔断计数：连续结构性空响应（1-token 死循环特征）≥6 即熔断；与 bad_rounds 语义不同。
    empty_spins: u32,
    last_mirror: u32,
    pending_tail: Option<String>,
    last_edit_preflight: u32,
    finished: bool,
    /// 行动事实：最近一次 write/modify 成功的轮次（0=从未）
    last_edit_round: u32,
    write_mark: std::time::SystemTime,
    /// 任务起点（供自动验证取"本任务写过的文件"）
    task_start: std::time::SystemTime,
    /// 上次自动验证的水位（工作树口径）
    last_verify_mark: std::time::SystemTime,
    /// 工作树扫描不可用（目录过大）——本任务内不再重试，避免每轮白付扫描成本
    wt_unavailable: bool,
    /// 编辑账：(edit_id, file, sig, round)——同签名 ≥2 次 = 重复提交；同文件不同位置是正常节奏。
    edit_ledger: Vec<(String, String, u64, u32)>,
    edit_seq: u32,
    /// 阅读账：文件 → (首读轮次, 读取次数)——重复 read 从不可见变可见
    read_memo: std::collections::HashMap<String, (u32, u32)>,
    stall_count: u32,
    /// 交账宽限计数：止损触底后，先注「交账指令」再收口的缓冲轮数（有新的实体进展即清零）。
    stall_grace: u32,
    /// 分相计时累计（毫秒）：[0]=决策相（含 LLM 流式），[1]=执行+治理+镜像相。
    phase_ms: [u64; 2],
    /// 最近一次探索进展轮次（0=从未）——本轮是否探索进展用它 == rounds 判定
    last_explore_round: u32,
    /// 本任务读过的文件集合（判「首读新文件」：不在集合里的 read 才算探索进展）
    read_seen: std::collections::HashSet<String>,
    /// 上一次 run 命令参数签名（同签名重跑不算探索进展）
    last_run_sig: u64,
    /// 只读收窄态：连续零进展达收窄档后进入——禁 write/modify/run，逼收束总结或向用户提问。
    narrowed: bool,
    /// 路径证据账：已实证存在的目录/文件（防路径漂移）。
    path_map: Vec<String>,
    /// 失败账：文件 → (失败次数, 最近错误类型)——同文件连败对模型可见。
    same_file_fails: std::collections::HashMap<String, (u32, String)>,
    /// 复现账：模型写的复现脚本路径——后端自动重跑它的依据
    repro_script: Option<String>,
    /// 复现最近状态：Some((轮次, 是否仍崩溃))——None = 从未运行
    repro_last: Option<(u32, bool)>,
    /// 复现脚本的工作目录（模型首次运行时记录，自动复现沿用）
    repro_cwd: Option<String>,
    /// 任务内轮间治理：上次探针的 (轮次, 历史字节) —— 供弹性水位算"真实每轮增量"。
    last_midgov: Option<(u32, usize)>,
    /// 输出语言（全局设置）：本 run 内冻结，供帧声明与语言锚共用。
    lang: crate::agent::output_lang::OutputLang,
}

struct Loop<'a> {
    ctx: &'a AppState,
    session_id: String,
    run_id: String,
    model: String,
    effort: String,
    image_blocks: Vec<Value>,
    tools: Vec<crate::model::types::ToolDef>,
    history: Vec<InputItem>,
    /// 当轮用户轮的库内行 id（`build_history` 交出来）—— 当轮帧写回用，见其文档。
    history_last_user_id: Option<String>,
    memory_ctx: String,
    goal_line: String,
    cancel: tokio_util::sync::CancellationToken,
    cost: Arc<std::sync::Mutex<crate::cost::CostTracker>>,
    reasoning_emitter: crate::sse::gate::StreamEmitter,
    message_emitter: crate::sse::gate::StreamEmitter,
    tx: tokio::sync::mpsc::UnboundedSender<(&'static str, serde_json::Value)>,
}

/// 运行产物：(答案, 轮次, 工具调用数, 改动文件)
type Spin = AppResult<(String, u32, u32, Vec<String>)>;

/// 上下文构成快照：请求各段按字节拆分（相对拆分够用，绝对值以 llm.usage 为准）。
fn context_snapshot(
    instructions_bytes: usize,
    tools: &[crate::model::types::ToolDef],
    items: &[InputItem],
) -> Value {
    fn bytes<T: serde::Serialize>(v: &T) -> usize {
        serde_json::to_string(v).map(|j| j.len()).unwrap_or(0)
    }
    let (mut msgs, mut tool_out, mut reasoning, mut calls) = (0usize, 0usize, 0usize, 0usize);
    for it in items {
        match it {
            InputItem::Message { .. } => msgs += bytes(it),
            InputItem::Raw(_) | InputItem::FunctionCallOutput { .. } => tool_out += bytes(it),
            InputItem::Reasoning { .. } => reasoning += bytes(it),
            InputItem::FunctionCall { .. } => calls += bytes(it),
        }
    }
    json!({
        "instructions_bytes": instructions_bytes,
        "tools_bytes": bytes(&tools),
        "messages_bytes": msgs,
        "tool_output_bytes": tool_out,
        "reasoning_bytes": reasoning,
        "call_def_bytes": calls,
        "history_bytes": msgs + tool_out + reasoning + calls,
        "items": items.len(),
    })
}

impl Loop<'_> {
    /// 本 run 的事件出口（唯一）：所有事件自动带上 run 身份。
    async fn emit(&self, kind: &str, payload: Value) {
        let _ = crate::sse::emit(
            self.ctx,
            &self.session_id,
            kind,
            crate::sse::with_run(payload, &self.run_id),
        )
        .await;
    }

    async fn spin(&self) -> Spin {
        let max_rounds = crate::config::settings::tuning()
            .max_rounds
            .load(std::sync::atomic::Ordering::Relaxed);
        // 输出语言（全局设置）：本 run 读一次，供 #frame 声明与语言锚共用。
        let lang = crate::agent::output_lang::load(&self.ctx.pool).await;
        // 任务内轮间治理间隔：**默认关闭（0）**，0=关。
        let midgov_interval: u32 = std::env::var("REAL_MIDGOVERN_INTERVAL")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let mut st = Ledger {
            goal: self.goal_line.clone(),
            items: Vec::new(),
            rounds: 0,
            changed_files: Vec::new(),
            tool_trace: Vec::new(),
            plan_steps: Vec::new(),
            bad_rounds: 0,
            empty_spins: 0,
            last_mirror: 0,
            pending_tail: None,
            last_edit_preflight: 0,
            finished: false,
            last_edit_round: 0,
            write_mark: std::time::SystemTime::now(),
            task_start: std::time::SystemTime::now(),
            last_verify_mark: std::time::SystemTime::now(),
            wt_unavailable: false,
            edit_ledger: Vec::new(),
            edit_seq: 0,
            read_memo: std::collections::HashMap::new(),
            stall_count: 0,
            stall_grace: 0,
            phase_ms: [0, 0],
            last_explore_round: 0,
            read_seen: std::collections::HashSet::new(),
            last_run_sig: 0,
            narrowed: false,
            path_map: Vec::new(),
            same_file_fails: std::collections::HashMap::new(),
            repro_script: None,
            repro_last: None,
            repro_cwd: None,
            last_midgov: None,
            lang,
        };
        // 前缀起点：历史 + 目标 + 记忆上下文（append-only，只追加）
        st.items.extend(self.history.iter().cloned());
        // goal 组装：静态模板在外、动态值后置（前缀缓存友好）；话术全部来自 prompts 资产
        let memory = if self.memory_ctx.trim().is_empty() {
            String::new()
        } else {
            crate::agent::context::render(
                &crate::agent::context::section(
                    include_str!("../../../prompts/workflow/round_frame.md"),
                    "memory",
                ),
                &[("memory", &self.memory_ctx)],
            )
        };
        let goal = crate::agent::context::render(
            &crate::agent::context::section(
                include_str!("../../../prompts/workflow/round_frame.md"),
                "frame",
            ),
            &[
                ("goal", &st.goal),
                ("memory", &memory),
                // 输出语言：全局设置，每 run 读一次（值只进 dynamic 区，见 output_lang.rs）
                ("lang", lang.frame_line()),
                // 域装配：基础契约 + 命中域的工作流话术（内核零域知识）
                ("contract", &contract_for(&st.goal)),
            ],
        );
        let mut user_blocks = vec![json!({"type": "input_text", "text": goal})];
        user_blocks.extend(self.image_blocks.iter().cloned());
        // ── 当轮帧就位：**替换**末条 user，而不是再 push 一条 ──
        let user_content = json!(user_blocks);
        let reused_row = match st.items.last_mut() {
            Some(InputItem::Message { role, content, tool_calls }) if role == "user" => {
                *content = user_content.clone();
                *tool_calls = None;
                true
            }
            _ => false,
        };
        if !reused_row {
            st.items.push(InputItem::Message {
                role: "user".into(),
                content: user_content.clone(),
                tool_calls: None,
            });
        }
        // 写回库：**只换 `content`**（保留 `attachments` 等旁路键）。
        if reused_row {
            if let Some(id) = self.history_last_user_id.as_deref() {
                if let Err(e) = crate::db::repos::update_message_item_content(
                    &self.ctx.pool,
                    id,
                    &user_content.to_string(),
                )
                .await
                {
                    tracing::warn!(
                        session = %self.session_id,
                        error = %e,
                        "当轮帧写回失败：本轮正常，但下一个 run 会在该位置断前缀缓存"
                    );
                }
            }
        }

        // 主循环头硬检查取消：取消即退出，杜绝残流交错。
        while !st.finished && st.rounds < max_rounds && !self.cancel.is_cancelled() {
            st.rounds += 1;

            // ── 分相计时起点（观测埋点）──
            let t_phase_round = std::time::Instant::now();

            // ── 插话注入：用户运行中发言作为真实用户消息注入对话流 ──
            match crate::db::repos::drain_interjections(&self.ctx.pool, &self.session_id, st.rounds as i64)
                .await
            {
                Ok(inj) if !inj.is_empty() => {
                    for it in &inj {
                        st.items.push(InputItem::Message {
                            role: "user".into(),
                            content: json!(format!(
                                "【任务中途插话·非新任务】以下是用户在任务运行中的补充说明：继续按当前任务收束，不要把它当作新的任务目标：\n{}",
                                it.text
                            )),
                            tool_calls: None,
                        });
                        // 插话以事件身份进流：前端在注入点渲染内联用户气泡
                        self.emit(
                            crate::sse::EV_USER_INTERJECTION,
                            json!({ "text": it.text, "round": st.rounds }),
                        )
                        .await;
                        if let Ok(pending) = crate::db::repos::consumed_interjections_without_message(
                            &self.ctx.pool,
                            &self.session_id,
                        )
                        .await
                        {
                            for (iid, text) in pending {
                                let item = json!({
                                    "type": "message",
                                    "role": "user",
                                    "interjection": true,
                                    "content": [{"type": "input_text", "text": text}],
                                });
                                if let Ok(m) = crate::db::repos::insert_message(
                                    &self.ctx.pool,
                                    &self.session_id,
                                    "user",
                                    &text,
                                    Some(&serde_json::to_string(&item).unwrap()),
                                )
                                .await
                                {
                                    let _ = crate::db::repos::attach_interjection_message(
                                        &self.ctx.pool,
                                        iid,
                                        &m.id,
                                    )
                                    .await;
                                }
                            }
                        }
                    }
                    self.emit(
                        crate::sse::EV_MESSAGE,
                        json!({"text": format!("（已看到你的 {} 条插话，正在并入当前任务）", inj.len())}),
                    )
                    .await;
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(session = self.session_id, error = %e, "插话注入失败（不影响本轮）"),
            }

            // 状态镜像（护栏家族·成功空转）：只反射事实，不指路不判定
            self.mirror_if_stuck(&mut st, max_rounds).await;

            // ── 决策相：上下文进 → 模型表态（≤1 LLM）──
            let resp = {
                // take() 而非 clone()：尾部提示是一次性的，留在原位会让它此后每轮重复注入。
                let tail = st.pending_tail.take();
                if let Some(t) = &tail {
                    st.items.push(InputItem::user_message(t));
                }
                let r = self.decide(&st.items).await;
                if tail.is_some() {
                    st.items.pop();
                }
                match r {
                    Ok(v) => v,
                    Err(e) if is_upstream_unavailable(&e) => {
                        tracing::warn!(session = %self.session_id, error = %e,
                                       round = st.rounds, "上游连续不可用：重试耗尽，优雅收口（不判死任务）");
                        return self.terminate(&mut st, &format!(
                            "上游连续不可用（已自动重试多次仍未成功）——\
这不是模型或任务的问题，已把当前进度交付。等上游恢复后**直接回复『继续』**即可带断点续跑。\
（原始错误：{e}）")).await;
                    }
                    Err(e) => return Err(e),
                }
            };

            // ── 决策相结束时点（LLM 流式 + 收敛判定之前）──
            let t_phase_decide = std::time::Instant::now();
            let dropped_imgs = Self::drop_ephemeral_attachments(&mut st);
            if dropped_imgs > 0 {
                tracing::debug!(
                    session = %self.session_id,
                    round = st.rounds,
                    dropped = dropped_imgs,
                    "摘掉上一轮回灌的图片附件（只活一次请求）"
                );
            }

            // ── 收敛相：纯文本 = 答案（模型自决）；空响应 = 坏轮 ──
            if let Some(outcome) = self.maybe_converge(&mut st, &resp).await? {
                return Ok(outcome);
            }

            // ── 执行相：0 LLM（协议归一 → 官方契约回灌 → 逐个执行）──
            let round_ok = self.execute_round(&mut st, resp).await?;

            // ── 自动复现：源码修改落地的轮次，后端自动
            if let Some(script) = st.repro_script.clone() {
                if st.last_edit_round == st.rounds {
                    let call_id = format!("repro_auto_{}", st.rounds);
                    let fc = FunctionCall {
                        call_id,
                        name: "run".into(),
                        arguments: json!({
                            "command": format!("python {}", script),
                            "cwd": st.repro_cwd.clone().unwrap_or_else(|| ".".into()),
                        })
                        .to_string(),
                    };
                    let _ = self.execute_one(&mut st, &fc).await;
                }
            }

            let wt_written: Vec<String> = if st.wt_unavailable {
                Vec::new()
            } else {
                let root = self.workspace_root().await;
                match crate::wt::written_since(&root, st.last_verify_mark) {
                    Some(v) => v,
                    None => {
                        st.wt_unavailable = true;
                        Vec::new()
                    }
                }
            };
            let verify_due = st.last_edit_round == st.rounds || !wt_written.is_empty();
            if verify_due && st.repro_script.is_none() && verify_auto_enabled() {
                let mut verify_input: Vec<String> = st.changed_files.clone();
                verify_input.extend(wt_written.iter().cloned());
                if let Some(plan) = crate::tools::verify::plan_for(&verify_input) {
                    st.last_verify_mark = std::time::SystemTime::now();
                    let call_id = format!("verify_auto_{}", st.rounds);
                    let fc = FunctionCall {
                        call_id,
                        name: "run".into(),
                        arguments: json!({
                            "command": plan.command.clone(),
                            "cwd": if plan.root.is_empty() { st.repro_cwd.clone().unwrap_or_else(|| ".".into()) } else { plan.root.clone() },
                            "reason": format!("编辑后自动验证（{}·后端不变量）", plan.kind),
                            "timeout": 180,
                        })
                        .to_string(),
                    };
                    let _ = self.execute_one(&mut st, &fc).await;
                } else if let Some(hint) =
                    crate::tools::verify::declared_reject_hint(&verify_input)
                {
                    st.pending_tail = Some(match st.pending_tail.take() {
                        Some(prev) => format!("{prev}\n{hint}"),
                        None => hint,
                    });
                }
            }

            // 坏轮记账：成功即清零；仅镜像数据，终止只剩轮次上限。
            self.note_bad_round(&mut st, round_ok);

            // ── 任务内轮间治理：每 N 轮**探一次**（govern_in_task 自带**水位**闸门 ——
            if midgov_interval > 0 && st.rounds % midgov_interval == 0 {
                // 先取当前水位再调闸门：`prev` 交的是**上次探针**的 (轮次, 字节)，
                let now_bytes = crate::agent::history::intake_bytes_of(&st.items);
                // 记账尺子必须与 `record_rewrite` 内部一致（`items_bytes`），
                let before_bytes = crate::agent::history::items_bytes(&st.items);
                let saved = crate::agent::history::govern_in_task(
                    &mut st.items,
                    st.rounds,
                    max_rounds,
                    st.last_midgov,
                );
                st.last_midgov = Some((st.rounds, now_bytes));
                if saved > 0 {
                    crate::agent::history::record_rewrite(
                        &self.ctx.pool,
                        &self.session_id,
                        "intask",
                        before_bytes,
                        &st.items,
                    )
                    .await;
                    // 合计口径：含**工具输出 / 重复读 / 思考**三段，别把某一段当成全部。
                    tracing::info!(
                        session = %self.session_id, round = st.rounds, saved_bytes = saved,
                        "任务内轮间治理：旧条目已按预算改写（合计：工具输出/重复读/思考）"
                    );
                }
            }

            if st.last_edit_round == st.rounds {
                st.stall_count = 0;
                st.stall_grace = 0;
                st.narrowed = false;
            } else if st.last_explore_round != st.rounds {
                st.stall_count += 1;
            }
            // 唤醒档：到达阈值那一轮经 pending_tail 注入一次（一次性尾部提示，次轮生效即被 take）
            {
                let warn_at = crate::config::settings::tuning()
                    .stall_warn_rounds
                    .load(std::sync::atomic::Ordering::Relaxed);
                if st.stall_count == warn_at {
                    st.pending_tail = Some(format!(
                        "【停滞唤醒】已连续 {} 轮零新进展（复读旧文件/重跑同命令不算）。必须换策略：\
把当前卡点写出来向用户提问，或换一条路径——不许原样重试，也不许把诊断包装成新尝试。",
                        st.stall_count
                    ));
                    tracing::warn!(
                        session = %self.session_id, round = st.rounds, stall_count = st.stall_count,
                        "停滞唤醒：注入换策略提示"
                    );
                }
            }

            if let Some(reason) = stall_reason(&st) {
                // 交账宽限（柔性化）：触底不再「一撞就死」，先给 grace 轮让它自己收尾/交账。
                // 宽限内若产出新的实体进展，上面那段已把 stall_count/stall_grace 清零、自动解除。
                let grace_cap = crate::config::settings::tuning()
                    .stall_grace_rounds
                    .load(std::sync::atomic::Ordering::Relaxed);
                if st.stall_grace < grace_cap {
                    st.stall_grace += 1;
                    st.pending_tail = Some(format!(
                        "【交账收尾】已连续 {} 轮零新进展，触底止损已挂起（宽限 {}/{}）。\
从本轮起**停止试探**，只做一件事：把现有发现整理成可交付的结论——\
已查明的部分、卡在哪里、还差什么信息或决定。不要原样重试，也不要另开战线。",
                        st.stall_count, st.stall_grace, grace_cap
                    ));
                    tracing::warn!(
                        session = %self.session_id, round = st.rounds, stall_count = st.stall_count,
                        grace = st.stall_grace, grace_cap,
                        "无进展止损触底：进入交账宽限（先收尾，不立即停机）"
                    );
                } else {
                    tracing::warn!(
                        session = %self.session_id, round = st.rounds,
                        last_edit_round = st.last_edit_round, files = st.changed_files.len(),
                        "无进展止损：宽限用尽，提前收口（不再干到轮次上限）"
                    );
                    return self.terminate(&mut st, reason).await;
                }
            }

            // ── 分相计时累加（观测埋点）：决策相（含 LLM 流式）/ 执行+治理+镜像相 ──
            st.phase_ms[0] += t_phase_decide.duration_since(t_phase_round).as_millis() as u64;
            st.phase_ms[1] += t_phase_decide.elapsed().as_millis() as u64;
        }

        // 收口分流：用户取消走 Err 由外层 cancelled 分支接管，不误报轮次上限。
        if self.cancel.is_cancelled() {
            return Err(AppError::Internal("编排被用户取消".into()));
        }
        // 轮次上限：非收敛终止 = 未完成（陈述，不判定工作质量）
        self.terminate(&mut st, &format!("达到轮次上限（{max_rounds} 轮）")).await
    }

    /// 决策相原子步：组请求（items + tools 字段）→ 流式调用
    async fn decide(&self, items: &[InputItem]) -> AppResult<StreamResult> {
        let items: Vec<InputItem> = crate::agent::history::reconcile_for_request(items);
        // 缓存探针：逐条指纹落日志（定位"两轮第一个不同的条目"；默认摘要，可关）。
        crate::agent::history::cache_probe(&self.session_id, &items, "round");
        // 前缀探针：盯住「instructions + tools」这 32KB 是否漂移。它不在这条 items 路径上，
        crate::agent::history::prefix_probe(
            &self.session_id,
            (*CONVERGENT_SYSTEM_PROMPT).len(),
            &self.tools,
        );
        // 上下文构成快照（成本度量·一）：每轮请求前拆各段字节并落 ctx.snapshot 事件——
        self.emit(
            crate::sse::EV_CTX_SNAPSHOT,
            context_snapshot((*CONVERGENT_SYSTEM_PROMPT).len(), &self.tools, &items),
        )
        .await;
        let wire = to_wire_items(&items);
        if let Some(why) = crate::model::types::validate_wire_items(&wire) {
            tracing::error!(session = %self.session_id, why = %why, "wire 契约自检不通过（已尝试归一后仍异常）");
            self.emit(
                "wire.violation",
                serde_json::json!({"why": why}),
            )
            .await;
        }
        let req = ResponsesRequest::builder(&self.model)
            .instructions((*CONVERGENT_SYSTEM_PROMPT).to_string())
            .input(wire)
            .tools(self.tools.clone())
            .tool_choice(ToolChoice::Auto)
            .reasoning_effort(&self.effort)
            // DeepSeek 用于代码任务 ⇒ 温度取 0 求确定性。
            .temperature(0.0)
            // 单次输出上限：**现读**调优台账（面板「调优」页保存即作用于下一次请求），
            .max_output_tokens(
                crate::config::settings::tuning()
                    .max_output_tokens
                    .load(std::sync::atomic::Ordering::Relaxed),
            )
            .user(&self.session_id)
            .build();

        let mut on_reasoning = {
            let e = self.reasoning_emitter.clone();
            move |d: &str| e.feed(d)
        };
        let mut on_text = {
            let e = self.message_emitter.clone();
            move |d: &str| e.feed(d)
        };

        // ── 传输断流重试：SSE 中断退避后整调用重来（协议/配额错误不重试；对账本透明）──
        const DECIDE_MAX_TRIES: u32 = 3;
        let mut attempt = 0u32;
        let resp = loop {
            // thinking start：首次与每次重试都开新块（前端半截思考被 end 关掉后新块续思考）
            let _ = self.tx.send((crate::sse::EV_THINKING,
                json!({"status": "start", "label": "深度思考", "summary": ""})));

            let result = tokio::select! {
                r = self.ctx.llm.stream_reasoning_with_text(&req, &mut on_reasoning, &mut on_text) => r,
                _ = self.cancel.cancelled() => {
                    // cancel 同样必须关思考块（end 丢失 → 前端思考块永远展开）
                    self.reasoning_emitter.flush();
                    let _ = self.tx.send((crate::sse::EV_THINKING,
                        json!({"status": "end", "label": "深度思考", "summary": ""})));
                    return Err(AppError::Internal("编排被用户取消".into()));
                }
            };
            match result {
                Ok(resp) => {
                    // flush 尾巴再发 end：end 不得越过尾部增量
                    self.reasoning_emitter.flush();
                    let _ = self.tx.send((crate::sse::EV_THINKING,
                        json!({"status": "end", "label": "深度思考", "summary": ""})));
                    break resp;
                }
                Err(e) if is_transport_error(&e) && attempt < DECIDE_MAX_TRIES => {
                    attempt += 1;
                    // 关掉半截思考块（flush 尾巴 + end），下次重试重开新块
                    self.reasoning_emitter.flush();
                    let _ = self.tx.send((crate::sse::EV_THINKING,
                        json!({"status": "end", "label": "深度思考", "summary": ""})));
                    self.reasoning_emitter.reset();
                    self.message_emitter.reset();
                    let backoff = std::time::Duration::from_millis(1000 * (1 << attempt));
                    crate::sse::emit_ephemeral(
                        self.ctx,
                        &self.session_id,
                        crate::sse::EV_LLM_RETRY,
                        json!({
                            "attempt": attempt,
                            "max": DECIDE_MAX_TRIES,
                            "backoff_ms": backoff.as_millis() as u64,
                            "reason": e.to_string(),
                            "upstream": is_upstream_unavailable(&e),
                        }),
                    );
                    tracing::warn!(session = %self.session_id, attempt, backoff_ms = backoff.as_millis() as u64,
                                   error = %e, "LLM 传输断流，退避后整体重试");
                    tokio::time::sleep(backoff).await;
                }
                Err(e) => {
                    // 错误退出前必须关思考块（end 丢失 → 前端永远展开）
                    self.reasoning_emitter.flush();
                    let _ = self.tx.send((crate::sse::EV_THINKING,
                        json!({"status": "end", "label": "深度思考", "summary": ""})));
                    return Err(e);
                }
            }
        };

        // 成本记账 + usage 事件（finish: stop/length/error；含 reasoning 拆分与单轮金额）
        let (i, o, cached) = crate::cost::usage_breakdown(&resp.usage);
        let finish = match resp.status {
            crate::model::types::StreamStatus::Completed => "stop",
            crate::model::types::StreamStatus::Incomplete => "length",
            crate::model::types::StreamStatus::Failed => "error",
        };
        let reasoning = resp.usage.as_ref().and_then(|u| u.reasoning_tokens);
        let cost_yuan = crate::pricing::cost_of_now(&self.model, i, o, cached);
        // 分项：把“钱花在哪”一并透出（前端费用行显示占比，而非裸 token 数）
        let (cost_miss, cost_cached, cost_output) =
            crate::pricing::cost_split_now(&self.model, i, o, cached);
        self.emit(crate::sse::EV_LLM_USAGE, json!({
            "model": self.model, "input": i, "cached": cached, "output": o,
            "reasoning": reasoning, "finish": finish, "cost_yuan": cost_yuan,
            "cost_miss_yuan": cost_miss, "cost_cached_yuan": cost_cached,
            "cost_output_yuan": cost_output,
        })).await;
        self.cost.lock().unwrap_or_else(|e| e.into_inner()).record(i, o, cached);
        Ok(resp)
    }

    /// 收敛相：纯文本 = 答案（模型自决）。有工具调用 → None（进执行相）。
    fn is_truncated(resp: &StreamResult) -> bool {
        matches!(resp.status, crate::model::types::StreamStatus::Incomplete)
    }

    async fn maybe_converge(&self, st: &mut Ledger, resp: &StreamResult) -> AppResult<Option<(String, u32, u32, Vec<String>)>> {
        if !resp.function_calls.is_empty() {
            st.empty_spins = 0;
            return Ok(None);
        }
        // ★ 截断的文本**不是答案** —— 不收，交回循环重发。
        if Self::is_truncated(resp) {
            st.bad_rounds += 1;
            tracing::warn!(
                text_bytes = resp.output_text.len(),
                bad_rounds = st.bad_rounds,
                "收敛相遇截断：不作为答案交付，交回循环重发（否则用户拿到半截答案且会话已 done）"
            );
            return Ok(None);
        }
        let text = resp.output_text.trim().to_string();
        if text.is_empty() {
            st.bad_rounds += 1;
            if empty_spin_signal(resp) {
                st.empty_spins += 1;
            }
            if st.empty_spins >= 6 {
                let reason = "后端熔断：连续 6 轮模型输出为空且无工具（疑似 LLM 流被上游掐断/上下文异常，1-token 空转）——已终止避免烧轮次。请重试；若复现，查 usage 事件的 finish 字段（length=上游截断）定位。";
                return Ok(Some(self.terminate(st, reason).await?));
            }
            return Ok(None);
        }
        st.finished = true;
        self.message_emitter.flush();
        { let pool = &self.ctx.pool;
            // 最终回答落库（"记忆做丰富"）：收敛文本需入 messages 表——
            let item = InputItem::assistant_message(&text);
            let ij = serde_json::to_string(&item).unwrap_or_default();
            let _ = crate::db::repos::insert_message(pool, &self.session_id, "assistant", &text, Some(&ij)).await;
            let _ = crate::db::repos::update_session_status(
                pool,
                &self.session_id,
                crate::db::repos::SessionStatus::Done.as_str(),
            )
            .await;
        }
        Ok(Some((text, st.rounds, st.tool_trace.len() as u32, st.changed_files.clone())))
    }

    /// 执行相整轮：协议归一 → 官方契约回灌（reasoning/assistant）→ 逐个执行 → output 回灌。
    async fn execute_round(&self, st: &mut Ledger, mut resp: StreamResult) -> AppResult<bool> {
        // 协议归一先行：assistant 承载归一后 tool_calls（防孤儿 output 400）
        let fcs = normalize_calls(st, std::mem::take(&mut resp.function_calls));
        // ★ 本轮是否撞了输出上限 —— 判据统一走 `is_truncated`（执行相与收敛相共用一处定义）。
        let truncated = Self::is_truncated(&resp);
        // 轮边界排空正文发射器：本轮模型说的话（旁白）必须先于本轮工具卡片到达前端。
        self.message_emitter.flush();
        // thinking mode 契约：reasoning 必须回传（**非空**即可，长度不受约束 → 可安全瘦身）
        if !resp.reasoning.trim().is_empty() {
            // 入库瘦身·思考（`intask::slim_reasoning_intake`）：单条超 `INTAKE_REASONING_BUDGET_BYTES` 的思考，
            let slim = crate::agent::history::slim_reasoning_intake(&resp.reasoning);
            st.items.push(InputItem::reasoning_item(&slim));
            // 落库（messages 表）：多轮历史重建 + 前端对话显示依赖它。
            { let pool = &self.ctx.pool;
                let ij = serde_json::to_string(&st.items.last().unwrap()).unwrap_or_default();
                let _ = crate::db::repos::insert_message(pool, &self.session_id, "reasoning", &resp.reasoning, Some(&ij)).await;
            }
        }
        st.items.push(InputItem::assistant_with_tools(&resp.reasoning, "", &fcs));
        { let pool = &self.ctx.pool;
            let ij = serde_json::to_string(&st.items.last().unwrap()).unwrap_or_default();
            let _ = crate::db::repos::insert_message(pool, &self.session_id, "assistant", &resp.output_text, Some(&ij)).await;
        }
        if truncated {
            let note = concat!(
                "【本轮输出被截断 · 未执行】你上一步的响应撞到了输出上限（max_output_tokens），",
                "**这个工具调用的参数不完整，已被放弃执行**。\n",
                "（执行残缺参数可能造成部分写入——比如只写了半个文件——代价高于重来一次。）\n",
                "下一步请把动作**拆小重发**：一次只做一件事、只写一个文件的一段；\n",
                "内容本就很长时，先写骨架再分段补；不要让单轮输出逼近上限。"
            );
            for fc in &fcs {
                tracing::warn!(
                    call_id = %fc.call_id,
                    tool = %fc.name,
                    args_bytes = fc.arguments.len(),
                    "本轮输出被截断：跳过执行残缺 tool_call"
                );
                st.tool_trace.push((fc.name.clone(), false));
                { let pool = &self.ctx.pool;
                    st.items
                        .push(InputItem::function_call_output(&fc.call_id, note));
                    let ij = serde_json::to_string(&st.items.last().unwrap()).unwrap_or_default();
                    let _ = crate::db::repos::insert_message(
                        pool, &self.session_id, "tool", note, Some(&ij),
                    ).await;
                }
            }
            // 截断且**无 tool_call**（纯文本被砍，典型是长答案写一半）
            if fcs.is_empty() {
                let note = concat!(
                    "【上一轮输出被截断，且未产生任何工具调用】你的回复撞到了输出上限。\n",
                    "请**把回复改短、分次给出**：先给结论骨架（几句话），必要时再分段展开；\n",
                    "不要试图在一轮里写完整篇长文。"
                );
                tracing::warn!("本轮输出被截断且无 tool_call，注入提示后重试");
                // 只进当轮上下文，**不落库**：这是「发给模型的纠偏指令」，不是用户说的话。
                st.items.push(InputItem::user_message(note));
            }
            return Ok(false);
        }
        if !st.narrowed {
            let narrow_at = crate::config::settings::tuning()
                .stall_narrow_rounds
                .load(std::sync::atomic::Ordering::Relaxed);
            if st.stall_count >= narrow_at {
                st.narrowed = true;
                tracing::warn!(
                    session = %self.session_id, round = st.rounds, stall_count = st.stall_count,
                    "只读收窄：连续零进展达阈值，禁写禁跑（逼收束总结/提问）"
                );
            }
        }
        let mut ok_any = false;
        for fc in &fcs {
            if st.narrowed && matches!(fc.name.as_str(), "write" | "modify" | "run") {
                let call_id = fc.call_id.clone();
                let tool = fc.name.clone();
                let env = ToolEnvelope::error(
                    &call_id,
                    &tool,
                    "【只读收窄】任务已连续多轮零新进展，写/跑类工具暂时停用。本轮只做两件事之一：\
① 直接输出收束总结（试过什么/为何卡/还差什么信息）；② 用 ask 向用户提问（一问续命）。\
产出新修改且生效后收窄自动解除。",
                    0,
                );
                st.tool_trace.push((fc.name.clone(), false));
                self.emit_tool(&call_id, &tool, &Value::Null, &env, false).await;
                let out = serde_json::to_string(&env).unwrap_or_else(|_| {
                    env.content.iter().filter_map(|c| c.text.clone()).collect::<String>()
                });
                st.items.push(InputItem::function_call_output(&fc.call_id, &out));
                continue;
            }
            let (mut env, ok) = self.execute_one(st, fc).await;
            st.tool_trace.push((fc.name.clone(), ok));
            if ok {
                ok_any = true;
            }
            // 语言锚（线上反馈"推理开头中文、越往后越英文"）：长任务里
            if st.rounds % 5 == 0
                && fc.call_id == fcs.last().map(|f| f.call_id.clone()).unwrap_or_default()
            {
                env.content
                    .push(crate::mcp::envelope::ContentPart::text(st.lang.anchor().to_string()));
            }
            let imgs: Vec<(String, String)> = env
                .content
                .iter()
                // 判据取 `ContentPart::reachable_image_uri`（**单一事实源**）
                .filter_map(|p| {
                    let uri = p.reachable_image_uri()?.to_string();
                    Some((uri, p.mime_type.clone().unwrap_or_else(|| "image/png".into())))
                })
                .collect();
            // **A 方案**：图片只在本轮以附件可见，历史与库里的那份**不留 base64**。
            let img_bytes_stripped = env.strip_inline_image_bodies();
            if img_bytes_stripped > 0 {
                tracing::debug!(
                    session = %self.session_id,
                    tool = %fc.name,
                    stripped = img_bytes_stripped,
                    "内联图片 base64 不随历史留存（本轮已按附件回灌）"
                );
            }
            let out = serde_json::to_string(&env).unwrap_or_else(|_| {
                env.content.iter().filter_map(|c| c.text.clone()).collect::<String>()
            });
            // 工具结果的治理**已统一到工具层**（`mcp::spill::maybe_spill`，工具返回那一刻处理）——
            st.items.push(InputItem::function_call_output(&fc.call_id, &out));
            { let pool = &self.ctx.pool;
                let ij = serde_json::to_string(&st.items.last().unwrap()).unwrap_or_default();
                let _ = crate::db::repos::insert_message(pool, &self.session_id, "tool", &out, Some(&ij)).await;
            }
            if !imgs.is_empty()
                && crate::model::catalog::model_vision(&self.ctx.effective_model(&self.session_id).await)
            {
                let mut blocks = vec![serde_json::json!({
                    "type": "input_text",
                    "text": format!(
                        "{}{} 返回了 {} 张图片——后端已按**附件**回灌（与你从聊天框发图同一条通道）。\
直接看图；**不要用 read 读图片**（read 是文本读取器，只会得到乱码）。\
此附件**只在本轮可见**：下一轮若要再看，read 上面那句说明里的原路径即可重新取回。",
                        crate::model::types::InputItem::IMAGE_ATTACHMENT_MARK,
                        fc.name,
                        imgs.len()
                    )
                })];
                for (uri, _mime) in &imgs {
                    blocks.push(serde_json::json!({
                        "type": "input_image",
                        "image_url": uri,
                        "detail": "high"
                    }));
                }
                st.items.push(InputItem::user_message_blocks(serde_json::json!(blocks)));
            }
        }
        Ok(ok_any)
    }

    fn drop_ephemeral_attachments(st: &mut Ledger) -> usize {
        let before = st.items.len();
        st.items.retain(|it| !it.is_ephemeral_image_attachment());
        before - st.items.len()
    }

    /// 坏轮记账（事实归属：成功发生在执行层，这一刻清零）
    fn note_bad_round(&self, st: &mut Ledger, round_ok: bool) {
        if round_ok {
            st.bad_rounds = 0;
        } else {
            st.bad_rounds += 1;
        }
    }

    /// 执行相原子步：路由 → 契约校验 → 参数净化 → 执行 → 信封
    async fn execute_one(&self, st: &mut Ledger, fc: &FunctionCall) -> (ToolEnvelope, bool) {
        let call_id = fc.call_id.clone();
        // ① 路由（原子名直通；未注册名由契约层报第一手错误）
        let tool = fc.name.clone();
        let args: Value = crate::agent::plan::parse_json_lenient(&fc.arguments).unwrap_or(Value::Null);
        // ② 契约校验（执行前）：schema 校验失败不执行，错误码 + suggestion 回灌
        let schema = self
            .tools
            .iter()
            .find(|t| t.name == tool)
            .map(|t| t.parameters.clone());
        if let Some(schema) = &schema {
            if let Err(e) = crate::tools::contract::validate(schema, &args) {
                let env = ToolEnvelope::error(&call_id, &tool, &crate::tools::contract::err_text(&e), 0);
                self.emit_tool(&call_id, &tool, &args, &env, false).await;
                return (env, false);
            }
        }
        // ③ 参数净化（M2 参数层）
        let mut args = match crate::agent::execution::prepare_args::prepare_tool_args(
            &tool,
            &args,
            &std::collections::HashMap::new(),
            &self.session_id,
            &std::sync::Mutex::new(std::collections::HashMap::new()),
        ) {
            Ok(a) => a,
            Err(e) => {
                let env = ToolEnvelope::error(&call_id, &tool, &e, 0);
                self.emit_tool(&call_id, &tool, &args, &env, false).await;
                return (env, false);
            }
        };
        // ③.5 路径静默纠正（归 path 部门：候选收集/匹配/改写全在 path::silent_correct_args，
        let memo_files: Vec<String> = st.read_memo.keys().cloned().collect();
        let path_corrected = if tool == "write" {
            None
        } else {
            crate::path::silent_correct_args(
                &mut args,
                &memo_files,
                &st.changed_files,
                &st.path_map,
                &crate::agent::plan::extract_paths(&st.goal),
            )
        };
        let cwd_corrected: Option<(String, String)> = if tool == "run" {
            let ws = crate::db::repos::get_session_workspace(&self.ctx.pool, &self.session_id)
                .await
                .ok()
                .flatten();
            let fixed = crate::path::correct_invalid_cwd(&args, ws.as_deref());
            if let Some((_, used)) = &fixed {
                if let Some(m) = args.as_object_mut() {
                    m.insert("cwd".into(), serde_json::json!(used));
                }
            }
            fixed
        } else {
            None
        };
        // ③.75 确认门：删除类/敏感区写入挂起等批准；批准注入一次性 token（模型无法伪造），
        let mut confirm_asked = false;
        if let Some(spec) = crate::confirm::needs_confirm(&self.session_id, &tool, &args) {
            confirm_asked = true;
            let result = crate::confirm::request_confirmation(self.ctx, &self.session_id, &spec).await;
            if result.approved {
                if spec.action == "select_workspace" {
                    // 用户选定了工作区路径（None = 批准但没给路径，按拒绝处理）
                    if let Some(ws) = result.selected {
                        // 写会话工作区（治本：后续命令/上下文都锚定它）
                        let _ = crate::db::repos::set_session_workspace(
                            &self.ctx.pool,
                            &self.session_id,
                            ws.trim(),
                        )
                        .await;
                        // 用选定路径覆盖本条命令的 cwd 重跑
                        if let Some(m) = args.as_object_mut() {
                            m.insert("cwd".into(), serde_json::json!(ws));
                        }
                        tracing::info!(
                            session = %self.session_id,
                            tool = %tool,
                            workspace = %ws,
                            "确认门：用户选定工作区，命令以新 cwd 重跑"
                        );
                    } else {
                        let env = ToolEnvelope::error(
                            &call_id,
                            &tool,
                            "用户未提供工作区路径（确认弹窗），未执行",
                            0,
                        );
                        self.emit_tool(&call_id, &tool, &args, &env, false).await;
                        return (env, false);
                    }
                } else if let Some(token) = result.token {
                    // 批准：token 注入 args（写入/edit 消费端读取的键；run 不读、无副作用）
                    if let Some(m) = args.as_object_mut() {
                        m.insert(
                            "__real_self_edit_token".into(),
                            serde_json::json!(token),
                        );
                    }
                }
            } else {
                let msg = if self.cancel.is_cancelled() {
                    "编排被用户取消"
                } else {
                    "用户拒绝了该操作（确认弹窗），未执行"
                };
                let env = ToolEnvelope::error(&call_id, &tool, msg, 0);
                self.emit_tool(&call_id, &tool, &args, &env, false).await;
                return (env, false);
            }
        }
        // ③.8 模型主动提问（ask）：与②上的护栏门**共用载体、不共用判据**。
        if tool == "ask" {
            let env = match crate::tools::ask::spec_from_args(&args) {
                Err(reason) => ToolEnvelope::error(&call_id, &tool, &reason, 0),
                Ok(spec) => {
                    let r =
                        crate::confirm::request_confirmation(self.ctx, &self.session_id, &spec).await;
                    // 超时/取消 → approved=false：模型必须知道"没人答"，
                    let note = r.note.clone().unwrap_or_default();
                    let mut data = serde_json::json!({
                        "question": spec.target,
                        "chosen": r.selected.clone().unwrap_or_default(),
                        "timed_out": !r.approved,
                    });
                    // 用户自己补的那句话。只在写了的时候出现——空键会让模型去解读
                    if !note.is_empty() {
                        data["note"] = serde_json::Value::String(note);
                    }
                    let out = serde_json::json!({ "kind": "user_choice", "data": data });
                    ToolEnvelope::success(
                        &call_id,
                        &tool,
                        vec![crate::mcp::envelope::ContentPart::text(&out.to_string())],
                        0,
                    )
                }
            };
            self.emit_tool(&call_id, &tool, &args, &env, false).await;
            return (env, false);
        }
        // ④ 执行（超时 + 取消竞速）：信封 = max(默认, 参数 timeout+15s)——工具自身超时先触发，信封只兜死锁。
        let tsecs = tool_envelope_secs(
            std::env::var("REAL_TOOL_TIMEOUT_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(TOOL_TIMEOUT_DEFAULT),
            &tool,
            &args,
        );
        let timeout = std::time::Duration::from_secs(tsecs);
        let call_fut = self.ctx.registry.call(&call_id, &tool, args.clone(), timeout, &self.session_id);
        // running 先上屏，再挂 4s 心跳竞速（长任务 UI 有 elapsed 跳动；select 命中即 drop）
        self.emit_tool_running(&call_id, &tool, &args).await;
        let mut env = tokio::select! {
            r = call_fut => r,
            _ = self.cancel.cancelled() => {
                return (ToolEnvelope::error(&call_id, &tool, "编排被用户取消", 0), false);
            }
            _ = self.progress_heartbeat(&call_id, &tool) => {
                unreachable!("心跳循环只在工具完成/取消时被 drop 终止")
            }
        };
        let mut ok = !env.is_error;
        if !ok && !confirm_asked {
            let text: String = env
                .content
                .iter()
                .filter_map(|c| c.text.clone())
                .collect::<Vec<_>>()
                .join(" ");
            if text.contains("EDIT_REQUIRES_CONFIRM") || text.contains("WRITE_REQUIRES_CONFIRM") {
                let target = args
                    .get("file")
                    .or_else(|| args.get("path"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let spec = crate::confirm::ConfirmSpec::guarded(
                    "sensitive_write",
                    target.clone(),
                    format!("该路径需你确认后才能修改（Real 自身目录/敏感区）: {target}"),
                    "warn",
                );
                let res = crate::confirm::request_confirmation(self.ctx, &self.session_id, &spec).await;
                if res.approved {
                    if let Some(tok) = res.token {
                        if let Some(m) = args.as_object_mut() {
                            m.insert("__real_self_edit_token".into(), serde_json::json!(tok));
                        }
                        tracing::info!(session = %self.session_id, tool = %tool, "补弹窗批准：注入自修改令牌并重跑一次");
                        env = self
                            .ctx
                            .registry
                            .call(&call_id, &tool, args.clone(), timeout, &self.session_id)
                            .await;
                        ok = !env.is_error;
                    }
                } else {
                    env = ToolEnvelope::error(
                        &call_id,
                        &tool,
                        "用户拒绝了该操作（确认弹窗），未执行",
                        0,
                    );
                }
            }
        }
        // 路径纠正事实反馈（后端静默纠正，模型学到但不打断）
        if let Some((cand, orig)) = &path_corrected {
            env.content.push(crate::mcp::envelope::ContentPart::text(format!(
                "〔路径已自动纠正〕你请求的路径 {orig} 不存在，已改用 {cand} 执行（两者仅差一个字符，连字符/下划线拼写）。以后按 {cand} 写。"
            )));
        }
        if let Some((orig, used)) = &cwd_corrected {
            env.content.push(crate::mcp::envelope::ContentPart::text(format!(
                "〔cwd 已自动纠正〕你指定的工作目录 {orig} 不存在，本次已在 {used} 执行。以后请用真实存在的目录（先 list 确认），不要沿用旧路径。"
            )));
        }
        if ok && tool == "run" {
            {
                let mut h = std::collections::hash_map::DefaultHasher::new();
                std::hash::Hash::hash(&fc.arguments, &mut h);
                let sig = std::hash::Hasher::finish(&h);
                if sig != st.last_run_sig {
                    st.last_run_sig = sig;
                    st.last_explore_round = st.rounds;
                }
            }
            if let Some(cwd) = args.get("cwd").and_then(|v| v.as_str()).filter(|s| !s.trim().is_empty()) {
                let cur = crate::db::repos::get_session_workspace(&self.ctx.pool, &self.session_id)
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                let verdict = crate::facts::judge_on_write(&[cwd.to_string()]);
                let usable = verdict
                    .first()
                    .map(|(_, l)| *l != crate::facts::Liveness::Stale)
                    .unwrap_or(false);
                if cur.is_empty() && usable {
                    let _ = crate::db::repos::set_session_workspace(&self.ctx.pool, &self.session_id, cwd).await;
                    tracing::info!(session = %self.session_id, from = %cur, to = %cwd, "会话工作区已按真实 cwd 校准");
                }
            }
        }
        // 事实账本：改过哪些文件（只记录，不判定）
        if ok {
            if let Some(p) = args
                .get("file")
                .or_else(|| args.get("path"))
                .or_else(|| args.get("target"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
            {
                let p = p.to_string();
                if !st.changed_files.contains(&p) {
                    st.changed_files.push(p);
                }
            }
            // 路径证据账：成功触及路径入 path_map（"存在什么"与"改了什么"分账）
            if tool == "list" || tool == "read" || tool == "write" || tool == "modify" {
                // 文件本体：read 的 paths[]；write/modify 的 file/path/target
                let files: Vec<String> = if tool == "read" {
                    args.get("paths").and_then(|v| v.as_array())
                        .map(|a| a.iter().filter_map(|v| v.as_str()).map(str::to_string).collect())
                        .unwrap_or_default()
                } else if tool == "write" || tool == "modify" {
                    args.get("file")
                        .or_else(|| args.get("path"))
                        .or_else(|| args.get("target"))
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.trim().is_empty())
                        .map(|s| vec![s.to_string()])
                        .unwrap_or_default()
                } else {
                    Vec::new()
                };
                let dirs: Vec<String> = if tool == "list" {
                    args.get("path").and_then(|v| v.as_str()).map(|s| vec![s.to_string()]).unwrap_or_default()
                } else {
                    files
                        .iter()
                        .filter_map(|p| std::path::Path::new(p).parent().map(|x| x.to_string_lossy().to_string()))
                        .filter(|x| !x.is_empty())
                        .collect::<Vec<_>>()
                };
                // list 返回条目全量入账（信封 data.entries[].path；上限 800 防递归大目录撑爆内存）
                if tool == "list" {
                    const PATH_MAP_CAP: usize = 800;
                    for part in &env.content {
                        let Some(txt) = part.text.as_deref() else { continue };
                        let Ok(v) = serde_json::from_str::<serde_json::Value>(txt) else { continue };
                        let Some(entries) = v.get("data").and_then(|d| d.get("entries")).and_then(|e| e.as_array()) else { continue };
                        for e in entries {
                            if st.path_map.len() >= PATH_MAP_CAP { break; }
                            if let Some(p) = e.get("path").and_then(|x| x.as_str()) {
                                if !p.is_empty() && !st.path_map.iter().any(|k| k == p) {
                                    st.path_map.push(p.to_string());
                                }
                            }
                        }
                    }
                }
                for p in files.iter().chain(dirs.iter()) {
                    if st.path_map.len() >= 800 { break; }
                    if !p.is_empty() && !st.path_map.contains(p) {
                        st.path_map.push(p.clone());
                    }
                }
            }
            // 编辑账：成功 write/modify 登记 ID + 签名（镜像据此区分正常进展与原样重试）
            if tool == "write" || tool == "modify" {
                st.last_edit_round = st.rounds;
                if let Some((file, sig)) = edit_signature(&tool, &args) {
                    st.edit_seq += 1;
                    let edit_id = format!("E{}", st.edit_seq);
                    st.edit_ledger.push((edit_id, file, sig, st.rounds));
                }
            }
            // 失败账：write/modify 失败时记账 + 信封附事实——
            if !ok && (tool == "write" || tool == "modify") {
                if let Some(fp) = args
                    .get("file")
                    .or_else(|| args.get("path"))
                    .or_else(|| args.get("target"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                {
                    let fp = fp.to_string();
                    let err_type = crate::tools::contract::extract_error_type(&env);
                    let entry = st.same_file_fails.entry(fp.clone()).or_insert((0, err_type.clone()));
                    entry.0 += 1;
                    entry.1 = err_type;
                    let (_, etype) = st.same_file_fails.get(&fp).cloned().unwrap_or((0, "".into()));
                    // 失败归因范本填充：分类 + 证据 + 对照（账本成功轨迹）+ 建议
                    let attr = crate::tools::contract::classify_failure(&etype);
                    let base = fp.rsplit(['/', '\\']).next().unwrap_or(&fp).to_lowercase();
                    let contrast = st
                        .read_memo
                        .iter()
                        .filter(|(k, _)| *k != &fp && k.rsplit(['/', '\\']).next().map(|x| x.to_lowercase()) == Some(base.clone()))
                        .map(|(k, (round, _))| format!("你第 {round} 轮读过 {}", k))
                        .next()
                        .unwrap_or_default();
                    // 归因范本（话术资产）：mirror.md attribution 段编译期嵌入，改文案不动代码
                    let mut msg = crate::agent::context::render(
                        &crate::agent::context::section(include_str!("../../../prompts/workflow/mirror.md"), "attribution"),
                        &[
                            ("rounds", &st.rounds.to_string()),
                            ("tool", &tool),
                            ("error", &etype),
                            ("category", &attr.category),
                            ("evidence", &attr.evidence),
                            ("contrast", "__CONTRAST__"),
                            ("advice", &attr.advice),
                        ],
                    );
                    if contrast.is_empty() {
                        msg = msg.replace("· 对照：__CONTRAST__\n", "");
                    } else {
                        msg = msg.replace("__CONTRAST__", &contrast);
                    }
                    msg.push_str(&format!("· 建议：{}", attr.advice));
                    // 回照维度补全：归因照"错误的分类"，这里照"模型自己刚发送的原始参数"——
                    if let Ok(raw) = serde_json::to_string(&args) {
                        let mut prev: String = raw.chars().take(400).collect();
                        if raw.chars().count() > 400 {
                            prev.push('…');
                        }
                        msg.push_str(&format!("· 你实际发送：{prev}\n"));
                    }
                    env.content.push(crate::mcp::envelope::ContentPart::text(msg));
                }
            }
            // 复现账：repro/probe 脚本登记，修复落地后循环自动重跑
            if tool == "write" {
                if let Some(p) = args
                    .get("file")
                    .or_else(|| args.get("path"))
                    .or_else(|| args.get("target"))
                    .and_then(|v| v.as_str())
                {
                    let fname = p.rsplit(['/', '\\']).next().unwrap_or("").to_lowercase();
                    if (fname.starts_with("repro") || fname.starts_with("_repro")
                        || fname.starts_with("_probe") || fname.starts_with("_dbg"))
                        && fname.ends_with(".py")
                    {
                        st.repro_script = Some(p.to_string());
                    }
                }
            }
            // 复现脚本的运行结果直接入账（模型自己跑的也算事实）
            if tool == "run" {
                if let Some(script) = &st.repro_script {
                    let short = script.rsplit(['/', '\\']).next().unwrap_or("");
                    if let Some(cmd) = args.get("command").and_then(|v| v.as_str()) {
                        if cmd.contains(short) {
                            let crashed = !ok || env.content.iter().filter_map(|c| c.text.clone())
                                .any(|t| t.contains("Traceback") || t.contains("Error"));
                            st.repro_last = Some((st.rounds, crashed));
                            if st.repro_cwd.is_none() {
                                if let Some(cwd) = args.get("cwd").and_then(|v| v.as_str()) {
                                    st.repro_cwd = Some(cwd.to_string());
                                }
                            }
                        }
                    }
                }
            }
            // 阅读账（ReadMemo）：read 成功记账；重复读 = 信封附事实标注（对账机制）
            if tool == "read" {
                let mut paths: Vec<String> = Vec::new();
                if let Some(arr) = args.get("paths").and_then(|v| v.as_array()) {
                    for pv in arr {
                        if let Some(p) = pv.as_str() {
                            paths.push(p.to_string());
                        }
                    }
                }
                if let Some(p) = args.get("path").and_then(|v| v.as_str()) {
                    paths.push(p.to_string());
                }
                for p in paths {
                    if st.read_seen.insert(p.clone()) {
                        st.last_explore_round = st.rounds;
                    }
                    let (first_round, count) = {
                        let e = st.read_memo.entry(p.clone()).or_insert((st.rounds, 0));
                        e.1 += 1;
                        *e
                    };
                    if count >= 2 {
                        env.content.push(crate::mcp::envelope::ContentPart::text(format!(
                            "〔阅读账〕{} 是第 {} 次读取（首读于第 {} 轮，此前内容仍在你的上下文中）——先回忆已读内容，确需新段落再用行号区间读取。",
                            p, count, first_round
                        )));
                    }
                    if let Some(memo) = self.readmemo_maintain(&p, first_round == st.rounds).await {
                        env.content.push(crate::mcp::envelope::ContentPart::text(memo));
                    }
                }
            }
        }
        self.emit_tool(&call_id, &tool, &args, &env, ok).await;
        (env, ok)
    }

    async fn readmemo_maintain(&self, path: &str, is_first_in_task: bool) -> Option<String> {
        let meta = tokio::fs::metadata(path).await.ok()?;
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let size = meta.len();
        let workspace = crate::db::repos::get_session_workspace(&self.ctx.pool, &self.session_id)
            .await
            .ok()
            .flatten()?;
        let key = format!("readmemo:{}", path.replace('\\', "/"));
        let old = crate::db::repos::recall_memory_by_key(&self.ctx.pool, &workspace, &key)
            .await
            .ok()
            .flatten();
        let old_fp = old.as_ref().and_then(|row| {
            let v: serde_json::Value = serde_json::from_str(&row.value).ok()?;
            Some((
                v.get("mtime")?.as_u64()?,
                v.get("size")?.as_u64()?,
                v.get("skeleton")?.as_str()?.to_string(),
            ))
        });
        let unchanged = matches!(&old_fp, Some((m, s, _)) if *m == mtime && *s == size);
        // 任务内首读 + 指纹变化（或无旧记忆）→ 落/刷新骨架
        if !unchanged && is_first_in_task {
            if let Ok(content) = tokio::fs::read_to_string(path).await {
                if content.len() <= 2 * 1024 * 1024 {
                    let (lines, skeleton) = extract_skeleton(&content);
                    let val = serde_json::json!({
                        "mtime": mtime, "size": size, "lines": lines, "skeleton": skeleton,
                    })
                    .to_string();
                    let _ = crate::db::repos::remember(
                        &self.ctx.pool,
                        &workspace,
                        &self.session_id,
                        &key,
                        &val,
                        "note",
                        0,
                    )
                    .await;
                }
            }
        }
        match (unchanged, is_first_in_task) {
            // 首读 + 文件没变：回填骨架（跨任务复用的主收益点）
            (true, true) => {
                let (_, _, sk) = old_fp.unwrap();
                Some(format!(
                    "〔读缓存命中〕{} 与上次读取版本一致（mtime/size 未变，共 {} 字节）——符号骨架：\n{}\n（需要精确内容就按上面行号定向读，勿再全量重读）",
                    path,
                    size,
                    clip_utf8(&sk, 4 * 1024)
                ))
            }
            // 首读 + 文件已变更：旧骨架刚刷新，提醒模型别拿旧印象干活
            (false, true) if old.is_some() => Some(format!(
                "〔读缓存〕{} 较上次读取已变更（旧符号骨架已刷新），以下判断以本次读取为准。",
                path
            )),
            _ => None,
        }
    }

    /// 工具开始拍：running 卡片先上屏（契约：running → done 两拍）。只在真正执行前发——
    async fn emit_tool_running(&self, call_id: &str, name: &str, args: &Value) {
        let path = crate::sse::tool_path_from_args(args);
        let action = crate::sse::tool_action_zh(name);
        let reason = args.get("reason").and_then(|v| v.as_str()).unwrap_or("");
        self.emit(
            crate::sse::EV_TOOL,
            json!({
                "tools": [{
                    "name": name, "path": path,
                    "status": "running", "action": action,
                    "step_id": call_id, "reason": reason,
                }],
            }),
        )
        .await;
    }

    /// 后台长任务活性心跳：每 4s 一拍（只广播不落库）。工具完成/取消/超时即随
    async fn progress_heartbeat(&self, call_id: &str, tool: &str) {
        let started = std::time::Instant::now();
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(4)).await;
            crate::sse::emit_ephemeral(
                self.ctx,
                &self.session_id,
                crate::sse::EV_PROGRESS,
                crate::sse::with_run(json!({
                    "step_id": call_id,
                    "tool": tool,
                    "elapsed_ms": started.elapsed().as_millis() as u64,
                    "phase": "running",
                }), &self.run_id),
            );
        }
    }

    /// 工具事件（前端时间线契约：running → done 两拍）
    async fn emit_tool(&self, call_id: &str, name: &str, args: &Value, env: &ToolEnvelope, ok: bool) {
        let path = crate::sse::tool_path_from_args(args);
        let action = crate::sse::tool_action_zh(name);
        let status = if ok { "success" } else { "error" };
        // 摘要优先 render_full（干净渲染文本）；为空才退化拼原始 content。
        let summary = env
            .render_full
            .clone()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| env.content.iter().filter_map(|c| c.text.clone()).collect::<String>());
        // reason 原样透传（registry 注入的一等意图；空则前端回落旁白推导）
        let reason = args
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        self.emit(
            crate::sse::EV_TOOL,
            json!({
                "tools": [{
                    "name": name, "args": args, "path": path,
                    "status": status, "duration_ms": env.duration_ms,
                    "action": action,
                    "result_summary": summary,
                    "step_id": call_id,
                    "reason": reason,
                    "exit_code": if ok { 0 } else { 1 },
                }],
            }),
        )
        .await;
    }

    /// 会话工作区根（会话库有值且可达 → 用它；否则回落项目根）。
    async fn workspace_root(&self) -> std::path::PathBuf {
        let ws = crate::db::repos::get_session_workspace(&self.ctx.pool, &self.session_id)
            .await
            .ok()
            .flatten();
        ws.map(std::path::PathBuf::from)
            .filter(|p| p.is_dir())
            .unwrap_or_else(crate::tools::fs_read::project_root)
    }

    /// 状态镜像：纯事实零判定；行动卡壳提前镜像 + 每 10 轮常规镜像（三账摘要）。
    async fn mirror_if_stuck(&self, st: &mut Ledger, max_rounds: u32) {
        let total_fails: u32 = st.same_file_fails.values().map(|(c, _)| *c).sum();
        if total_fails >= 3 && st.rounds.saturating_sub(st.last_edit_preflight) >= 3 {
            let mut fam: std::collections::HashMap<String, u32> = Default::default();
            for (_, etype) in st.same_file_fails.values() {
                *fam.entry(etype.clone()).or_insert(0) += 1;
            }
            let mut fams: Vec<(String, u32)> = fam.into_iter().collect();
            fams.sort_by(|a, b| b.1.cmp(&a.1));
            let mut cures = String::new();
            for (fam_name, _n) in fams.iter().take(3) {
                let cure = match fam_name.as_str() {
                    "REPLACEMENT_REQUIRED" | "REPLACEMENTS_REQUIRED" =>
                        "find 与 replace 必须成对出现；追加意图用 insert_after（find+新增自动合并）；大段改写怕失配就整文件 write",
                    "FIND_NOT_FOUND" =>
                        "find 必须逐字来自 read/信封定位窗口的真实内容（含缩进/行尾）；内容已过时就先 read 最新版再改",
                    "MODIFY_FILE_PLACEHOLDER" =>
                        "file 必须是真实路径（可先用 list/search 定位 basename）",
                    "FIND_AMBIGUOUS" =>
                        "find 出现多处——多含几行上下文使其唯一，或改用 line 行号模式",
                    _ => "用信封里的【真实内容/定位窗口】重构这次修改（含行号可直接走 line 模式）",
                };
                cures.push_str(&format!("    - {}：{}
", fam_name, cure));
            }
            let msg = format!(
                "【编辑自查】（本任务编辑类报错已累计 {total_fails} 次，勿再犯同类）：
{cures}
发送下一次 modify 前逐条对照；换一次失败就换一个族，别在同族上原样重试。"
            );
            st.items.push(InputItem::user_message(&msg));
            st.last_edit_preflight = st.rounds;
            return;
        }
        // ── 行动卡壳反射：只反射事实，不判定不强停。
        let no_edit = st.rounds.saturating_sub(st.last_edit_round);
        let wt_writes = if st.wt_unavailable {
            None
        } else {
            let root = self.workspace_root().await;
            let r = crate::wt::written_since(&root, st.write_mark);
            if r.is_none() {
                st.wt_unavailable = true;
            }
            r
        };
        let no_write_evidence = matches!(&wt_writes, Some(v) if v.is_empty());
        // 对账：同签名 ≥2 次 = 原样重试，不计入进展。
        let mut by_sig: std::collections::HashMap<(String, u64), Vec<String>> = Default::default();
        for (id, file, sig, _) in st.edit_ledger.iter() {
            by_sig
                .entry((file.clone(), *sig))
                .or_default()
                .push(id.clone());
        }
        let repeated: Vec<String> = by_sig
            .into_iter()
            .filter(|(_, ids)| ids.len() >= 2)
            .map(|((f, _), mut ids)| {
                ids.sort();
                let name = f.rsplit(['/', '\\']).next().unwrap_or(&f).to_string();
                format!("{name}（{}）", ids.join("、"))
            })
            .collect();
        // 卡壳 = 跑够 8 轮、且**自上次镜像以来工作树一个字都没被写过**。
        let action_stuck = (st.rounds >= 8 && no_write_evidence) || !repeated.is_empty();
        if action_stuck && st.rounds.saturating_sub(st.last_mirror) >= 5 {
            let mut lines: Vec<String> = Vec::new();
            if !repeated.is_empty() {
                lines.push(format!(
                    "- 同一段修改被重复提交：{}",
                    repeated.join("、")
                ));
                lines.push(format!(
                    "- 最近一次经 write/modify 的修改在第 {} 轮（已过 {no_edit} 轮）",
                    st.last_edit_round
                ));
                lines.push("- 对账规则：同一位置的相同修改只允许尝试一次——先核实上次结果（可能已生效），再换策略；原样重试不计入进展".to_string());
            }
            match &wt_writes {
                Some(v) if v.is_empty() => lines.push(
                    "- 自上次检查以来，工作树没有源码文件被写入（脚本改动也算）".to_string(),
                ),
                Some(v) => lines.push(format!(
                    "- 工作树近期被写入的文件：{}",
                    v.join("、")
                )),
                None => {}
            }
            let facts = lines.join("\n");
            // 尾部注入（不落 items）：镜像不再插进历史中间——否则下一轮重建历史时它会消失，
            st.pending_tail = Some(crate::agent::context::render(
                &crate::agent::context::section(include_str!("../../../prompts/workflow/mirror.md"), "action"),
                &[
                    ("rounds", &st.rounds.to_string()),
                    ("max_rounds", &max_rounds.to_string()),
                    ("facts", &facts),
                    ("tool_calls", &st.tool_trace.len().to_string()),
                    ("goal", &st.goal.clone()),
                ],
            ));
            st.last_mirror = st.rounds;
            st.write_mark = std::time::SystemTime::now();
            return;
        }
        // 注意：**这里不重置 `write_mark`** —— 它记的是「上次发射镜像的时刻」，是卡壳判据的窗口起点。
        if st.rounds.saturating_sub(st.last_mirror) < MIRROR_MIN_INTERVAL {
            return;
        }
        let recent: Vec<String> = st
            .tool_trace
            .iter()
            .rev()
            .take(6)
            .map(|(n, _)| n.clone())
            .collect();
        let trace = recent.iter().rev().map(|s| s.as_str()).collect::<Vec<_>>().join(" → ");
        let changed = {
            let mut files: Vec<String> = st.changed_files.clone();
            if !st.wt_unavailable {
                let root = self.workspace_root().await;
                if let Some(v) = crate::wt::written_since(&root, st.task_start) {
                    for f in v {
                        if !files.contains(&f) {
                            files.push(f);
                        }
                    }
                }
            }
            files.retain(|f| !f.trim().is_empty());
            if files.is_empty() {
                "无".to_string()
            } else {
                format!("{}（含经 run/脚本的改动）", files.join("、"))
            }
        };
        // 阅读账摘要：已读文件数 + 重复读总次数
        let dup_reads: u32 = st.read_memo.values().map(|(_, n)| n.saturating_sub(1)).sum();
        let reads = if st.read_memo.is_empty() {
            "无".to_string()
        } else if dup_reads > 0 {
            format!("{} 个文件（其中重复读取 {} 次——内容仍在上下文，勿重读）", st.read_memo.len(), dup_reads)
        } else {
            format!("{} 个文件", st.read_memo.len())
        };
        let left = max_rounds.saturating_sub(st.rounds);
        // 镜像只展示前 40 条路径（完整账在纠正器，防镜像膨胀）
        let paths_value = if st.path_map.is_empty() {
            "（尚未确认任何目录）".to_string()
        } else if st.path_map.len() <= 40 {
            st.path_map.join("\n")
        } else {
            format!(
                "{}\n（另有 {} 条已确认路径未展示，纠正器持有全文）",
                st.path_map.iter().take(40).cloned().collect::<Vec<_>>().join("\n"),
                st.path_map.len() - 40
            )
        };
        // 尾部注入（不落 items）：理由同上——保历史前缀逐字稳定。
        st.pending_tail = Some(crate::agent::context::render(
            &crate::agent::context::section(include_str!("../../../prompts/workflow/mirror.md"), "progress"),
            &[
                ("rounds", &st.rounds.to_string()),
                ("max_rounds", &max_rounds.to_string()),
                ("left", &left.to_string()),
                ("reads", &reads),
                ("changed", &changed),
                ("trace", &trace),
                ("paths", &paths_value),
                ("goal", &st.goal),
            ],
        ));
        st.last_mirror = st.rounds;
        // 进度镜像也是一次「发射」→ 同样刷新卡壳判据的窗口起点，免得窗口无限拉长
        st.write_mark = std::time::SystemTime::now();
    }

    /// 收口门：非收敛终止 = 未完成（陈述循环如何结束，不判定工作质量）
    async fn terminate(&self, st: &mut Ledger, reason: &str) -> Spin {
        st.finished = true;
        // 分相耗时汇总（观测埋点）：定位"时间花在决策相还是执行/治理相"。
        tracing::info!(
            session = %self.session_id, rounds = st.rounds,
            decide_ms = st.phase_ms[0], exec_ms = st.phase_ms[1],
            per_round_ms = if st.rounds > 0 { (st.phase_ms[0] + st.phase_ms[1]) / st.rounds as u64 } else { 0 },
            "任务结束·分相耗时汇总（毫秒）"
        );
        crate::db::repos::update_session_status(
            &self.ctx.pool,
            &self.session_id,
            crate::db::repos::SessionStatus::Blocked.as_str(),
        )
        .await?;
        let changed = if st.changed_files.is_empty() {
            "无".to_string()
        } else {
            st.changed_files.join(", ")
        };
        let answer = crate::agent::context::render(
            &crate::agent::context::section(include_str!("../../../prompts/workflow/mirror.md"), "terminate"),
            &[
                ("reason", reason),
                ("rounds", &st.rounds.to_string()),
                ("tool_calls", &st.tool_trace.len().to_string()),
                ("changed", &changed),
            ],
        );
        Ok((answer, st.rounds, st.tool_trace.len() as u32, st.changed_files.clone()))
    }
}

// 协议归一（call_id 全历史唯一）

/// call_id 全历史唯一（防 Duplicate tool output 400）+ 参数缺失不吞动作
fn normalize_calls(st: &Ledger, fcs: Vec<FunctionCall>) -> Vec<FunctionCall> {
    let hist: HashSet<String> = st
        .items
        .iter()
        .filter_map(|it| match it {
            InputItem::Message { tool_calls: Some(tcs), .. } => Some(
                tcs.iter()
                    .filter_map(|tc| tc.get("call_id").and_then(|c| c.as_str()).map(|s| s.to_string()))
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .flatten()
        .collect();
    let mut seen = hist.clone();
    let mut out = Vec::new();
    for mut fc in fcs {
        if seen.contains(&fc.call_id) {
            if !hist.contains(&fc.call_id) {
                continue;
            }
            fc.call_id = format!("{}-r{}", fc.call_id, st.rounds);
        }
        seen.insert(fc.call_id.clone());
        out.push(fc);
    }
    out
}

/// 编辑签名：modify=hash(file+mode+find/replace 前缀)；write=hash(file+content 前缀+长度)。
fn edit_signature(tool: &str, args: &Value) -> Option<(String, u64)> {
    use std::hash::{Hash, Hasher};
    let file = args
        .get("file")
        .or_else(|| args.get("path"))
        .or_else(|| args.get("target"))
        .and_then(|v| v.as_str())?
        .trim()
        .to_string();
    if file.is_empty() {
        return None;
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    file.hash(&mut hasher);
    if tool == "modify" {
        let cs = args.get("change_spec").unwrap_or(args);
        let mode = cs.get("mode").and_then(|v| v.as_str()).unwrap_or("block");
        let find = cs
            .get("find")
            .and_then(|v| v.as_str())
            .map(|s| s.chars().take(80).collect::<String>());
        let line = cs.get("line").and_then(|v| v.as_u64()).unwrap_or(0);
        let replace = cs
            .get("replace")
            .and_then(|v| v.as_str())
            .map(|s| s.chars().take(80).collect::<String>());
        mode.hash(&mut hasher);
        find.hash(&mut hasher);
        line.hash(&mut hasher);
        replace.hash(&mut hasher);
    } else {
        let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
        content.chars().take(80).collect::<String>().hash(&mut hasher);
        content.chars().count().hash(&mut hasher);
    }
    Some((file, hasher.finish()))
}

/// 空转特征：0 工具 + 0 文本 + 输出≤2 token + 思考<40 字（1-token 死循环）。
fn empty_spin_signal(resp: &StreamResult) -> bool {
    resp.function_calls.is_empty()
        && resp.output_text.trim().is_empty()
        && resp.usage.as_ref().map(|u| u.output_tokens <= 2).unwrap_or(true)
        && resp.reasoning.trim().chars().count() < 40
}

/// 无进展止损判据：只陈述「这段时间没有产出」，不判定模型做错了什么。
fn stall_reason(st: &Ledger) -> Option<&'static str> {
    let t = crate::config::settings::tuning();
    stall_reason_of(
        st.stall_count,
        st.rounds,
        st.last_edit_round,
        st.changed_files.is_empty(),
        t.stall_stop_rounds.load(std::sync::atomic::Ordering::Relaxed),
        t.stall_dry_rounds.load(std::sync::atomic::Ordering::Relaxed),
        t.stall_min_rounds.load(std::sync::atomic::Ordering::Relaxed),
    )
}

/// 符号行启发式（Rust / TS / Python / Markdown 通用）：骨架只认这些前缀，别贪。
fn is_skeleton_line(t: &str) -> bool {
    if t.len() > 300 {
        return false;
    }
    const PREFIXES: &[&str] = &[
        "pub fn ", "pub async fn ", "fn ", "async fn ", "pub struct ", "struct ", "pub enum ",
        "enum ", "impl ", "pub trait ", "trait ", "pub mod ", "mod ", "pub const ", "const ",
        "pub static ", "static ", "class ", "def ", "interface ", "export ", "function ",
        "# ", "## ", "### ",
    ];
    PREFIXES.iter().any(|p| t.starts_with(p))
}

/// 从文件全文提取符号骨架：`(总行数, "行号: 符号行" 串)`；cap 60 条 / 6KB（预算硬顶）。
fn extract_skeleton(content: &str) -> (u64, String) {
    const MAX_ITEMS: usize = 60;
    const MAX_BYTES: usize = 6 * 1024;
    let mut sk = String::new();
    for (i, l) in content.lines().enumerate() {
        let t = l.trim_start();
        if is_skeleton_line(t) {
            use std::fmt::Write as _;
            let _ = writeln!(sk, "{}: {}", i + 1, t);
            if sk.lines().count() >= MAX_ITEMS || sk.len() >= MAX_BYTES {
                break;
            }
        }
    }
    (content.lines().count() as u64, sk.trim_end().to_string())
}

/// UTF-8 安全截断（floor_char_boundary 未稳定，手写）：回填注入的体积硬顶。
fn clip_utf8(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut i = max;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    &s[..i]
}

/// 判据本体（纯标量入参 + 阈值显式传入，便于单测——`Ledger` 有 30+ 字段，整体构造成本高）。
fn stall_reason_of(
    stall_count: u32,
    rounds: u32,
    last_edit_round: u32,
    no_files: bool,
    stop_rounds: u32,
    dry_rounds: u32,
    min_rounds: u32,
) -> Option<&'static str> {
    if rounds < min_rounds {
        return None;
    }
    // ① 曾有实体进展、但已连续 stop_rounds 个零进展轮 ⇒ 大概率卡在同一处反复试。
    if last_edit_round > 0 && stall_count >= stop_rounds {
        return Some(
            "无进展止损：已连续多个零进展轮无任何新产出（默认 12 轮，env REAL_STALL_STOP_ROUNDS 可调），\
疑似在绕圈——已收口并交付现有发现（省下的轮次即省下的费用）。\
这不是封死：**直接回复『继续』（可附新思路或纠偏信息），将带着断点与已有发现续跑**；也可把剩余工作拆小后重发。",
        );
    }
    // ② 从未改过文件的长任务 ⇒ 按"只诊断不改动"看待，到点也该有结论了。
    if last_edit_round == 0 && no_files && rounds >= dry_rounds {
        return Some(
            "无进展止损：本任务轮数已达兜底线且未产生任何文件改动（默认 30 轮，env REAL_STALL_DRY_ROUNDS 可调，\
按纯诊断任务处理），已收口并交付已查明的结论。\
若还需继续：**直接回复『继续』（可附新方向）即可带断点续跑**，或拆成更小的题重发。",
        );
    }
    None
}

#[cfg(test)]
#[path = "workflow_tests.rs"]
mod workflow_tests;
