-- E1：memories 表规范化（对齐《记忆与上下文管理架构规范 v1.0》§2.1）
-- ① 清理同 workspace+key 的重复（保留最新一条，防唯一索引建立失败）
-- ② UNIQUE(workspace, key)：防膨胀铁律（upsert 依赖）
-- ③ scope 列：隔离维度（workspace=跨会话共享 / session=本会话隔离 / user=全局）

-- ① 清理重复：同 workspace+key 只保留 id 最大（最新）的一条
DELETE FROM memories WHERE id NOT IN (
    SELECT MAX(id) FROM memories GROUP BY workspace, key
);

-- ② 唯一索引（规范 §2.1 uq_memories_ws_key）
CREATE UNIQUE INDEX IF NOT EXISTS uq_memories_ws_key ON memories(workspace, key);

-- ③ scope 列（默认 session）
ALTER TABLE memories ADD COLUMN scope TEXT NOT NULL DEFAULT 'session';

-- ④ 回填 scope：事实型/结论型/锚定型默认 workspace（跨会话共享注入，ADR-009 隔离铁律）
UPDATE memories SET scope = 'workspace'
 WHERE record_type IN ('user_fact','user_fact_neg','conclusion','modification','path_anchor','digest','files_read','compact_snapshot');
