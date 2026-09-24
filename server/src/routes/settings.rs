//! 运行时设置接口

use crate::db::repos;
use crate::error::{AppError, AppResult};
use crate::config::settings::mask_key;
use crate::model::catalog::{base_slot, key_slot};
use crate::state::AppState;
use axum::extract::State;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

/// ⚠️ `allow(dead_code)`：本结构是**请求契约** —— 字段的意义是"接受前端提交"，
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct UpdateSettingsReq {
    /// Some("real"/"mock") 更新；None 不变
    pub llm_mode: Option<String>,
    /// Some("") 清空 Key；Some(非空) 设置；None 不变
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub model: Option<String>,
    /// 带此字段时，model 落到**该会话**而不是全局默认（见 `switch_model_for_session`）。
    pub session_id: Option<String>,
    pub default_system_prompt: Option<String>,
    /// 思考模式（E7）：auto/on/off
    pub thinking_mode: Option<String>,
    /// 思考强度（E7）：low/high/max
    pub thinking_effort: Option<String>,
    /// 长期记忆作用域：`shared`（按工作区跨会话共享）| `session`（只在本任务可见）。
    pub memory_scope: Option<String>,
    /// 输出语言（思考/旁白/工具卡片文案）：`zh` | `en`；缺省 `zh`。
    /// 前端界面语言开关（`useLang`）变更时同步写入——模型每轮自读，不随消息传。
    pub output_lang: Option<String>,
    // ---- 收敛/记忆调优（设置面板「调优」页，热生效）----
    pub max_rounds: Option<u64>,
    /// 上下文 token 硬预算（≥10_000 生效；默认 150_000）
    pub compact_trigger: Option<u64>,
    /// 记忆压缩保留原文条数（≥1；默认 20）
    pub compact_keep_raw: Option<u64>,
    /// 记忆压缩压力阈值（累计字符 ≥1_000；默认 60_000）
    pub compact_pressure_chars: Option<u64>,
    /// 任务内保留最近工具输出条数（1..=50；默认 3）
    pub task_keep_outputs: Option<u64>,
    /// 正文保留最近**回合**数（2..=200；默认 2）
    pub archive_keep_recent: Option<u64>,
    /// 正文归档启动字节门槛（≥1_000；默认 60_000）
    pub max_output_tokens: Option<u64>,
    /// 任务内瘦身起切线（字节，128..=4_000_000；默认 2_304）——工具输出超过它才被压。
    pub task_stub_min_bytes: Option<u64>,
    /// read 全文落盘线（字符，1_000..=1_000_000；默认 8_000）——超过才落盘。
    pub read_spill_threshold_chars: Option<u64>,
    /// read 精读落盘线（字符，2_000..=2_000_000；默认 40_000）——`mode=lines` 用这条。
    pub read_lines_spill_threshold_chars: Option<u64>,
    /// 落盘预览保留头部（字符，≤10_000；默认 200）。
    pub spill_preview_head_chars: Option<u64>,
    /// 落盘预览保留尾部（字符，≤10_000；默认 120）。
    pub spill_preview_tail_chars: Option<u64>,
    /// spill 单文件字节硬上限（1..=512 MB；默认 8 MB）。
    pub spill_max_file_bytes: Option<u64>,
    // ---- 无进展止损五档（设置面板「调优」页，热生效）----
    /// 停滞唤醒档（1..=50；默认 4）——达此零进展轮数注入"换策略"提示。
    pub stall_warn_rounds: Option<u64>,
    /// 只读收窄档（1..=50；默认 8）——达此进入只读态：禁 write/modify/run，逼收束或向用户提问。
    pub stall_narrow_rounds: Option<u64>,
    /// 收口停机档（1..=100；默认 12）——达此收束停机（文案带「回复继续可续跑」指引）。
    pub stall_stop_rounds: Option<u64>,
    /// 纯诊断兜底档（5..=200；默认 30）——从未改文件的任务满此轮数收口。
    pub stall_dry_rounds: Option<u64>,
    /// 止损总门限（1..=200）——轮数不足不判停机（见 `DEF_STALL_MIN_ROUNDS`）。
    pub stall_min_rounds: Option<u64>,
    /// 交账宽限（0..=20；默认 3）——触底后先注「交账指令」的缓冲轮数（0=触底即停）。
    pub stall_grace_rounds: Option<u64>,
    /// 数据根目录（用户可选存放位置；Some("") 清除回到默认；重启后生效）
    pub data_dir: Option<String>,
    /// 联网搜索凭据（Some("") 清除；Some(非空) 设置；None 不变）。
    pub web_search_api_key: Option<String>,
}

