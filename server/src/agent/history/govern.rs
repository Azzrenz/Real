//! 上下文治理 —— **类型 × 时机 × 部门** 的唯一事实源。

/// 阈值总表（**唯一事实源**）。
pub(crate) mod th {

    // ── L1 回合内（本回合工作面）──────────────────────────────────────
    pub(crate) const INTASK_KEEP_RECENT: usize = 3;
    /// 单条小于它不动 —— 省不出体积却要多破一次前缀缓存。
    pub(crate) const INTASK_MIN_ITEM_BYTES: usize = 2_304;
    /// 回合内去重读的收益门槛（字节）：省不出这个数不值得破前缀缓存。
    pub(crate) const STUB_EST_BYTES: usize = 200;

    // ── 摘要（**所有压缩的统一产物形态**）─────────────────────────────

    /// 结构化摘要的**主体预算**（字符）：结构大纲 / 要点行。
    pub(crate) const DIGEST_CHARS: usize = 600;
    /// 摘要里**保留的头部原文**（字符）。
    pub(crate) const DIGEST_HEAD_CHARS: usize = 100;
    /// 摘要里保留的**尾部原文**（字符）—— 命令的结论与报错都在尾部。
    pub(crate) const DIGEST_TAIL_CHARS: usize = 300;
    /// 摘要里最多列几条结构点（超出只报总数）。
    pub(crate) const DIGEST_MAX_OUTLINE: usize = 24;

    /// `code_outline`（spill 预览的结构大纲）行数下/上限。
    pub(crate) const CODE_OUTLINE_MIN_ITEMS: usize = 5;
    pub(crate) const CODE_OUTLINE_MAX_ITEMS: usize = 16;
    /// 一条 stub 的总体积上限（字符）—— 摘要+定位+指针合计不超它。
    pub(crate) const DIGEST_TOTAL_CHARS: usize = 1_000;

    /// 「钥匙键」的值上限（字符）：只有它能把那次调用**重新走一遍**，不跟着一起截短。
    pub(crate) const LOCATOR_KEY_VALUE_CHARS: usize = 400;
    /// 其余定位键的值上限（字符）。
    pub(crate) const LOCATOR_VALUE_CHARS: usize = 160;
    /// 回合内保留的 reasoning **字节预算**（更早的摘成决策句）。
    pub(crate) const INTASK_KEEP_REASONING_BYTES: usize = 8_000;

    // ── L2 跨回合（旧回合退场）───────────────────────────────────────

    /// 工具回执的**新鲜回合数**：距当前 ≤ 它，原文保留。
    pub(crate) const RETIRE_FRESH_ROUNDS: usize = 3;

    /// 超过它直接压成**路标**（只留定位 + 回取指针）。
    pub(crate) const RETIRE_ROLL_ROUNDS: usize = 5;

    /// 参数压缩：距今 ≥ 几个回合才压（当前回合内模型还在引用）。
    pub(crate) const ARG_MIN_AGE_TASKS: usize = 2;
    /// 参数小于它不动。
    pub(crate) const ARG_MIN_BYTES: usize = 800;
    /// 改写冷却：距今 < 几个回合不再改写（冷却的意义 = 别每轮都破前缀缓存）。
    pub(crate) const REWRITE_COOLDOWN_TASKS: usize = 1;

    /// 装配期改写的**免费时刻**判据（秒）：距上次 LLM 调用 ≥ 它 ⇒ 视为缓存已冷。
    pub(crate) const ASSEMBLY_IDLE_SECONDS: i64 = 300;
    /// 一次改写收益不到这个比例就不动（**基准值**，实际按 input 自适应）。
    pub(crate) const REWRITE_MIN_GAIN_PCT: usize = 8;
    pub(crate) const REWRITE_MAX_GAIN_PCT: usize = 25;
    /// 存量会话兜底阈值（无 turn_logs 记录时的引用断续替代判据）。
    pub(crate) const LEGACY_BLANKET_TOKENS: i64 = 100_000;

    // ── L3 硬顶（全历史爆表）──────────────────────────────────────────
    pub(crate) const HARD_CAP_BYTES: usize = 120_000;
    /// token → 字节系数（把 token_budget 接到字节口径上）。**是估算，别当精确换算用**。
    pub(crate) const BYTES_PER_TOKEN_EST: usize = 3;

