//! 模型能力目录（models.json）——"一键切换"的单一事实源

use crate::model::types::ResponsesRequest;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::OnceLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSpec {
    /// API 参数名（发给厂商的 `model` 字段，**不可显示给人看、不可改写**）。
    pub id: String,
    /// 人读显示名（界面下拉/日志/错误提示用）。
    #[serde(default)]
    pub label: Option<String>,
    pub max_output_tokens: u32,
}

impl ModelSpec {
    /// 显示名（缺省回退 id）——界面/日志统一走这里，杜绝各处自行拼字符串
    pub fn display_name(&self) -> &str {
        self.label.as_deref().unwrap_or(&self.id)
    }
}

/// 上游协议（**厂商差异的唯一表达处**）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    /// OpenAI 兼容：POST {base_url}/chat/completions（GLM、Moonshot、Qwen 等多数厂商）
    #[default]
    ChatCompletions,
    /// DeepSeek 自有 Responses API：POST {base_url}/responses
    Responses,
}

/// 思考参数的厂商怪癖（GLM 的 `thinking.type=enabled` + `clear_thinking` 是它独有的）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingStyle {
    /// 不注入思考扩展（通用）
    #[default]
    None,
    /// GLM 形态：恒发 thinking{type:enabled, clear_thinking} + reasoning_effort
    GlmAlwaysOn,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderSpec {
    /// 档案标识（= 凭据槽前缀：DB 键 `{id}_api_key` / `{id}_base_url`）
    pub id: String,
    /// 人读厂商名（界面展示）
    #[serde(default)]
    pub name: String,
    /// 官方端点（不含协议路径后缀）
    pub base_url: String,
    /// 上游协议（缺省 chat_completions）
    #[serde(default)]
    pub protocol: Protocol,
    /// 是否原生多模态（决定是否向请求注入看图块）
    #[serde(default)]
    pub vision: bool,
    /// 该厂商的官方主机片段（用于"端点与厂商错配"拦截；自建代理不受限）
    #[serde(default)]
    pub hosts: Vec<String>,
    /// 型号召名前缀（**厂商归属的判定依据**）。
    #[serde(default)]
    pub model_prefixes: Vec<String>,
    /// 思考参数怪癖（缺省不注入）
    #[serde(default)]
    pub thinking: ThinkingStyle,
    /// 该厂商的廉价模型（短输出辅助任务路由用；缺省回落主模型）
    #[serde(default)]
    pub cheap_model: Option<String>,
    /// 该厂商的凭据是否供 `web_search` 工具使用。
    #[serde(default)]
    pub provides_web_search: bool,
    pub models: Vec<ModelSpec>,
}

/// 厂商凭据槽名（**唯一取名点**）。
pub fn key_slot(provider_id: &str) -> String {
    format!("{provider_id}_api_key")
}

/// 厂商端点槽名（见 `key_slot` 的说明）
pub fn base_slot(provider_id: &str) -> String {
    format!("{provider_id}_base_url")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Catalog {
    pub version: u32,
    pub providers: Vec<ProviderSpec>,
}

/// 老格式单文件 `models.json`（内含 providers 数组）——**只读兼容，不作为写入目标**。
fn legacy_catalog_file() -> PathBuf {
    if let Ok(p) = std::env::var("REAL_MODELS_FILE") {
        return PathBuf::from(p);
    }
    crate::path::data_root::data_root().join("models.json")
}

/// 一家公司的档案文件：providers/<id>.json
fn provider_file(id: &str) -> PathBuf {
    crate::path::data_root::providers_dir().join(format!("{id}.json"))
}

static CATALOG: OnceLock<Catalog> = OnceLock::new();

/// 加载厂商档案目录（**一家公司一个 JSON 文件**）。
fn load_catalog() -> &'static Catalog {
    CATALOG.get_or_init(|| {
        let dir = crate::path::data_root::providers_dir();
        let mut specs = read_provider_dir(&dir);

        if specs.is_empty() {
            // ② 老单文件迁移 / ③ 无档案落默认
            let legacy = legacy_catalog_file();
            let mut from_legacy = match std::fs::read_to_string(&legacy) {
                Ok(text) => match serde_json::from_str::<Catalog>(&text) {
                    Ok(mut c) => {
                        backfill_specs(&mut c);
                        tracing::info!(
                            from = %legacy.display(),
                            providers = c.providers.len(),
                            "检测到老格式 models.json，迁移为一公司一文件（providers/<id>.json）"
                        );
                        c.providers
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, path = %legacy.display(), "models.json 解析失败，改用默认档案");
                        Vec::new()
                    }
                },
                Err(_) => Vec::new(),
            };
            if from_legacy.is_empty() {
                from_legacy = default_catalog().providers;
            }
            for p in &from_legacy {
                write_provider_file(p);
            }
            specs = from_legacy;
        } else {
            // 权威路径：逐文件回填缺字段（老文件补 protocol/vision/hosts/model_prefixes/label）
            let mut changed_any = false;
            for p in &mut specs {
                if backfill_provider(p) {
                    write_provider_file(p);
                    changed_any = true;
                }
            }
            if changed_any {
                tracing::info!(
                    path = %dir.display(),
                    "厂商档案已回填缺失字段（protocol/vision/hosts/model_prefixes/label）"
                );
            }
        }

        if specs.is_empty() {
            tracing::warn!("厂商档案为空且无默认可用，模型目录为空（设置面板将无型号可选）");
        }
        Catalog {
            version: 1,
            providers: specs,
        }
    })
}

