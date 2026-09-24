-- Real 多轮锚定（任务管理 + 断点续跑 + 快照）
-- 目的：跨轮次"永远锚得住"——上一轮审查 D:\A，这一轮修复 D:\A，第三轮续接，
-- workspace/断点/已读清单全部持久化，模型不重探索、不重读、不丢上下文。

-- ① sessions 表加 workspace 列：任务容器级锚定
--    创建任务时记录；每轮入口从 DB 恢复（比"从输入猜"可靠）
ALTER TABLE sessions ADD COLUMN workspace TEXT NOT NULL DEFAULT '';

-- ② checkpoints 表：中断断点
--    保存时机：LLM 失败/用户取消/轮次上限；恢复：输入含"继续/接着/续接/resume"
CREATE TABLE IF NOT EXISTS checkpoints (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id    TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    round         INTEGER NOT NULL DEFAULT 0,      -- 中断时的轮次/步数
    summary       TEXT NOT NULL DEFAULT '',        -- 进度摘要（已做/卡住/下一步）
    plan_exists   INTEGER NOT NULL DEFAULT 0,      -- 是否有可恢复计划
    files_read    TEXT NOT NULL DEFAULT '',        -- 已读文件列表（逗号分隔，防重读）
    created_at    TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_checkpoints_session ON checkpoints(session_id);
CREATE UNIQUE INDEX IF NOT EXISTS uq_checkpoints_session ON checkpoints(session_id);