    // ── 注入（**每次 run 一次**，不是每轮）─────────────────────────────
    pub(crate) const INJECT_MEMORY_CHARS: usize = 6_000;
    /// 轮任务回灌注入上限（字符）。
    pub(crate) const INJECT_TURN_HINTS_CHARS: usize = 6_000;
    /// 技能索引注入上限（字符）。
    pub(crate) const INJECT_SKILL_IDX_CHARS: usize = 3_000;

    // ── L0 入库 · 全文层落盘（spill）───────────────────────────────────
    pub(crate) const SPILL_THRESHOLD_CHARS: usize = 2_000;

    /// 动态溢出阈值的**下限**（字符）。
    pub(crate) const MIN_DYNAMIC_SPILL_CHARS: usize = 1_000;

    /// 动态溢出阈值的**上限**（字符）。
    pub(crate) const MAX_DYNAMIC_SPILL_CHARS: usize = 4_000;

    /// **动态 spill 阈值** —— 按「当前历史体量 vs 稳态水位」的**反比**缩放，取代一刀切常量。
    pub(crate) fn spill_threshold_for(input_tokens: u64) -> usize {
        // 空历史 ⇒ 最宽松（无压力，不必省）
        if input_tokens == 0 {
            return MAX_DYNAMIC_SPILL_CHARS;
        }
        let anchor = super::window_safe_tokens() as u64;
        if anchor == 0 {
            return SPILL_THRESHOLD_CHARS;
        }
        // 反比：历史越大，允许的单条越小。`saturating_mul` 防溢出（anchor 量级 1e4）。
        let v = (SPILL_THRESHOLD_CHARS as u64).saturating_mul(anchor) / input_tokens.max(1);
        (v as usize).clamp(MIN_DYNAMIC_SPILL_CHARS, MAX_DYNAMIC_SPILL_CHARS)
    }
    /// read 全文（mode != lines）落盘线 —— **出厂默认**。
    ///
    /// ⚠️ 运行期取值走 `mcp::spill::cur_read_spill_threshold_chars()`（设置面板「调优档位」
    /// 可热更）；这个常量只作**默认值锚点与测试断言**用，故标 `allow(dead_code)`。
    ///
    /// 定值依据（2026-09-25）：判据量的是**整个信封**（含 `tool_call_id` / `content[].text` 的
    /// JSON 包装与转义），不是纯正文 —— 转义后正文膨胀约一倍，故 8,000 实际只放行正文 ~4K 字符。
    /// 取近两日 13 条 read 落盘样本（剔除测试会话），其中 12 条属本档，尺寸落在 8,471–31,765、
    /// 中位 ≈15.5K ⇒ 定 16,000 恰好卡住样本中位（详见 `config::settings::DEF_*` 处注释）。
    #[allow(dead_code)]
    pub(crate) const READ_SPILL_THRESHOLD_CHARS: usize =
        crate::config::settings::DEF_READ_SPILL_THRESHOLD_CHARS;
    /// read 精读（mode = lines）落盘线 —— **出厂默认 40,000**（≈1000 行，信封口径）。
    ///
    /// 复核结论：同一批采样只 1 条落在本档（93,049 字节），40,000–93,000 之间无落盘样本
    /// ⇒ 无「冤案」证据，故不动。运行期走 `cur_read_lines_spill_threshold_chars()`。
    #[allow(dead_code)]
    pub(crate) const READ_LINES_SPILL_THRESHOLD_CHARS: usize =
        crate::config::settings::DEF_READ_LINES_SPILL_THRESHOLD_CHARS;
    /// 溢出预览保留的头部原文 —— **出厂默认**（运行期走 `spill::cur_preview_head()`）。
    #[allow(dead_code)]
    pub(crate) const SPILL_PREVIEW_HEAD_CHARS: usize =
        crate::config::settings::DEF_SPILL_PREVIEW_HEAD_CHARS;
    /// 溢出预览保留的尾部原文 —— **出厂默认**（运行期走 `spill::cur_preview_tail()`）。
    #[allow(dead_code)]
    pub(crate) const SPILL_PREVIEW_TAIL_CHARS: usize =
        crate::config::settings::DEF_SPILL_PREVIEW_TAIL_CHARS;
    /// 溢出文件保留天数（启动时按目录 mtime 清理更早的会话目录）。
    pub(crate) const SPILL_RETENTION_DAYS: i64 = 7;