/// 读 providers/ 下全部 *.json（单文件解析失败只跳过该文件，不影响其余厂商）
fn read_provider_dir(dir: &PathBuf) -> Vec<ProviderSpec> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in entries.flatten() {
        let path = e.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<ProviderSpec>(&text) {
                Ok(spec) => out.push(spec),
                Err(err) => tracing::warn!(
                    error = %err, path = %path.display(),
                    "厂商档案解析失败，已跳过该文件（其余厂商不受影响）"
                ),
            },
            Err(err) => tracing::warn!(
                error = %err, path = %path.display(), "厂商档案读取失败，已跳过"
            ),
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// 写单个厂商档案（幂等；失败只告警，不影响本次运行的内存态）
fn write_provider_file(p: &ProviderSpec) {
    let path = provider_file(&p.id);
    if let Ok(text) = serde_json::to_string_pretty(p) {
        match std::fs::write(&path, text) {
            Ok(_) => tracing::info!(
                path = %path.display(),
                "已写入厂商档案（改这个文件即可增删模型/协议/端点，无需重编译）"
            ),
            Err(e) => tracing::warn!(
                error = %e, path = %path.display(),
                "厂商档案写入失败（本次运行仍按内存生效）"
            ),
        }
    }
}

/// 旧 Catalog 形态整体回填（迁移路径用）
fn backfill_specs(c: &mut Catalog) {
    for p in &mut c.providers {
        backfill_provider(p);
    }
}

/// 给单个厂商档案回填缺失字段（返回是否有改动）。
fn backfill_provider(p: &mut ProviderSpec) -> bool {
    let mut changed = false;
    if let Some(known) = known_provider(&p.id) {
        // 仅在用户未显式写 protocol（= 缺省值）时补——DeepSeek 必须 responses，
        if p.protocol == Protocol::default() && known.protocol != Protocol::default() {
            p.protocol = known.protocol;
            changed = true;
        }
        if !p.vision && known.vision {
            p.vision = true;
            changed = true;
        }
        if p.hosts.is_empty() {
            p.hosts = known.hosts.clone();
            changed = true;
        }
        if p.model_prefixes.is_empty() && !known.model_prefixes.is_empty() {
            p.model_prefixes = known.model_prefixes.clone();
            changed = true;
        }
        if p.thinking == ThinkingStyle::default() && known.thinking != ThinkingStyle::default() {
            p.thinking = known.thinking;
            changed = true;
        }
        if p.cheap_model.is_none() {
            if known.cheap_model.is_some() {
                p.cheap_model = known.cheap_model.clone();
                changed = true;
            }
        }
        if p.name.is_empty() {
            p.name = known.name.clone();
            changed = true;
        }
        if !p.provides_web_search && known.provides_web_search {
            p.provides_web_search = true;
            changed = true;
        }
    }
    for s in &mut p.models {
        if s.label.is_none() {
            if let Some(l) = known_label(&s.id) {
                s.label = Some(l.to_string());
                changed = true;
            }
        }
    }
    // 内置登记过、而本档案里缺的型号 → 补回（**只补缺、不改有、不删多余**）。
    if let Some(known) = known_provider(&p.id) {
        for k in &known.models {
            let exists = p.models.iter().any(|s| s.id.eq_ignore_ascii_case(&k.id));
            if !exists {
                // 追加到末尾（**不重排**）：用户可能有意排过序，重排会让手工维护的
                p.models.push(k.clone());
                changed = true;
            }
        }
    }
    changed
}

/// 已内置的厂商档案（**仅供回填与默认生成**；运行时一律以 providers/*.json 为准）。
fn known_provider(id: &str) -> Option<ProviderSpec> {
    match id {
        "deepseek" => Some(ProviderSpec {
            id: "deepseek".into(),
            name: "DeepSeek".into(),
            base_url: "https://api.deepseek.com".into(),
            protocol: Protocol::Responses,
            vision: true,
            hosts: vec!["api.deepseek.com".into()],
            model_prefixes: vec!["deepseek".into()],
            thinking: ThinkingStyle::None,
            cheap_model: None,
            provides_web_search: false,
            models: builtin_models("deepseek"),
        }),
        "glm" => Some(ProviderSpec {
            id: "glm".into(),
            name: "智谱 BigModel".into(),
            base_url: "https://open.bigmodel.cn/api/paas/v4".into(),
            protocol: Protocol::ChatCompletions,
            vision: true,
            hosts: vec!["bigmodel.cn".into()],
            model_prefixes: vec!["glm".into(), "chatglm".into()],
            thinking: ThinkingStyle::GlmAlwaysOn,
            cheap_model: Some("glm-5.3-flash".into()),
            provides_web_search: true,
            models: builtin_models("glm"),
        }),
        _ => None,
    }
}

/// 内置厂商的官方型号表（**唯一来源**，`known_provider` 与 `default_catalog` 共用）。
fn builtin_models(provider_id: &str) -> Vec<ModelSpec> {
    match provider_id {
        "deepseek" => vec![
            // API 名 deepseek-flash（官方 quick_start/pricing 明示 "Use deepseek-flash"）
            ModelSpec {
                id: "deepseek-flash".into(),
                label: Some("DeepSeek V4.1 Flash".into()),
                max_output_tokens: 384_000,
            },
        ],
        "glm" => vec![ModelSpec {
            id: "glm-5.3-flash".into(),
            label: Some("GLM 5.3 Flash".into()),
            max_output_tokens: 131_072,
        }],
        _ => vec![],
    }
}

/// 已知模型的官方版本名（API 名 → 人读名）。
fn known_label(id: &str) -> Option<&'static str> {
    match id {
        "deepseek-flash" => Some("DeepSeek V4.1 Flash"),
        "glm-5.3-flash" => Some("GLM 5.3 Flash"),
        _ => None,
    }
}

