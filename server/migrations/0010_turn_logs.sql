-- 轮任务日志与关键词激活索引（2026-09-02 用户定调"记忆部门化"）：
-- 后端统一管理——任务收敛时后端自动写轮日志（结论要点+激活关键词），
-- 下一任务启动时后端按新输入对历史关键词打分，命中的任务要点自动回灌注入。
-- 模型全程不自取、不自调——喂与回灌都是后端部门的行为。

CREATE TABLE IF NOT EXISTS turn_logs (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id    TEXT NOT NULL,               -- 产生该轮的会话
    seq           INTEGER NOT NULL,            -- 会话内任务序号（1 起）
    user_input    TEXT NOT NULL,               -- 用户原话（截断 500 字）
    answer_digest TEXT NOT NULL DEFAULT '',    -- 结论要点（从最终回答结构化抽取，≤800 字）
    keywords      TEXT NOT NULL DEFAULT '[]',  -- 激活关键词 JSON 数组（用户原话词元 + 改动文件名）
    files         TEXT NOT NULL DEFAULT '[]',  -- 改动文件 JSON 数组
    created_at    TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_turn_logs_session ON turn_logs(session_id, seq);