    /// 单个 spill 全文文件的**字节硬上限** —— **出厂默认 8 MB**。
    ///
    /// 由来：原先无上限，一条全盘 `find /` 落了 73.5 MB（≈2,900 万 token），模型按 locator
    /// 回取时单次调用即可撑爆上下文。**复核**：13 条采样里最大落盘仅 93 KB，距 8 MB 有两个
    /// 数量级余量 ⇒ 翻倍改动没有实测依据，回退。运行期走 `spill::cur_spill_max_file_bytes()`。
    #[allow(dead_code)]
    pub(crate) const SPILL_MAX_FILE_BYTES: usize =
        crate::config::settings::DEF_SPILL_MAX_FILE_BYTES;

    /// 非会话 spill 条目的保留天数（**短保留期**）。
    pub(crate) const SPILL_ORPHAN_RETENTION_DAYS: i64 = 1;

    // ── 窗口安全线（L1 回合内 + 装配期共用同一份）──────────────────────
    pub(crate) const WINDOW_TOKENS: u64 = 1_048_576;
    pub(crate) const WINDOW_B_PER_TOKEN_X100: u64 = 359;
    /// 上下文水位线（‰ 窗口）—— **全套治理只认这一个比例**：历史超过它就从最老处删。
    pub(crate) const WINDOW_SAFE_PERMILLE: u64 = 5;

    /// **水位线的字节形式 —— 下游所有阈值的唯一锚点。**
    pub(crate) const WATERMARK_BYTES: usize =
        (WINDOW_TOKENS * WINDOW_B_PER_TOKEN_X100 / 100 * WINDOW_SAFE_PERMILLE / 1000) as usize;

    /// 水位线要装下的轮数 —— 对齐 `RETIRE_FRESH_ROUNDS`（那几轮保原文）。
    pub(crate) const WATERMARK_KEEP_ROUNDS: usize = 3;
    /// 水位线余量（×100）：1.5 倍。
    pub(crate) const WATERMARK_HEADROOM_X100: usize = 150;
    /// 水位线硬上限（字节）：96 KB。
    pub(crate) const WATERMARK_HARD_CAP_BYTES: usize = 96 * 1024;
    /// 单轮新增的 EMA 平滑系数（×100）：新样本占 20%。
    pub(crate) const ROUND_EMA_ALPHA_X100: usize = 20;

    /// 发送前按窗口比例兜底 —— **只为不触发上游 400，不是省钱手段**。
    pub(crate) const SEND_SAFE_PERMILLE: u64 = 70;

    // ── 装配期改写闸门（L2 装配 · 什么时候允许动历史）────────────────────

}

static OBSERVED_ROUND_BYTES: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// 每轮装配时登记「本轮真实新增字节」（EMA 平滑），供水位线自动推导。
pub(crate) fn observe_round_bytes(delta: usize) {
    use std::sync::atomic::Ordering;
    if delta == 0 {
        return;
    }
    let mut cur = OBSERVED_ROUND_BYTES.load(Ordering::Relaxed);
    loop {
        let next = if cur == 0 {
            delta
        } else {
            cur * (100 - th::ROUND_EMA_ALPHA_X100) / 100 + delta * th::ROUND_EMA_ALPHA_X100 / 100
        };
        match OBSERVED_ROUND_BYTES.compare_exchange_weak(
            cur,
            next,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => return,
            Err(actual) => cur = actual,
        }
    }
}

/// 测试专用：把观测值复位。
#[cfg(test)]
pub(crate) fn reset_observed_round_bytes() {
    OBSERVED_ROUND_BYTES.store(0, std::sync::atomic::Ordering::Relaxed);
}

pub(crate) fn watermark_bytes() -> usize {
    let observed = OBSERVED_ROUND_BYTES.load(std::sync::atomic::Ordering::Relaxed);
    let per_round = if observed == 0 {
        th::WATERMARK_BYTES
    } else {
        observed
    };
    let want = per_round
        .saturating_mul(th::WATERMARK_KEEP_ROUNDS)
        .saturating_mul(th::WATERMARK_HEADROOM_X100)
        / 100;
    want.clamp(th::WATERMARK_BYTES, th::WATERMARK_HARD_CAP_BYTES)
}

/// 回合内输出的治理门槛（= 水位线 × 0.6）—— 原 `th::INTASK_TOTAL_BYTES`，随水位线动态化。
pub(crate) fn intask_total_bytes() -> usize {
    watermark_bytes() * 6 / 10
}

/// 回合内改写的最小收益门槛（= 水位线 / 20）—— 原 `th::INTASK_MIN_SAVED_BYTES`。
pub(crate) fn intask_min_saved_bytes() -> usize {
    watermark_bytes() / 20
}