/// 官方端点常量已收口到 config::settings（单一事实源），此处不再重复定义。

/// model 名 → 厂商 id（**查档案**，不按前缀硬编码）。
fn provider_of(model: &str) -> String {
    if let Some(p) = crate::model::catalog::provider_of(model) {
        return p.id.clone();
    }
    let derived = model
        .split(['-', '.', '/'])
        .next()
        .unwrap_or(model)
        .to_ascii_lowercase();
    tracing::warn!(
        model = %model,
        slot = %derived,
        dir = %crate::model::catalog::providers_dir_hint(),
        "模型不在任何厂商档案中——凭据槽按名首段派生；建议在档案目录补一个 JSON"
    );
    derived
}

/// 厂商人读名（提示文案用；未建档回退 id）
fn provider_name(provider_id: &str) -> String {
    crate::model::catalog::provider_by_id(provider_id)
        .map(|p| p.name.clone())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| provider_id.to_string())
}

/// 端点与厂商错配校验（**按档案声明的 hosts**，不再硬编码主机名）。
fn known_host_mismatch(provider: &str, url: &str) -> Option<String> {
    crate::model::catalog::host_mismatch(provider, url)
}

/// 环境变量注入的 Key 是否属于指定厂商。
fn env_key_for_provider(state: &AppState, provider: &str) -> Option<String> {
    let env_provider = crate::model::catalog::provider_of(&state.cfg.llm_model).map(|p| p.id.clone())?;
    if env_provider != provider {
        return None;
    }
    state.cfg.deepseek_api_key.clone()
}

pub async fn get(State(state): State<AppState>) -> AppResult<Json<Value>> {
    let s = state.settings.read().await;
    let masked = mask_key(s.api_key.as_deref());
    // 各厂商是否已有凭据档案（前端"切到该厂商是否需要补 Key"据此判断）。
    let mut provider_has_key = serde_json::Map::new();
    for (id, v) in s.provider_keys.iter() {
        if !v.trim().is_empty() {
            provider_has_key.insert(id.clone(), json!(true));
        }
    }
    let ws_provider_id = crate::model::catalog::web_search_provider().map(|p| p.id.clone());
    let has_glm_key = ws_provider_id
        .as_deref()
        .map(|id| s.provider_key(id).is_some())
        .unwrap_or(false);
    // 下拉列表 = 计价表 ∪ 能力目录（目录收录但暂无定价的模型也可见可选）
    let mut models = crate::pricing::model_list();
    models.extend(crate::model::catalog::model_ids());
    models.sort();
    models.dedup();
    // 显示名对账表：[{id, label}]——前端下拉显示 label（DeepSeek V4.1 Flash），
    let label_of = |id: &str| {
        crate::model::catalog::display_name_of(id)
    };
    let model_labels: Vec<Value> = models
        .iter()
        .map(|id| json!({ "id": id, "label": label_of(id) }))
        .collect();
    // 厂商分组（界面按厂商分组展示 + 切换模型时取该厂商端点）。
    let providers: Vec<Value> = crate::model::catalog::provider_list()
        .iter()
        .map(|p| {
            json!({
                "id": p.id,
                "name": if p.name.is_empty() { p.id.clone() } else { p.name.clone() },
                "base_url": p.base_url,
                "models": p.models.iter()
                    .map(|m| json!({"id": m.id, "label": m.display_name()}))
                    .collect::<Vec<_>>(),
            })
        })
        .collect();
    // 长期记忆作用域（`settings` 表键 `memory_scope`）——不在 `RuntimeSettings` 里，
    let memory_scope = repos::get_setting(&state.pool, "memory_scope")
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| "shared".to_string());
    Ok(Json(json!({
        "llm_mode": match s.llm_mode { crate::config::LlmMode::Real => "real", _ => "mock" },
        "model": s.model,
        // 当前模型的版本名（DeepSeek V4.1 Flash）——界面显示用，别拿 API 名给人看
        "model_label": crate::model::catalog::display_name_of(&s.model),
        "base_url": s.base_url,
        "has_api_key": s.api_key.is_some(),
        "api_key_masked": masked,
        // GLM 档案是否已有 Key（前端模型选择弹窗据此决定是否要求一次性输入）
        "has_glm_key": has_glm_key,
        // 联网搜索凭据是否已配（新前端据此显示「联网搜索」开关状态；
        "has_web_search_key": crate::tools::web_tools::has_search_key(),
        "web_search_key_masked": repos::get_setting(&state.pool, crate::tools::web_tools::SEARCH_KEY_SETTING)
            .await
            .ok()
            .flatten()
            .as_deref()
            .and_then(|k| mask_key(Some(k))),
        // 工具面是否已挂上 web_search（启动时定，与 has_web_search_key 不一致 = 需重启）
        "web_search_tool_active": crate::tools::web_tools::has_search_key(),
        // 通用版：任一厂商是否已有凭据档案（新前端按厂商判断，无需为每家加字段）
        "provider_has_key": Value::Object(provider_has_key),
        "default_system_prompt": s.default_system_prompt,
        "thinking_mode": s.thinking_mode,
        "thinking_effort": s.thinking_effort,
        // 长期记忆作用域：`shared` = 按工作区跨会话共享；`session` = 只在本任务可见
        "memory_scope": memory_scope,
        "models": models,
        "model_labels": model_labels,
        // 厂商分组（id/name/base_url/型号+版本名）——前端据此分组与取端点
        "providers": providers,
        // 收敛/记忆调优（设置面板「调优」页；保存即热生效）
        "max_rounds": crate::config::settings::tuning_snapshot().max_rounds,
        "compact_trigger": crate::config::settings::tuning_snapshot().compact_trigger,
        "compact_keep_raw": crate::config::settings::tuning_snapshot().compact_keep_raw,
        "compact_pressure_chars": crate::config::settings::tuning_snapshot().compact_pressure_chars,
        "task_keep_outputs": crate::config::settings::tuning_snapshot().task_keep_outputs,
        "archive_keep_recent": crate::config::settings::tuning_snapshot().archive_keep_recent,
        "max_output_tokens": crate::config::settings::tuning_snapshot().max_output_tokens,
        // 出厂默认值快照（九个键与上面同构）：面板文案里的"默认 X"从这里取，
        "tuning_defaults": serde_json::to_value(crate::config::settings::tuning_defaults())
            .unwrap_or(Value::Null),
        // 数据目录（设置面板「数据目录」；标记值 + 当前生效根）
        "data_dir": crate::path::data_root::get_data_dir_setting(),
        "data_root_effective": crate::path::data_root::data_root().display().to_string(),
        "server": { "host": state.cfg.host, "port": state.cfg.port },
    })))
}

