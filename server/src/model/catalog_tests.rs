//! model/catalog.rs 的测试外置（部门盘查：测试全部移出核心文件）
use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glm_flash_clamped_to_official_limit() {
        let over = ResponsesRequest::builder("glm-5.3-flash").max_output_tokens(393_216).build();
        assert_eq!(clamp_request(&over).max_output_tokens, Some(131_072));
        let within = ResponsesRequest::builder("glm-5.3-flash").max_output_tokens(65_536).build();
        assert!(matches!(clamp_request(&within), Cow::Borrowed(_)));
    }

    #[test]
    fn deepseek_keeps_full_limit() {
        // 新规格名上限 384K：请求恰在上限内 → 不钳制（借用原值）
        let req = ResponsesRequest::builder("deepseek-flash")
            .max_output_tokens(384_000)
            .build();
        assert!(matches!(clamp_request(&req), Cow::Borrowed(_)));
    }

    #[test]
    fn unknown_model_passthrough() {
        let req = ResponsesRequest::builder("qwen3-8b-q4").max_output_tokens(999_999).build();
        assert!(matches!(clamp_request(&req), Cow::Borrowed(_)));
    }

    #[test]
    fn prefix_match_hits_longest_spec() {
        let req = ResponsesRequest::builder("glm-5.3-flash-0731").max_output_tokens(200_000).build();
        let clamped = clamp_request(&req);
        assert_eq!(clamped.max_output_tokens, Some(131_072));
        assert_eq!(provider_base("glm-5.3-flash-0731").as_deref(), Some("https://open.bigmodel.cn/api/paas/v4"));
    }

    #[test]
    fn provider_base_exact_and_missing() {
        assert_eq!(
            provider_base("deepseek-flash").as_deref(),
            Some("https://api.deepseek.com")
        );
        assert_eq!(provider_base("nonexistent-model"), None);
    }

    #[test]
    fn clamp_request_borrows_when_within_limit() {
        let req = ResponsesRequest::builder("glm-5.3-flash").build();
        let clamped = clamp_request(&req);
        assert!(matches!(clamped, Cow::Borrowed(_)));
    }

    #[test]
    fn clamp_request_clones_when_over_limit() {
        let req = ResponsesRequest::builder("glm-5.3-flash")
            .max_output_tokens(393_216)
            .build();
        let clamped = clamp_request(&req);
        match clamped {
            Cow::Owned(o) => assert_eq!(o.max_output_tokens, Some(131_072)),
            Cow::Borrowed(_) => panic!("超限必须克隆钳制"),
        }
    }

    #[test]
    fn clamp_never_crushes_by_byte_estimate() {
        // 回归：动态"字节估算"钳制已整体移除——输入仅 9.5 万真实 token
        let mut req = ResponsesRequest::builder("glm-5.3-flash")
            .max_output_tokens(131_072)
            .build();
        let big = crate::model::types::InputItem::user_message(&"字".repeat(2_000_000));
        req.input = vec![big];
        // 动态段不参与钳制（静态钳制下 131072 未超 glm 上限）
        assert!(matches!(clamp_request(&req), Cow::Borrowed(_)),
            "字节估算钳制已移除，超大输入应原样放行");
    }

    #[test]
    fn api_name_and_label_are_two_sonames() {
        // API 名（发给上游的 model 参数）必须是官方接口口径；
        assert_eq!(
            display_name_of("deepseek-flash"),
            "DeepSeek V4.1 Flash",
            "界面必须标出版本号 V4.1"
        );
        // 未知模型不臆造版本名，原样回显
        assert_eq!(display_name_of("qwen3-8b-q4"), "qwen3-8b-q4");
    }

    #[test]
    fn every_known_model_has_label() {
        // 防复发：新增模型忘了登记 label，界面就会退回显示 API 名（版本号消失）。
        for (id, label) in model_display_names() {
            assert_ne!(id, label, "模型 {id} 未登记人读显示名（label 缺失，界面将显示 API 名）");
        }
    }

    /// 构造一个"最小档案"（字段全缺省，模拟用户手写的最小 JSON）
    fn minimal_spec(id: &str, models: Vec<ModelSpec>) -> ProviderSpec {
        ProviderSpec {
            id: id.into(),
            name: String::new(),
            base_url: String::new(),
            protocol: Protocol::default(),
            vision: false,
            hosts: vec![],
            model_prefixes: vec![],
            thinking: ThinkingStyle::default(),
            cheap_model: None,
            provides_web_search: false,
            models,
        }
    }

    fn model(id: &str, label: Option<&str>) -> ModelSpec {
        ModelSpec {
            id: id.into(),
            label: label.map(|s| s.to_string()),
            max_output_tokens: 8192,
        }
    }

    #[test]
    fn backfill_fills_missing_fields_without_touching_user_values() {
        // 老档案（单文件 models.json 时代）没有 protocol/vision/hosts/label
        let mut p = minimal_spec(
            "deepseek",
            vec![
                model("deepseek-flash", None),
                // 用户自建模型：未登记 → 不动（不臆造显示名）
                model("my-private-model", None),
                // 用户手改过的 label：绝不能被覆盖
                model("glm-5.3-flash", Some("我的 glm")),
            ],
        );
        assert!(backfill_provider(&mut p), "应报告有改动");
        // 厂商级字段按内置表回填
        assert_eq!(p.protocol, Protocol::Responses, "DeepSeek 必须是 responses 协议");
        assert!(p.vision, "DeepSeek 应回填 vision");
        assert!(!p.hosts.is_empty(), "应回填官方主机（错配校验用）");
        assert_eq!(p.name, "DeepSeek", "应回填厂商名");
        // 型号级：缺的补，手改的不动，未知的不猜
        assert_eq!(p.models[0].label.as_deref(), Some("DeepSeek V4.1 Flash"), "缺 label 应回填");
        assert_eq!(p.models[1].label, None, "未知模型不得臆造显示名");
        assert_eq!(p.models[2].label.as_deref(), Some("我的 glm"), "用户手改值不得覆盖");
        // 幂等：再跑一次不应报告改动
        assert!(!backfill_provider(&mut p), "回填必须幂等");
    }

    #[test]
    fn unknown_provider_gets_no_invented_fields() {
        // 用户自建厂商（不在内置表里）：除 label 外一律不动——
        let mut p = minimal_spec("my-company", vec![model("my-model-x", None)]);
        assert!(!backfill_provider(&mut p), "未知厂商无可回填字段");
        assert_eq!(p.protocol, Protocol::default(), "不得臆造协议");
        assert!(p.hosts.is_empty(), "不得臆造主机");
        assert!(p.base_url.is_empty(), "不得臆造端点");
    }

    #[test]
    fn legacy_json_without_new_fields_still_parses() {
        // 向后兼容：老文件（无 protocol/vision/hosts/label）必须能解析，
        let legacy = r#"{"version":1,"providers":[{"id":"deepseek",
            "name":"DeepSeek","base_url":"https://api.deepseek.com",
            "models":[{"id":"deepseek-flash","max_output_tokens":384000}]}]}"#;
        let c: Catalog = serde_json::from_str(legacy).expect("老格式必须可解析");
        assert_eq!(c.providers[0].models[0].display_name(), "deepseek-flash",
            "无 label 时 display_name 回退 id");
        assert_eq!(c.providers[0].protocol, Protocol::ChatCompletions,
            "缺 protocol 时按通用协议（回填阶段再按内置表纠正 DeepSeek）");
        assert!(!c.providers[0].provides_web_search, "缺省不提供 web_search");
    }

    #[test]
    fn backfill_restores_glm_web_search() {
        // 模拟从老 models.json 迁来的档案：没有 provides_web_search 字段
        let legacy = r#"{
            "id": "glm",
            "name": "智谱 BigModel",
            "base_url": "https://open.bigmodel.cn/api/paas/v4",
            "models": [{"id": "glm-5.3-flash", "max_output_tokens": 131072}]
        }"#;
        let mut p: ProviderSpec = serde_json::from_str(legacy).expect("老档案必须可解析");
        assert!(
            !p.provides_web_search,
            "前提确认：serde 缺省为 false —— 这正是它被静默关掉的原因"
        );

        assert!(backfill_provider(&mut p), "回填应报告发生了变更");
        assert!(
            p.provides_web_search,
            "GLM 的搜索能力必须被回填回来，否则 web_search 永远拿不到 Key"
        );
    }

    /// 反向：**不得给不提供搜索的厂商臆造**这个能力（DeepSeek 没有搜索服务）。
    #[test]
    fn backfill_does_not_invent_web_search_for_deepseek() {
        let legacy = r#"{
            "id": "deepseek",
            "name": "DeepSeek",
            "base_url": "https://api.deepseek.com",
            "models": [{"id": "deepseek-flash", "max_output_tokens": 384000}]
        }"#;
        let mut p: ProviderSpec = serde_json::from_str(legacy).expect("可解析");
        backfill_provider(&mut p);
        assert!(
            !p.provides_web_search,
            "DeepSeek 不提供搜索服务 —— 内置表是 false，不得臆造"
        );
    }

    #[test]
    fn single_provider_file_parses_standalone() {
        // 一公司一文件的形态：文件内容**就是一个 ProviderSpec**（无外层包装）。
        let one = r#"{
            "id": "moonshot",
            "name": "月之暗面",
            "base_url": "https://api.moonshot.cn/v1",
            "models": [{"id": "kimi-k2", "label": "Kimi K2", "max_output_tokens": 262144}]
        }"#;
        let p: ProviderSpec = serde_json::from_str(one).expect("单厂商档案必须可解析");
        assert_eq!(p.id, "moonshot");
        assert_eq!(p.protocol, Protocol::ChatCompletions, "新公司缺省走通用协议");
        assert_eq!(key_slot(&p.id), "moonshot_api_key", "槽名由 id 派生");
        assert_eq!(p.models[0].label.as_deref(), Some("Kimi K2"));
    }

    #[test]
    fn protocol_and_thinking_are_data_driven() {
        // 协议分流与思考怪癖都从档案读——代码里没有厂商名分支。
        assert_eq!(protocol_of("deepseek-flash"), Protocol::Responses);
        assert_eq!(protocol_of("glm-5.3-flash"), Protocol::ChatCompletions);
        // 未知模型 → 通用协议（宁可让上游 400 暴露，也不静默走错）
        assert_eq!(protocol_of("brand-new-model"), Protocol::ChatCompletions);
    }

    #[test]
    fn vision_is_data_driven_with_name_fallback() {
        assert!(model_vision("deepseek-flash"), "档案声明 vision");
        assert!(model_vision("glm-5.3-flash"), "档案声明 vision");
        // 名字自带 vision 标记的兜底（未知厂商也能识别）
        assert!(model_vision("foo-vision-exp"));
        assert!(!model_vision("brand-new-model"), "未声明即无视觉，不猜");
    }

    #[test]
    fn provider_ownership_survives_empty_model_list() {
        let mut p = minimal_spec("acme", vec![]);
        p.model_prefixes = vec!["acme".into()];
        p.vision = true;

        assert_eq!(
            belongs(&p, "acme-vl-9b"),
            Some(4),
            "空型号清单下，仍须按前缀认领（这正是事故的种子）"
        );
        assert_eq!(belongs(&p, "other-model"), None, "别家型号不得被认领");
    }

    /// 归属判定的两路并集：前缀声明为主，`models` 登记为兼容老档案。
    #[test]
    fn provider_ownership_is_union_of_prefixes_and_models() {
        // ① 只有前缀（主路径）
        let mut by_prefix = minimal_spec("acme", vec![]);
        by_prefix.model_prefixes = vec!["acme".into()];
        assert!(belongs(&by_prefix, "acme-anything").is_some());

        // ② 只有 models（老档案：用户没写前缀但把型号列全了）
        let by_models = minimal_spec("acme", vec![model("acme-chat-7b", Some("Acme 7B"))]);
        assert!(belongs(&by_models, "acme-chat-7b").is_some(), "老档案必须继续可认");
        assert!(belongs(&by_models, "acme-chat-7b-0731").is_some(), "前缀延伸仍可认");

        // ③ 都没有 ⇒ 不认（宁可落到"未知厂商"，也不乱认领）
        let bare = minimal_spec("acme", vec![]);
        assert_eq!(belongs(&bare, "acme-x"), None);
    }

    /// 精确命中必须压过前缀：`glm-x` 精确登记在某厂商时，不能被别家的 `glm` 前缀抢走。
    #[test]
    fn exact_model_hit_outranks_prefix() {
        // 前缀更长者优先（既有语义，回归保护）
        let long = minimal_spec("p-long", vec![]);
        let mut long = long;
        long.model_prefixes = vec!["glm-5".into()];
        let short = minimal_spec("p-short", vec![]);
        let mut short = short;
        short.model_prefixes = vec!["glm".into()];
        assert_eq!(belongs(&long, "glm-5.3-flash"), Some(5));
        assert_eq!(belongs(&short, "glm-5.3-flash"), Some(3));
        // ⇒ provider_of 取更长命中者
    }

    /// 回填必须把 `model_prefixes` 补进老档案（否则存量档案修不好）。
    #[test]
    fn backfill_fills_model_prefixes() {
        let mut p = minimal_spec("deepseek", vec![]);
        assert!(p.model_prefixes.is_empty(), "前提：老档案没有该字段");
        assert!(backfill_provider(&mut p), "应报告有改动");
        assert_eq!(
            p.model_prefixes,
            vec!["deepseek".to_string()],
            "DeepSeek 的归属前缀必须被回填，否则存量档案仍然认不出自己的型号"
        );
        assert!(!backfill_provider(&mut p), "回填必须幂等");
    }

    /// 反向：不得给未知厂商臆造归属前缀（否则会认领别家型号）。
    #[test]
    fn backfill_does_not_invent_prefixes_for_unknown_provider() {
        let mut p = minimal_spec("my-company", vec![model("my-model-x", None)]);
        assert!(!backfill_provider(&mut p), "未知厂商无可回填");
        assert!(p.model_prefixes.is_empty(), "不得臆造归属前缀");
    }

    #[test]
    fn backfill_restores_builtin_models_when_list_is_empty() {
        let mut p = minimal_spec("deepseek", vec![]);
        assert!(backfill_provider(&mut p), "应报告有改动");
        assert_eq!(p.models.len(), 1, "内置登记过的型号必须被补回");
        assert_eq!(p.models[0].id, "deepseek-flash");
        assert_eq!(
            p.models[0].max_output_tokens, 384_000,
            "补回的条目要带官方上限，否则钳制失效"
        );
        assert!(!backfill_provider(&mut p), "回填必须幂等（第二次不再变更）");
    }

    /// 补回型号时**绝不能动用户的条目**：手改的 label、自建型号、已有型号的排序都要保留。
    #[test]
    fn backfill_restores_models_without_touching_user_entries() {
        let mut p = minimal_spec(
            "deepseek",
            vec![
                model("my-private-model", Some("自建")),
                model("deepseek-flash", Some("我的闪")),
            ],
        );
        backfill_provider(&mut p);
        assert_eq!(p.models.len(), 2, "已登记过的型号不得重复追加");
        assert_eq!(p.models[0].id, "my-private-model", "用户自建型号必须保留且不重排");
        assert_eq!(p.models[1].label.as_deref(), Some("我的闪"), "手改 label 不得覆盖");
    }

    /// 未知厂商：型号清单为空就空着，不得从别处搬型号进来。
    #[test]
    fn backfill_never_invents_models_for_unknown_provider() {
        let mut p = minimal_spec("my-company", vec![]);
        backfill_provider(&mut p);
        assert!(p.models.is_empty(), "未知厂商不得臆造任何型号条目");
    }

    #[test]
    fn host_mismatch_uses_archive_declared_hosts() {
        // 按档案 hosts 判定，而不是硬编码主机名
        assert!(host_mismatch("glm", "https://api.deepseek.com").is_some(),
            "glm 用 DeepSeek 端点应被判错配");
        assert!(host_mismatch("deepseek", "https://open.bigmodel.cn/api/paas/v4").is_some(),
            "deepseek 用智谱端点应被判错配");
        assert!(host_mismatch("glm", "https://proxy.mycorp.com/v4").is_none(),
            "自建代理不在任何 hosts 里，不受限");
        assert!(host_mismatch("glm", "https://open.bigmodel.cn/api/paas/v4").is_none(),
            "自家端点当然不报错");
    }

    #[test]
    fn is_known_model_reflects_archive() {
        assert!(is_known_model("deepseek-flash"));
        assert!(is_known_model("glm-5.3-flash"));
        assert!(!is_known_model("nonexistent-xyz"));
    }

    #[test]
    fn cheap_model_comes_from_archive_not_hardcode() {
        // 声明了 cheap_model 且该厂商有 Key → 用它的廉价型号
        let with_glm_key = |id: &str| id == "glm";
        assert_eq!(
            cheap_model_of(with_glm_key, "deepseek-flash").as_deref(),
            Some("glm-5.3-flash")
        );
        // 没 Key → 不挑（回落主模型）
        let no_keys = |_: &str| false;
        assert_eq!(cheap_model_of(no_keys, "deepseek-flash"), None);
        // 主模型就是那家 → 不重复挑自己
        assert_eq!(cheap_model_of(with_glm_key, "glm-5.3-flash"), None);
    }
}