/// 窗口安全线（字节）。
pub(crate) fn window_safe_bytes() -> usize {
    watermark_bytes()
}

/// 同一条线的 token 口径（跨任务归档的入口线用 token 计数）。
pub(crate) fn window_safe_tokens() -> i64 {
    // 从**字节水位**换算，不再另起一条 `WINDOW_TOKENS × permille`
    (window_safe_bytes() as u64 * 100 / th::WINDOW_B_PER_TOKEN_X100) as i64
}

/// `①archive_old_tasks` 的启动线（token）—— **按缓存冷热分档**。
pub(crate) fn deep_cut_tokens(cache_cold: bool) -> i64 {
    // 热路径 = **自动水位**（`window_safe_tokens()`）。
    let _ = cache_cold;
    window_safe_tokens()
}

/// `⑤archive_old_body` 的启动线（字节）—— 与 [`deep_cut_tokens`] 同一条线，只是换成字节口径。
pub(crate) fn deep_cut_bytes(cache_cold: bool) -> usize {
    // 热路径 = **自动水位**（同 `deep_cut_tokens`：写死比例会与自动推导分叉）。
    let _ = cache_cold;
    window_safe_bytes()
}

pub(crate) fn assembly_rewrite_allowed(idle_seconds: i64, input_tokens: i64) -> bool {
    // ① 硬约束：不压就撞上游窗口（400）⇒ 必须压 —— 比的是**发送安全线**不是水位线！
    input_tokens > send_safe_tokens() || cache_is_cold(idle_seconds)
}

/// 发送安全线（token）—— 「请求绝不能超过上游窗口」的那条线，**不是**成本水位线。
pub(crate) fn send_safe_tokens() -> i64 {
    (th::WINDOW_TOKENS * th::SEND_SAFE_PERMILLE / 1000) as i64
}

/// 缓存是否已冷：距上次 LLM 调用 ≥ [`th::ASSEMBLY_IDLE_SECONDS`]。
pub(crate) fn cache_is_cold(idle_seconds: i64) -> bool {
    idle_seconds >= th::ASSEMBLY_IDLE_SECONDS
}

// ── 装配期改写的**收益门槛**（五条改写路径共用一把尺子）──────────────────

/// 改写门槛（**按 input 规模自适应**）—— 代价 ∝ input，门槛就该跟着走。
pub(crate) fn rewrite_min_gain_pct(input_tokens: i64) -> usize {
    const BASE: usize = th::REWRITE_MIN_GAIN_PCT;
    const MAX: usize = th::REWRITE_MAX_GAIN_PCT;
    let extra = (input_tokens.max(0).saturating_sub(60_000) / 100_000) as usize * 7;
    (BASE + extra).min(MAX)
}

/// 装配期改写**省出多少字节才值得破前缀缓存**。
pub(crate) fn rewrite_min_saved_bytes(before_bytes: usize, input_tokens: i64) -> usize {
    before_bytes * rewrite_min_gain_pct(input_tokens) / 100
}

/// 治理层（部门）。**一层只管一件事**，层内按类型分派。
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dept {
    /// L0：单条回执入库时的瘦身。
    Intake,
    /// L1：本回合工作面。
    InTask,
    /// L2：旧回合退场。
    CrossTask,
    /// L3：全历史硬顶（最后防线）。
    HardCap,
}

/// 工具类别 —— 决定"这条回执还能不能再生、再生贵不贵"。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolKind {
    /// 探索型：read / search / list / find_files / audit / env / diagnose / db_query。
    Explore,
    /// 执行型：run / verify。动作已完成、产物在盘上 ⇒ 留在上下文里是过期副本 ⇒ 可压。
    Execute,
    /// 写入型：write / modify。文件已落盘 ⇒ 可压（但定位键必须留）。
    Write,
    /// 通讯型：web_fetch / spill 回读等。
    Fetch,
}

/// 工具名 → 类别。**判据唯一**（表驱动，不做字符串模糊匹配）。
pub(crate) fn kind_of_tool(name: &str) -> ToolKind {
    const EXPLORE: &[&str] = &[
        "read", "search", "list", "find_files", "audit", "env", "diagnose", "db_query",
    ];
    const EXECUTE: &[&str] = &["run", "verify"];
    const WRITE: &[&str] = &["write", "modify"];
    const FETCH: &[&str] = &["web_fetch", "fetch", "spill_read"];
    if EXPLORE.contains(&name) {
        ToolKind::Explore
    } else if EXECUTE.contains(&name) {
        ToolKind::Execute
    } else if WRITE.contains(&name) {
        ToolKind::Write
    } else if FETCH.contains(&name) {
        ToolKind::Fetch
    } else {
        // 未登记的工具：按"执行型"处理（保守 —— 压掉它等价于"重跑一次"，代价可控；
        ToolKind::Execute
    }
}