/// 会话级切模型：把选择落到本会话，并维护**目标厂商**的凭据档案槽。
async fn switch_model_for_session(
    state: &AppState,
    session_id: &str,
    model: &str,
    api_key: Option<&str>,
    base_override: Option<&str>,
) -> AppResult<()> {
    let provider = provider_of(model);

    if let Some(k) = api_key.map(str::trim).filter(|k| !k.is_empty()) {
        // 内存与 DB 双写：前者本次调用即生效，后者重启后仍在
        state
            .settings
            .write()
            .await
            .provider_keys
            .insert(provider.clone(), k.to_string());
        repos::set_setting(&state.pool, &key_slot(&provider), k).await?;
    }
    if let Some(b) = base_override.map(str::trim).filter(|b| !b.is_empty()) {
        state
            .settings
            .write()
            .await
            .provider_bases
            .insert(provider.clone(), b.to_string());
        repos::set_setting(&state.pool, &base_slot(&provider), b).await?;
    }

    // 档案里有该厂商的 Key 才敢走真实调用；没有就维持原模式，别发出必然 401 的请求
    let has_key = state.settings.read().await.provider_key(&provider).is_some();
    if has_key {
        state.settings.write().await.llm_mode = crate::config::LlmMode::Real;
        repos::set_setting(&state.pool, "llm_mode", "real").await?;
    }

    repos::set_session_model(&state.pool, session_id, Some(model)).await?;
    tracing::info!(session = %session_id, model, provider = %provider, "会话级模型已切换");
    Ok(())
}

