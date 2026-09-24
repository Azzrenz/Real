-- Real 会话管理数据模型（ADR-004）
-- 全部表强制携带 session_id，行级隔离第一道锁

CREATE TABLE IF NOT EXISTS sessions (
    id            TEXT PRIMARY KEY,             -- session_id (uuid)
    title         TEXT NOT NULL DEFAULT '新任务',
    status        TEXT NOT NULL DEFAULT 'idle', -- idle|planning|executing|solving|reflecting|done|error|cancelled
    system_prompt TEXT NOT NULL DEFAULT '',
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS messages (
    id            TEXT PRIMARY KEY,
    session_id    TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    role          TEXT NOT NULL,                -- user|assistant|tool
    content       TEXT NOT NULL DEFAULT '',     -- 展示用文本
    item_json     TEXT,                         -- Responses API input item 原样（历史重建零失真）
    created_at    TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id, created_at);

CREATE TABLE IF NOT EXISTS tool_calls (
    id            TEXT PRIMARY KEY,             -- tool_call_id (call_xxx)
    session_id    TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    plan_id       TEXT,
    step_id       TEXT,                         -- #E1
    name          TEXT NOT NULL,
    arguments     TEXT NOT NULL,                -- JSON 字符串
    result_json   TEXT,                         -- ToolEnvelope 序列化
    status        TEXT NOT NULL,                -- success|error|timeout|pending
    duration_ms   INTEGER NOT NULL DEFAULT 0,
    created_at    TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_tool_calls_session ON tool_calls(session_id);

CREATE TABLE IF NOT EXISTS plans (
    id            TEXT PRIMARY KEY,             -- plan_id
    session_id    TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    objective     TEXT NOT NULL,
    steps_json    TEXT NOT NULL,                -- ReWOOPlan 序列化（含 #E 占位符）
    status        TEXT NOT NULL,                -- planned|executing|done|failed
    attempt       INTEGER NOT NULL DEFAULT 1,   -- 反思重试轮次
    created_at    TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_plans_session ON plans(session_id);

CREATE TABLE IF NOT EXISTS events (
    id            INTEGER PRIMARY KEY AUTOINCREMENT, -- 天然递增序列，SSE 回放游标
    session_id    TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    kind          TEXT NOT NULL,                -- run.started|plan.created|tool.started|tool.result|answer|reflect|run.completed|error
    payload_json  TEXT NOT NULL DEFAULT '{}',
    created_at    TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_events_session ON events(session_id, id);