/// 「钥匙键」= 只有它能让人把那次调用**重新走一遍**，所以它的值**不跟着一起截短**
pub(crate) fn is_key_field(tool: &str, key: &str) -> bool {
    matches!(
        (tool, key),
        ("run", "command") | ("read", "paths") | ("read", "path") | ("write" | "modify", "file")
    )
}

/// 时机档 —— **时机的真实判据是"动作是否已结束"与"体积 vs 预算"，不是条数**。
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stage {
    /// T0 在飞：当前回合、最近窗口内 —— **绝不压**。
    InFlight,
    /// T1 完成即失效：动作已结束、产物已落盘 —— **可压**（最划算）。
    JustDone,
    /// T2 窗口内还要引用：滑出保留条数但仍在回合内 —— 压到"可回取"。
    InWindow,
    /// T3 跨任务：上一回合及更早 —— 压成**路标 + 指针**。
    CrossTask,
    Permanent,
}

/// 处置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    /// 原样保留（不可再生，或压了不划算）。
    Keep,
    /// 压成路标 + 回取钥匙（路径 / 原参数 / 行号）。
    Stub,
    /// 整段落盘 + 只留指针（大件的退场形态）。
    Spill,
}

/// 裁决：给定类型与时机，**该不该压**。
pub(crate) fn verdict(kind: ToolKind, stage: Stage, has_spill_ptr: bool) -> Action {
    // ① 永久层：任何时机都不压（不可再生）。
    if stage == Stage::Permanent {
        return Action::Keep;
    }
    // ② 在飞：当前工作面，绝不压。
    if stage == Stage::InFlight {
        return Action::Keep;
    }
    // ③ 已经落过盘的：直接压 —— 全文可回取，压掉不丢信息，是最划算的一类。
    if has_spill_ptr {
        return Action::Spill;
    }
    match (kind, stage) {
        // ④ 探索型：它的行号/命中/结构是后续编辑的依据。
        (ToolKind::Explore, Stage::InWindow) => Action::Keep,
        (ToolKind::Explore, Stage::CrossTask) => Action::Stub,
        (ToolKind::Explore, Stage::JustDone) => Action::Keep,
        // ⑤ 执行型 / 写入型 / 通讯型：动作已结束，产物在盘上 ⇒ 留的是过期副本。
        (_, Stage::JustDone) | (_, Stage::InWindow) | (_, Stage::CrossTask) => Action::Stub,
        // ⑥ Permanent / InFlight 上面已早退，这里不可达（留作穷尽匹配）。
        (_, Stage::Permanent) | (_, Stage::InFlight) => Action::Keep,
    }
}