/// 模型规格：精确匹配优先，其次最长前缀（glm-5.3-flash-0731 → glm-5.3-flash）
fn prefix_hit(m: &str, id: &str) -> usize {
    if m.starts_with(id) {
        id.len()
    } else if m.chars().count() >= 3 && id.starts_with(m) {
        m.len()
    } else {
        0
    }
}

pub fn spec_for(model: &str) -> Option<&'static ModelSpec> {
    let c = load_catalog();
    let m = model.to_lowercase();
    let mut best: Option<&ModelSpec> = None;
    for p in &c.providers {
        for s in &p.models {
            let id = s.id.to_lowercase();
            if m == id {
                return Some(s);
            }
            let hit = prefix_hit(&m, &id);
            if hit > 0 && best.is_none_or(|b| b.id.len() < hit) {
                best = Some(s);
            }
        }
    }
    best
}

/// 模型所属 provider 的端点（未收录模型 → None，调用方回退既有默认）
pub fn provider_base(model: &str) -> Option<String> {
    provider_of(model).map(|p| p.base_url.clone())
}

// 档案查询（**唯一的厂商判定入口**）

/// 归属判定：型号名是否属于该厂商。
fn belongs(p: &ProviderSpec, model_lower: &str) -> Option<usize> {
    let mut best = 0usize;
    for pre in &p.model_prefixes {
        if pre.is_empty() {
            continue;
        }
        let hit = prefix_hit(model_lower, &pre.to_lowercase());
        if hit > best {
            best = hit;
        }
    }
    for s in &p.models {
        let id = s.id.to_lowercase();
        if model_lower == id {
            // 精确命中：给一个不可能被前缀超越的长度，直接锁定该厂商
            return Some(usize::MAX);
        }
        let hit = prefix_hit(model_lower, &id);
        if hit > best {
            best = hit;
        }
    }
    (best > 0).then_some(best)
}

