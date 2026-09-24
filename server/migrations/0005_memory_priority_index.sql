-- Real 记忆按优先级检索（P0c：user_fact=90 排最前，conclusion=60，modification=40）
-- 召回排序从 created_at 改为 priority DESC 优先，配索引避免全表扫描
CREATE INDEX IF NOT EXISTS idx_memories_ws_priority ON memories(workspace, priority DESC, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_memories_type_workspace ON memories(workspace, record_type, created_at DESC);