/// 一条机制登记：**谁、在哪一层、管哪类内容的哪个时机**。
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Content {
    /// 工具回执（层内按 `ToolKind` 再分派）。
    ToolOutput,
    /// 工具调用参数。
    ToolArgs,
    /// 推理内容。
    Reasoning,
    /// 重复读去重（同一路径只留最后一次 full 读取）。
    ReadDedup,
    /// 附件（图片 / 文档）。
    Attachment,
    /// 正文摘要（**改长度**的那一步，只能排在落库之后）。
    BodySummary,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct Mechanism {
    /// 实现函数名（对得上代码才有意义）。
    pub(crate) name: &'static str,
    pub(crate) dept: Dept,
    /// 管的内容类别。
    pub(crate) content: Content,
    pub(crate) stage: Stage,
    /// 一句话：它凭什么动手。
    pub(crate) judge: &'static str,
}

/// **部门表**：现状登记。新增机制必须在此登记 —— 若与已有条目撞了 (dept 之外的 tool×stage)，
#[allow(dead_code)]
pub(crate) const MECHANISMS: &[Mechanism] = &[
    Mechanism {
        name: "maybe_spill",
        dept: Dept::Intake,
        content: Content::ToolOutput,
        stage: Stage::JustDone,
        judge: "单条回执超阈值 ⇒ 全文落盘 + 上下文只留「结构感知预览 + 回取指针」\
                （`mcp::spill::maybe_spill`，在**工具返回那一刻**执行）。阈值按工具分档：\
                通用 th::SPILL_THRESHOLD_CHARS(2,000) / read 全文 th::READ_SPILL_THRESHOLD_CHARS(16,000) / \
                read lines th::READ_LINES_SPILL_THRESHOLD_CHARS(40,000)；\
                `modify` 永不落盘（结果本来就小）；`read` 读自己落盘的 spill 文件时豁免\
                （防「落盘 → 回读 → 再落盘」的循环）。\
                （2026-09-15 由 `agent::history::slim_tool_output` 合并而来 —— 原先两层治理同一份数据，\
                代价是两张工具名单、两套阈值、两个落盘目录、两种回执标记。）",
    },
    Mechanism {
        name: "govern_in_task_outputs",
        dept: Dept::InTask,
        content: Content::ToolOutput,
        stage: Stage::InWindow,
        judge: "回合内输出总体积 > 预算（层内按 kind_of_tool 分派）",
    },
    Mechanism {
        name: "govern_reasoning",
        dept: Dept::InTask,
        content: Content::Reasoning,
        stage: Stage::InWindow,
        judge: "按字节预算保留最近若干条原文（th::INTASK_KEEP_REASONING_BYTES），其余摘成决策句",
    },
    Mechanism {
        name: "dedup_read_outputs",
        dept: Dept::InTask,
        content: Content::ReadDedup,
        stage: Stage::InWindow,
        judge: "同一路径的重复 read（full 模式为锚，更早的压成「已略」）—— 须过收益门槛",
    },
    Mechanism {
        name: "退场策略(stub_span/roll_span)",
        dept: Dept::CrossTask,
        content: Content::ToolOutput,
        stage: Stage::CrossTask,
        judge: "年龄 > RETIRE_FRESH_ROUNDS（硬）或 引用断续（软）—— 须过收益门槛",
    },
    Mechanism {
        name: "stub_old_call_args",
        dept: Dept::CrossTask,
        content: Content::ToolArgs,
        stage: Stage::JustDone,
        judge: "只需定位键，正文对后续轮次无用（≥ th::ARG_MIN_BYTES）",
    },
    Mechanism {
        name: "archive_hard_cap",
        dept: Dept::HardCap,
        content: Content::ToolOutput,
        stage: Stage::CrossTask,
        judge: "全历史体积 > th::HARD_CAP_BYTES 或 token 预算（须过收益门槛）",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    /// **不变量：一件事只有一个部门。**
    #[test]
    fn governance_has_no_duplicate_owners() {
        let mut seen: Vec<(Dept, Content, Stage)> = Vec::new();
        for m in MECHANISMS {
            let key = (m.dept, m.content, m.stage);
            assert!(
                !seen.contains(&key),
                "两个部门管一件事：{:?} 已被占用，又登记了 {}（判据：{}）",
                key,
                m.name,
                m.judge
            );
            seen.push(key);
        }
    }

    /// 阈值必须是**递进的链**（下一层入口 ≈ 上一层出口），不是各自为政的数。
    #[test]
    fn thresholds_form_a_chain() {
        let worst_total = th::WATERMARK_HARD_CAP_BYTES * 6 / 10;
        assert!(
            th::INTASK_MIN_ITEM_BYTES < worst_total,
            "单条起切线必须小于总量上限，否则永远压不动"
        );
        assert!(
            worst_total < th::HARD_CAP_BYTES,
            "L1 回合内({}) 必须小于 L3 硬顶({})，否则硬顶先触发、L1 轮不到动手",
            worst_total,
            th::HARD_CAP_BYTES
        );
        assert!(
            worst_total < th::HARD_CAP_BYTES * 6 / 10,
            "硬顶的**目标**(= 本值×0.6 = {}) 必须大于 L1 阈值({})，\
             否则硬顶一压到底，L1 的按类型分派永远不进场",
            th::HARD_CAP_BYTES * 6 / 10,
            worst_total
        );
    }

    /// **动态阈值必须锚住稳态、单调反比、两端夹紧**。
    #[test]
    fn dynamic_spill_threshold_is_anchored_monotone_and_clamped() {
        let anchor = super::window_safe_tokens() as u64;
        // ① 锚点恒等：压力恰在水位线上 ⇒ **逐字节等于**原常量（零回归的硬保证）
        assert_eq!(
            th::spill_threshold_for(anchor),
            th::SPILL_THRESHOLD_CHARS,
            "压力 = 稳态水位时动态阈值必须等于原常量（{anchor} token ⇒ {}），\
             否则所谓动态化就是偷偷调参",
            th::SPILL_THRESHOLD_CHARS
        );
        // ② 反比单调：历史越大，阈值越小（越该落盘）
        let short = th::spill_threshold_for(anchor / 2);
        let steady = th::spill_threshold_for(anchor);
        let long = th::spill_threshold_for(anchor * 4);
        assert!(
            short >= steady && steady >= long,
            "阈值必须随历史体量单调不增：短({short}) / 稳态({steady}) / 长({long})"
        );
        assert!(
            short > long,
            "短任务({short}) 与长任务({long}) 必须真的拉开差距 —— \
             若两端被夹到同一值，动态化等于没做"
        );
        // ③ 两端夹紧：任何输入（含 0 与极大值）都不得越界
        for input in [0u64, 1, anchor / 10, anchor, anchor * 4, anchor * 100, u64::MAX] {
            let v = th::spill_threshold_for(input);
            assert!(
                (th::MIN_DYNAMIC_SPILL_CHARS..=th::MAX_DYNAMIC_SPILL_CHARS).contains(&v),
                "input={input} 时阈值 {v} 越出 [{}, {}]",
                th::MIN_DYNAMIC_SPILL_CHARS,
                th::MAX_DYNAMIC_SPILL_CHARS
            );
        }
        // 夹逼边界本身也要与**链上邻居**相容（不只是自洽）
        assert!(
            th::MAX_DYNAMIC_SPILL_CHARS < th::READ_SPILL_THRESHOLD_CHARS,
            "动态上限({}) 必须低于 read 全文线({})，否则动态值会破坏链序",
            th::MAX_DYNAMIC_SPILL_CHARS,
            th::READ_SPILL_THRESHOLD_CHARS
        );
        let preview = th::SPILL_PREVIEW_HEAD_CHARS + th::SPILL_PREVIEW_TAIL_CHARS;
        assert!(
            th::MIN_DYNAMIC_SPILL_CHARS > preview,
            "动态下限({}) 必须大于预览({})，否则落盘省不出体积",
            th::MIN_DYNAMIC_SPILL_CHARS,
            preview
        );
    }

    /// **spill 的门槛必须成链**。
    #[test]
    fn spill_thresholds_form_a_chain() {
        // read 全文对修改有素材价值 ⇒ 门槛应高于通用线（回读代价大，少落一次省一次往返）
        assert!(
            th::READ_SPILL_THRESHOLD_CHARS >= th::SPILL_THRESHOLD_CHARS,
            "read 全文 spill 线({}) 不应低于通用线({})",
            th::READ_SPILL_THRESHOLD_CHARS,
            th::SPILL_THRESHOLD_CHARS
        );
        // ── 链序：通用(2,000) < read全文(16,000) < lines精读(40,000) ─────────
        assert!(
            th::READ_SPILL_THRESHOLD_CHARS < th::READ_LINES_SPILL_THRESHOLD_CHARS,
            "read lines 精读线({}) 必须**高于** read 全文线({})——\
             lines 的体量由模型参数决定（它点了行段），入库层不该抢在 L1/L2 之前\
             把模型点名要的东西拿走",
            th::READ_LINES_SPILL_THRESHOLD_CHARS,
            th::READ_SPILL_THRESHOLD_CHARS
        );
        assert!(
            th::SPILL_THRESHOLD_CHARS < th::READ_LINES_SPILL_THRESHOLD_CHARS,
            "通用线({}) 必须低于 lines 精读线({})——lines 是最不该在入库层落盘的一档",
            th::SPILL_THRESHOLD_CHARS,
            th::READ_LINES_SPILL_THRESHOLD_CHARS
        );
        // 预览必须比触发线小得多，否则"落盘"不省体积
        let preview = th::SPILL_PREVIEW_HEAD_CHARS + th::SPILL_PREVIEW_TAIL_CHARS;
        assert!(
            preview * 2 < th::SPILL_THRESHOLD_CHARS,
            "预览({} = head {} + tail {}) 相对 spill 线({}) 太大 ⇒ 落盘省不出体积",
            preview,
            th::SPILL_PREVIEW_HEAD_CHARS,
            th::SPILL_PREVIEW_TAIL_CHARS,
            th::SPILL_THRESHOLD_CHARS
        );
    }

    /// **预算线的压缩目标必须落在硬顶之下** —— 否则这条线触发也白压。
    #[test]
    fn token_budget_must_bite_before_hard_cap() {
        let budget = crate::config::settings::DEF_TOKEN_BUDGET as usize;
        let budget_target = budget * 6 / 10 * th::BYTES_PER_TOKEN_EST;
        assert!(
            budget_target < th::HARD_CAP_BYTES,
            "预算({budget} tok) × 0.6 × 系数({}) = {budget_target} 字节，必须小于体积硬顶({})；\
             否则预算线触发后仍压不到硬顶之下，等于白压",
            th::BYTES_PER_TOKEN_EST,
            th::HARD_CAP_BYTES,
        );
    }

    /// 裁决表：不可再生的内容在**任何时机**都不许被压。
    #[test]
    fn permanent_content_is_never_compressed() {
        for kind in [ToolKind::Explore, ToolKind::Execute, ToolKind::Write, ToolKind::Fetch] {
            assert_eq!(
                verdict(kind, Stage::Permanent, true),
                Action::Keep,
                "永久层（用户原话/结论/规矩）不得被压 —— 即使它已落盘"
            );
            assert_eq!(
                verdict(kind, Stage::InFlight, false),
                Action::Keep,
                "在飞的工作面不得被压"
            );
        }
    }

    /// 带 spill 指针的一律可压（全文可回取，压掉不丢信息）—— 最划算的一类。
    #[test]
    fn spilled_payload_is_always_compressible() {
        assert_eq!(verdict(ToolKind::Explore, Stage::InWindow, true), Action::Spill);
        assert_eq!(verdict(ToolKind::Write, Stage::CrossTask, true), Action::Spill);
    }

    /// 探索型的窗口内内容必须留 —— 压掉它不是省字节，是逼模型重读
    #[test]
    fn explore_in_window_is_kept() {
        assert_eq!(verdict(ToolKind::Explore, Stage::InWindow, false), Action::Keep);
        assert_eq!(verdict(ToolKind::Execute, Stage::InWindow, false), Action::Stub);
    }

    /// 未登记的工具按"执行型"兜底（保守：压掉 = 可重跑；误判成不压则上下文无上限增长）。
    #[test]
    fn unknown_tool_falls_back_to_execute() {
        assert_eq!(kind_of_tool("mystery_tool"), ToolKind::Execute);
        assert_eq!(kind_of_tool("read"), ToolKind::Explore);
        assert_eq!(kind_of_tool("run"), ToolKind::Execute);
        assert_eq!(kind_of_tool("write"), ToolKind::Write);
    }

    /// 热会话 + 未越线 ⇒ **一处都不动**（这是省钱的主开关）。
    #[test]
    fn rewrite_gate_blocks_hot_session() {
        assert!(!assembly_rewrite_allowed(12, 44_000), "热（idle=12s）不该动历史");
        assert!(
            !assembly_rewrite_allowed(th::ASSEMBLY_IDLE_SECONDS - 1, 44_000),
            "边界：差 1 秒仍算热"
        );
    }

    /// 冷（免费时刻）或越发送安全线（硬约束）⇒ 允许压。
    #[test]
    fn rewrite_gate_opens_when_cold_or_over_window() {
        assert!(
            assembly_rewrite_allowed(th::ASSEMBLY_IDLE_SECONDS, 44_000),
            "到点即冷：缓存大概率失效，此刻改写不多花钱"
        );
        assert!(
            assembly_rewrite_allowed(5, send_safe_tokens() + 1),
            "热也得压：越发送安全线不压，下一轮 `enforce_token_ceiling` 要从最老处硬删"
        );
        assert!(cache_is_cold(i64::MAX), "本会话还没调用过 ⇒ 无前缀可破 ⇒ 冷");
    }

    /// 硬约束必须比**发送安全线**，不能比水位线 —— 比水位线会恒真（水位线是"压完的目标"）。
    #[test]
    fn hard_gate_uses_send_safe_line_not_watermark() {
        assert!(
            window_safe_tokens() < 44_000,
            "前提：水位线 ≤ 27K token（hard cap 换算）< 典型 input 44K ⇒ 拿它当硬线必然恒真"
        );
        assert!(
            send_safe_tokens() > window_safe_tokens(),
            "发送安全线（绝不能超窗口）必须高于水位线（压到多小）"
        );
        // 同一份 input：热时不动、冷时才动 —— 判据只由 idle 与安全线决定。
        assert!(!assembly_rewrite_allowed(3, 44_000));
        assert!(assembly_rewrite_allowed(th::ASSEMBLY_IDLE_SECONDS + 1, 44_000));
    }
}
