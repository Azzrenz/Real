-- 工具结果缓存持久化（跨重启恢复）
--
-- 动机：进程内 TOOL_CACHE 是纯内存，重启即全丢，重启后同参调用被迫重跑。
-- 落盘后由 registry::warm_tool_cache 在启动时灌回内存。
--
-- 隔离性保证：key 形如 `{session_id}|{tool}|{args}`（见 registry.rs tool_cache_key），
-- **天然含 session 前缀** ⇒ 原样落盘即按会话隔离，不会跨会话串结果。
-- （教训：用空 session 命名空间会落成全局，导致残留永不清理。）
CREATE TABLE IF NOT EXISTS tool_cache (
    key        TEXT PRIMARY KEY,
    value      TEXT    NOT NULL,   -- ToolEnvelope 的 JSON
    created_at INTEGER NOT NULL    -- 落盘时刻 epoch 毫秒
);

CREATE INDEX IF NOT EXISTS idx_tool_cache_created ON tool_cache(created_at);
