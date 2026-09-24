-- 0026 任务中插话机制：插话队列（持久化，忙时不拒收，轮边界原子消费）
CREATE TABLE IF NOT EXISTS interjections (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    text TEXT NOT NULL,
    mode TEXT NOT NULL DEFAULT 'append',
    status TEXT NOT NULL DEFAULT 'pending',
    created_at TEXT NOT NULL,
    consumed_at TEXT,
    consumed_round INTEGER
);
CREATE INDEX IF NOT EXISTS idx_interjections_session_status
    ON interjections(session_id, status);