pub async fn put(
    State(state): State<AppState>,
    Json(req): Json<UpdateSettingsReq>,
) -> AppResult<Json<Value>> {
    // 切换目标 provider（请求带 model 用之，否则当前模型）——校验段要用，先算
    let (current_model_snapshot, current_base_snapshot) = {
        let s = state.settings.read().await;
        (s.model.clone(), s.base_url.clone())
    };
    let target_model = req.model.clone().unwrap_or(current_model_snapshot);
    let provider = provider_of(&target_model);

    // 前端切模型时会把"当前" base_url 原样回显上来——那是未改动的旧值，不是新选择。
    let stale_echo = req
        .base_url
        .as_deref()
        .map(|u| u.trim_end_matches('/') == current_base_snapshot.trim_end_matches('/'))
        .unwrap_or(false);
    let base_override: Option<&String> = if stale_echo { None } else { req.base_url.as_ref() };

    // ---- 校验 ----
    if let Some(m) = &req.llm_mode {
        if m != "mock" && m != "real" {
            return Err(AppError::Validation("llm_mode 必须是 mock 或 real".into()));
        }
    }
    if let Some(url) = base_override {
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err(AppError::Validation(
                "base_url 必须是 http(s):// 开头的地址".into(),
            ));
        }
        // 官方端点与目标 provider 错配 → 写入前拒（凭据档案在写入侧即保干净）
        if let Some(reason) = known_host_mismatch(&provider, url) {
            return Err(AppError::Validation(reason));
        }
    }
    if let Some(m) = &req.model {
        if m.trim().is_empty() {
            return Err(AppError::Validation("model 不能为空".into()));
        }
    }

    // ---- 会话级切换：写本会话，并让**全局默认同步跟上** ----
    if let (Some(sid), Some(m)) = (req.session_id.as_deref(), req.model.as_deref()) {
        switch_model_for_session(
            &state,
            sid,
            m.trim(),
            req.api_key.as_deref(),
            base_override.map(|s| s.as_str()),
        )
        .await?;
    }

    // ---- 持久化 + 更新内存（一次事务语义：先 DB 后内存，失败即回滚内存不写） ----
    if let Some(key) = &req.api_key {
        if key.trim().is_empty() {
            repos::delete_setting(&state.pool, "api_key").await?;
            state.settings.write().await.api_key = None;
            repos::set_setting(&state.pool, "llm_mode", "mock").await?;
            state.settings.write().await.llm_mode = crate::config::LlmMode::Mock;
            tracing::warn!("API Key 已清空，自动切回 mock 模式");
        } else {
            // 活跃 Key + provider 凭据档案双写（下次一键互切直接命中档案）
            repos::set_setting(&state.pool, "api_key", key).await?;
            repos::set_setting(&state.pool, &key_slot(&provider), key).await?;
            // 注：不再顺带写搜索凭据 —— 搜索凭据已是独立项（web_search_api_key），
            state.settings.write().await.api_key = Some(key.clone());
            repos::set_setting(&state.pool, "llm_mode", "real").await?;
            state.settings.write().await.llm_mode = crate::config::LlmMode::Real;
            tracing::info!(
                "API Key 已更新（掩码 {}，档案 {provider}），自动切换 real 模式",
                mask_key(Some(key)).unwrap_or_default()
            );
        }
    }
    if let Some(m) = &req.llm_mode {
        // 显式传入时仍尊重（兼容旧客户端）
        if m == "mock" || m == "real" {
            repos::set_setting(&state.pool, "llm_mode", m).await?;
            state.settings.write().await.llm_mode = if m == "real" {
                crate::config::LlmMode::Real
            } else {
                crate::config::LlmMode::Mock
            };
        }
    }
    if let Some(url) = base_override {
        repos::set_setting(&state.pool, "base_url", url).await?;
        repos::set_setting(&state.pool, &base_slot(&provider), url).await?;
        state.settings.write().await.base_url = url.clone();
    }
    if let Some(m) = &req.model {
        // 离开当前 provider 前，把当前生效凭据/地址快照进其档案
        let current_model = state.settings.read().await.model.clone();
        let current_provider = provider_of(&current_model);
        if current_provider != provider {
            let cur_key = state.settings.read().await.api_key.clone();
            if let Some(k) = cur_key {
                let pk = key_slot(&current_provider);
                if repos::get_setting(&state.pool, &pk).await?.is_none() {
                    repos::set_setting(&state.pool, &pk, &k).await?;
                }
            }
            let cur_base = state.settings.read().await.base_url.clone();
            let bk = base_slot(&current_provider);
            if repos::get_setting(&state.pool, &bk).await?.is_none() {
                repos::set_setting(&state.pool, &bk, &cur_base).await?;
            }
        }

        repos::set_setting(&state.pool, "model", m).await?;
        state.settings.write().await.model = m.clone();

        // 切模型 = 切 provider：base_url + api_key 从档案联动解析
        if base_override.is_none() {
            let stored_base = repos::get_setting(&state.pool, &base_slot(&provider)).await?;
            let base = stored_base
                .filter(|u| known_host_mismatch(&provider, u).is_none())
                // 档案优先（providers/<id>.json 单一事实源），缺省回退当前端点
                .or_else(|| crate::model::catalog::provider_base(&target_model))
                .unwrap_or_else(|| current_base_snapshot.clone());
            repos::set_setting(&state.pool, "base_url", &base).await?;
            state.settings.write().await.base_url = base;
        }
        let key_provided = req
            .api_key
            .as_deref()
            .map(|k| !k.trim().is_empty())
            .unwrap_or(false);
        if !key_provided {
            match repos::get_setting(&state.pool, &key_slot(&provider)).await? {
                Some(k) => {
                    repos::set_setting(&state.pool, "api_key", &k).await?;
                    let mut s = state.settings.write().await;
                    s.api_key = Some(k);
                    s.llm_mode = crate::config::LlmMode::Real;
                    drop(s);
                    repos::set_setting(&state.pool, "llm_mode", "real").await?;
                }
                None => {
                    // 家族内切换（如 deepseek-chat→deepseek-flash）：当前生效 Key 就是
                    if current_provider == provider {
                        // no-op：活跃 Key 继续生效
                    } else if let Some(env_key) = env_key_for_provider(&state, &provider) {
                        // 环境变量注入的 Key 属于**当前 env 模型那家**：若切回的正是那家，
                        repos::set_setting(&state.pool, "api_key", &env_key).await?;
                        let mut s = state.settings.write().await;
                        s.api_key = Some(env_key.clone());
                        s.llm_mode = crate::config::LlmMode::Real;
                        drop(s);
                        repos::set_setting(&state.pool, "llm_mode", "real").await?;
                        tracing::info!(
                            provider = %provider,
                            "该厂商无凭据档案，已回退 env 注入的 Key（掩码 {}）",
                            mask_key(Some(env_key.as_str())).unwrap_or_default()
                        );
                    } else {
                        // 文案按厂商名生成——新公司接入后无需改这里
                        let name = provider_name(&provider);
                        let hint = crate::model::catalog::provider_by_id(&provider)
                            .map(|p| p.base_url.clone())
                            .unwrap_or_default();
                        return Err(AppError::Validation(format!(
                            "{name} 的 API Key 尚未配置：在模型选择弹窗粘贴一次该厂商的 Key（{hint} 获取），之后可一键互切"
                        )));
                    }
                }
            }
        }
    }
    if let Some(p) = &req.default_system_prompt {
        repos::set_setting(&state.pool, "default_system_prompt", p).await?;
        state.settings.write().await.default_system_prompt = p.clone();
    }
    // ---- E7 思考配置（热更） ----
    if let Some(m) = &req.thinking_mode {
        if !matches!(m.as_str(), "auto" | "on" | "off") {
            return Err(AppError::Validation(
                "thinking_mode 必须是 auto/on/off".into(),
            ));
        }
        repos::set_setting(&state.pool, "thinking_mode", m).await?;
        state.settings.write().await.thinking_mode = m.clone();
    }
    if let Some(e) = &req.thinking_effort {
        // 契约值 = GLM「始终思考」模型接受的集合；medium 会被上游 API 400
        if !matches!(e.as_str(), "low" | "high" | "max") {
            return Err(AppError::Validation(
                "thinking_effort 必须是 low/high/max".into(),
            ));
        }
        repos::set_setting(&state.pool, "thinking_effort", e).await?;
        state.settings.write().await.thinking_effort = e.clone();
    }
    // ---- 长期记忆作用域（热更；读侧 = 注入，写侧 = 落库，共用 `repos::memory_scope_shared`）----
    if let Some(v) = &req.memory_scope {
        if !matches!(v.as_str(), "shared" | "session") {
            return Err(AppError::Validation(
                "memory_scope 必须是 shared/session".into(),
            ));
        }
        repos::set_setting(&state.pool, "memory_scope", v).await?;
    }
    // ---- 输出语言（全局设置；`agent/output_lang.rs` 是唯一权威）----
    if let Some(v) = &req.output_lang {
        let lang = crate::agent::output_lang::validate(v)?;
        repos::set_setting(&state.pool, crate::agent::output_lang::SETTING_KEY, lang.code()).await?;
    }
    // ---- 收敛/记忆调优（校验+原子热更+落库；非法值 400 带原因直传前端） ----
    // ⚠️ 这张表**有意只放 11 项** = max_rounds + spill 5 + 止损 5，与 `config/settings.rs`
    // 的启动回读表 `TUNING_KEYS` **逐项对齐** —— 落库的必须能被读回，否则重启即丢。
    // 其余 7 项（compact_trigger / compact_keep_raw / compact_pressure_chars /
    // task_keep_outputs / archive_keep_recent / max_output_tokens / task_stub_min_bytes）
    // 前端 `tuningPatch` 会发、`UpdateSettingsReq` 也声明了，但**刻意不列**：
    // 放开它们会改掉既有会话的收敛/压缩行为。2026-09-25 确认过要锁着，**不是接线漏了**。
    // 谁要放开某一项，先想清楚它会怎么改既有会话的行为，再动这张表。
    let tuning_fields: [(&str, Option<u64>); 12] = [
        ("max_rounds", req.max_rounds),
        ("read_spill_threshold_chars", req.read_spill_threshold_chars),
        (
            "read_lines_spill_threshold_chars",
            req.read_lines_spill_threshold_chars,
        ),
        ("spill_preview_head_chars", req.spill_preview_head_chars),
        ("spill_preview_tail_chars", req.spill_preview_tail_chars),
        ("spill_max_file_bytes", req.spill_max_file_bytes),
        ("stall_warn_rounds", req.stall_warn_rounds),
        ("stall_narrow_rounds", req.stall_narrow_rounds),
        ("stall_stop_rounds", req.stall_stop_rounds),
        ("stall_dry_rounds", req.stall_dry_rounds),
        ("stall_min_rounds", req.stall_min_rounds),
        ("stall_grace_rounds", req.stall_grace_rounds),
    ];
    for (key, value) in tuning_fields {
        if let Some(n) = value {
            crate::config::settings::set_tuning(key, n)
                .map_err(AppError::Validation)?;
            repos::set_setting(&state.pool, key, &n.to_string()).await?;
        }
    }

    // ---- 数据目录（设置面板「数据目录」；写标记文件，重启后生效） ----
    if let Some(d) = &req.data_dir {
        let d = d.trim();
        crate::path::data_root::set_data_dir_setting(if d.is_empty() { None } else { Some(d) })
            .map_err(AppError::Validation)?;
    }

    // ---- 联网搜索凭据（独立项，与模型 Key 无关）----
    if let Some(k) = &req.web_search_api_key {
        let k = k.trim();
        if k.is_empty() {
            repos::delete_setting(&state.pool, crate::tools::web_tools::SEARCH_KEY_SETTING).await?;
            crate::tools::web_tools::set_search_key(None);
            tracing::info!("联网搜索凭据已清空（重启后 web_search 从工具面撤下）");
        } else {
            repos::set_setting(&state.pool, crate::tools::web_tools::SEARCH_KEY_SETTING, k).await?;
            crate::tools::web_tools::set_search_key(Some(k.to_string()));
            tracing::info!("联网搜索凭据已更新（掩码 {}，重启后生效）", mask_key(Some(k)).unwrap_or_default());
        }
    }

    get(State(state)).await
}

/// POST /api/settings/test — 探测连通性（按当前模型解析出的端点，与主链路同源）
pub async fn test(State(state): State<AppState>) -> AppResult<Json<Value>> {
    let (base_url, _) = {
        let s = state.settings.read().await;
        s.credentials_for(&s.model.clone())
    };
    let base_url = base_url.trim_end_matches('/').to_string();
    let start = std::time::Instant::now();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .map_err(|e| AppError::Internal(format!("构建探测 client 失败: {e}")))?;

    let resp = client.get(&base_url).send().await;
    let latency_ms = start.elapsed().as_millis() as u64;

    match resp {
        Ok(r) => {
            let status = r.status().as_u16();
            // 任何 HTTP 响应（含 401/404）都说明 URL 可达、服务在线
            let note = if status == 401 {
                "服务在线 · 地址可达（探测未带密钥，属正常）"
            } else {
                "服务可达"
            };
            Ok(Json(json!({
                "ok": true,
                "url": base_url,
                "status": status,
                "latency_ms": latency_ms,
                "note": note,
            })))
        }
        Err(e) => Ok(Json(json!({
            "ok": false,
            "url": base_url,
            "latency_ms": latency_ms,
            "error": e.to_string(),
        }))),
    }
}