/// 模型归属的厂商档案（精确匹配优先，其次最长前缀）。
pub fn provider_of(model: &str) -> Option<&'static ProviderSpec> {
    let c = load_catalog();
    let m = model.to_lowercase();
    let mut best: Option<(&ProviderSpec, usize)> = None;
    for p in &c.providers {
        if let Some(hit) = belongs(p, &m) {
            if hit == usize::MAX {
                return Some(p);
            }
            if best.is_none_or(|(_, l)| l < hit) {
                best = Some((p, hit));
            }
        }
    }
    best.map(|(p, _)| p)
}

/// 全部厂商档案（**界面按厂商分组的单一数据源**）。
pub fn provider_list() -> &'static [ProviderSpec] {
    &load_catalog().providers
}

/// 按**厂商 id** 取档案（与 provider_of 的区别：这个按厂商，那个按型号）
pub fn provider_by_id(id: &str) -> Option<&'static ProviderSpec> {
    load_catalog().providers.iter().find(|p| p.id == id)
}

/// 该模型走哪种上游协议（决定发 /responses 还是 /chat/completions）。
pub fn protocol_of(model: &str) -> Protocol {
    provider_of(model).map(|p| p.protocol).unwrap_or_default()
}

/// 该模型是否支持看图（决定是否向请求注入 image 块）。
pub fn model_vision(model: &str) -> bool {
    if model.to_lowercase().contains("vision") {
        return true;
    }
    provider_of(model).map(|p| p.vision).unwrap_or(false)
}

/// 模型是否有档案（= 是否可被本实例服务）。启动 fail-fast 用。
pub fn is_known_model(model: &str) -> bool {
    provider_of(model).is_some()
}

/// 档案目录路径（错误提示里给用户看，告诉他把 JSON 放哪）
pub fn providers_dir_hint() -> String {
    crate::path::data_root::providers_dir().display().to_string()
}

/// 端点与厂商错配判定（**按档案的 hosts 字段**，不含任何硬编码主机名）。
pub fn host_mismatch(provider_id: &str, url: &str) -> Option<String> {
    let c = load_catalog();
    let u = url.to_ascii_lowercase();
    for p in &c.providers {
        if p.id == provider_id {
            continue;
        }
        for h in &p.hosts {
            if !h.is_empty() && u.contains(&h.to_ascii_lowercase()) {
                let own = c
                    .providers
                    .iter()
                    .find(|x| x.id == provider_id)
                    .map(|x| x.base_url.clone())
                    .unwrap_or_default();
                return Some(format!(
                    "base_url 指向 {} 官方端点，但目标模型属于 {} 厂商——应使用 {own}。两者错配会导致 401",
                    p.name, provider_id
                ));
            }
        }
    }
    None
}

/// 廉价模型（短输出辅助任务用）：从档案里挑一个"声明了 cheap_model"的厂商。
pub fn cheap_model_of(has_key: impl Fn(&str) -> bool, main: &str) -> Option<String> {
    let c = load_catalog();
    for p in &c.providers {
        if p.models.iter().any(|s| s.id == main) {
            continue;
        }
        if let Some(cm) = &p.cheap_model {
            // 该厂商必须有可用凭据才算"能挑"
            if has_key(&p.id) {
                return Some(cm.clone());
            }
        }
    }
    None
}

/// 提供 web_search 工具能力的厂商 id（档案声明 `provides_web_search`）。
pub fn web_search_provider() -> Option<&'static ProviderSpec> {
    load_catalog()
        .providers
        .iter()
        .find(|p| p.provides_web_search)
}

