-- 会话退场与摘要沉淀（2026-09-16 用户定调"缓慢退出清场 + 形成摘要"）
--
-- 背景：events 表只增不减。实测（2026-09-16）255951 行里 reasoning 占 215363 行（84%），
-- 平均 payload 仅 33 字节 —— 每个流式 token 一条 INSERT。real.db 涨到 65.8MB，
-- 且没有任何回收机制（只有 confirm 瞬时事件有清理）。
--
-- 抄大厂做法（Claude Code / Codex 三家一致）：
--   ① 原文一律落本地硬盘（不做在库里，不占热路径）
--   ② 摘要是索引不是全文，带指针，要细节去 read 包
--   ③ 清扫晚于摘要 —— 摘完还要等，不立即删
--
-- 本表是退场流水线的工作台账：一个会话一行，记录它走到哪一档。
-- 档位由「最后一次 llm.usage 之后的静默时长」驱动：
--   warm_at  静默满 1 天 → 做会话级摘要 + 回合包落盘（不动 events）
--   cold_at  静默满 7 天 → 摘要已自证够用，才清掉原始事件行
--
-- 「摘完还要等」的判据落在 summarized_at 与 purged_at 两个时间戳上，
-- 而不是靠调用方记住状态 —— 崩溃重启后按台账续跑。

CREATE TABLE IF NOT EXISTS session_retirement (
    session_id     TEXT PRIMARY KEY,
    -- 最后一次活动时间（UTC，来自 events 里最后一条 llm.usage）。
    -- 为什么在 SQL 里取：created_at 存的是 datetime('now') 即 UTC，
    -- 取回来跟本地时间比会差 8 小时 ⇒ 空档恒为负 ⇒ 闸门恒关（同 session_idle_seconds 的坑）。
    last_active_at TEXT NOT NULL,
    -- 温档：会话级摘要已生成的时间。空 = 还没摘。
    summarized_at  TEXT,
    -- 会话级摘要正文（一页纸）。摘要成功后写入，是退场后的唯一事实源。
    digest_json    TEXT,
    -- 冷档：原始事件行已清的时间。空 = 还没清。
    purged_at      TEXT,
    -- 清掉的事件行数与释放的估算字节（供观测，不参与逻辑）
    purged_rows    INTEGER NOT NULL DEFAULT 0,
    purged_bytes   INTEGER NOT NULL DEFAULT 0,
    -- 失败留痕：摘要是 LLM 调用，会失败。失败不阻断，下轮重试。
    last_error     TEXT
);
CREATE INDEX IF NOT EXISTS idx_retirement_summarized ON session_retirement(summarized_at);
CREATE INDEX IF NOT EXISTS idx_retirement_purged ON session_retirement(purged_at);
