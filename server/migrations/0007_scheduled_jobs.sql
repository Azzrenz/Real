-- 定时任务调度（Cron / Heartbeat）数据模型
-- 三件套闭环的「定时触发」缺口补齐：自然语言或标准 cron → 到点自动跑任务（复用 run_agent）。

CREATE TABLE IF NOT EXISTS scheduled_jobs (
    id            TEXT PRIMARY KEY,
    name          TEXT NOT NULL,
    prompt        TEXT NOT NULL,                -- 到点自动执行的任务指令
    cron_expr     TEXT NOT NULL,                -- 标准 5 字段 cron（分 时 日 月 周）
    natural_lang  TEXT NOT NULL DEFAULT '',     -- 原始自然语言描述（展示/回溯用）
    workspace     TEXT,                         -- 可选工作目录（提示 run_agent 锚定）
    enabled       INTEGER NOT NULL DEFAULT 1,   -- 1=启用 0=停用
    last_run_at   TEXT,                         -- 上次触发时间（RFC3339）
    last_status   TEXT,                         -- running|ok|error
    last_result   TEXT,                         -- 上次执行结果摘要（截断）
    next_run_at   TEXT,                         -- 下次触发时间（RFC3339），空则需补全
    created_at    TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_scheduled_jobs_next ON scheduled_jobs(enabled, next_run_at);