/// 请求级钳制（唯一钳制入口）：只在不超限时借用，超限才克隆（热路径零成本）。
pub fn clamp_request(req: &ResponsesRequest) -> Cow<'_, ResponsesRequest> {
    // 只在需要下调时才克隆（热路径零成本）：原值不超限 → 原样借用。
    let orig = req.max_output_tokens;
    let mut out = orig.unwrap_or(0);

    // ① 静态钳制：单值超模型目录上限
    if let Some(s) = spec_for(&req.model) {
        if out > s.max_output_tokens {
            tracing::warn!(
                model = %req.model,
                requested = out,
                limit = s.max_output_tokens,
                "max_output_tokens 超出模型上限，已钳制（目录 models.json）"
            );
            out = s.max_output_tokens;
        }
    }

    match orig {
        Some(o) if out < o => {
            let mut owned = req.clone();
            owned.max_output_tokens = Some(out);
            Cow::Owned(owned)
        }
        _ => Cow::Borrowed(req),
    }
}

/// 目录收录的全部模型名（设置面板下拉合并用）
pub fn model_ids() -> Vec<String> {
    let c = load_catalog();
    let mut ids: Vec<String> = c
        .providers
        .iter()
        .flat_map(|p| p.models.iter().map(|s| s.id.clone()))
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

/// 全部模型的「API 名 → 人读显示名」（逐条查用 display_name_of；本函数用于**枚举对账**）。
pub fn model_display_names() -> Vec<(String, String)> {
    let c = load_catalog();
    c.providers
        .iter()
        .flat_map(|p| {
            p.models
                .iter()
                .map(|s| (s.id.clone(), s.display_name().to_string()))
        })
        .collect()
}

/// 启动自检：目录里若有模型**没登记版本名**（label 缺失且回填表里也没有），告警。
pub fn warn_on_missing_labels() {
    for (id, label) in model_display_names() {
        if id == label {
            tracing::warn!(
                model = %id,
                dir = %providers_dir_hint(),
                "该模型未登记人读版本名（label），界面将退回显示 API 名——\
                 在该模型的厂商档案 JSON 里给 label 填上版本名即可"
            );
        }
    }
}

/// 单个模型的人读显示名（未知模型原样回显，不臆造）
pub fn display_name_of(model_id: &str) -> String {
    let c = load_catalog();
    c.providers
        .iter()
        .flat_map(|p| p.models.iter())
        .find(|s| s.id == model_id)
        .map(|s| s.display_name().to_string())
        .unwrap_or_else(|| model_id.to_string())
}

/// 默认目录（对齐官方）：文件缺失时自动落盘的兜底。
fn default_catalog() -> Catalog {
    Catalog {
        version: 1,
        providers: vec![
            ProviderSpec {
                id: "deepseek".into(),
                name: "DeepSeek".into(),
                base_url: "https://api.deepseek.com".into(),
                // DeepSeek 用自有 Responses API（POST /responses），非 chat/completions
                protocol: Protocol::Responses,
                vision: true,
                hosts: vec!["api.deepseek.com".into()],
                model_prefixes: vec!["deepseek".into()],
                thinking: ThinkingStyle::None,
                cheap_model: None,
                provides_web_search: false,
                models: builtin_models("deepseek"),
            },
            ProviderSpec {
                id: "glm".into(),
                name: "智谱 BigModel".into(),
                base_url: "https://open.bigmodel.cn/api/paas/v4".into(),
                // 智谱是 OpenAI 兼容 chat/completions；思考参数为其独有怪癖（恒开、不可关）
                protocol: Protocol::ChatCompletions,
                vision: true,
                hosts: vec!["bigmodel.cn".into()],
                model_prefixes: vec!["glm".into(), "chatglm".into()],
                thinking: ThinkingStyle::GlmAlwaysOn,
                cheap_model: Some("glm-5.3-flash".into()),
                // 智谱既是模型厂商、也提供 web_search 工具用的搜索 API —— 其 Key 需同步进工具层
                provides_web_search: true,
                models: builtin_models("glm"),
            },
        ],
    }
}

// 厂商档案模板 —— 接入新公司时把它写进 providers/<id>.json 即可

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod catalog_tests;
