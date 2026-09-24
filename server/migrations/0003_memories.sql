-- Real 多轮任务记忆（跨会话记忆 + 路径锚定）
-- 用途：上一轮任务修好的路径/结论/修改，下一轮"继续修"时注入 Planner，避免重新探索、路径丢失

CREATE TABLE IF NOT EXISTS memories (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    workspace     TEXT NOT NULL DEFAULT '',      -- 项目路径锚定（如 D:\AI Doc\node-v26.5.0-win-x64）
    session_id    TEXT NOT NULL,                 -- 产生该记忆的会话
    key           TEXT NOT NULL,                 -- 记忆键（path_anchor/conclusion/modification/error 等语义）
    value         TEXT NOT NULL,                 -- 记忆内容
    record_type   TEXT NOT NULL DEFAULT 'note',  -- path_anchor|conclusion|modification|error|tool_result|note
    priority      INTEGER NOT NULL DEFAULT 5,    -- 注入优先级（高优先先展示）
    created_at    TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_memories_workspace ON memories(workspace, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_memories_recent   ON memories(created_at DESC);
