-- 工具护栏事件审计（2026-08-19）：把「模型在工具使用环节踩坑、后端是否兜住」落库，
-- 让「后端兜底达成度」可机读判分，与终态 verifier（正确性）形成双指标。
--
-- 设计边界：
--  * 仅记录「后端护栏产生 error_code 的事件」（后端已拦并给引导，期望模型恢复）。
--  * 「后端漏检」类（如 write content 里的 TODO/your code here 占位桩，护栏不拦、不报错）
--    不产生 error_code，故不在此表；由终态 verifier 抓取（属正确性指标，不属兜底指标）。
--  * recovered：run 收敛后由 finalize_guard_events 回填——
--    该 step_id 在 tool_calls 中最终有 status='success' 记 1（已恢复），否则记 0（未恢复）。
--  * fatal：0=后端护栏已拦截并引导(期望恢复)；1=工具定义类问题(后端未兜住，需模型自纠)。

CREATE TABLE IF NOT EXISTS tool_guard_events (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id     TEXT NOT NULL,                 -- 对应 session_id（bench 一 run 一 session）
    step_id    TEXT,                          -- 关联 plan step（同 step 重试多次计多条）
    tool       TEXT NOT NULL,                -- 工具名（edit/read/write/run/...）
    error_code TEXT,                         -- 领域错误码（RELATIVE_PATH/OLD_TEXT_MISMATCH/...）
    fatal      INTEGER NOT NULL DEFAULT 0,   -- 0=后端护栏已拦 1=工具定义类缺口(后端未兜)
    recovered  INTEGER,                       -- NULL=未定 0=未恢复 1=已恢复
    detail     TEXT,                         -- 错误摘要（截断）
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_guard_run  ON tool_guard_events(run_id);
CREATE INDEX IF NOT EXISTS idx_guard_step ON tool_guard_events(run_id, step_id);
