-- 环境状态事实账（2026-09-09）：端口监听/进程在跑等"已确认事实"持久化——
-- 防"已确认三遍还重探"：重启后 activate 注入本表，模型直接使用勿重复验证。
CREATE TABLE IF NOT EXISTS env_probes (
  session_id TEXT NOT NULL,
  pkey TEXT NOT NULL,
  value TEXT NOT NULL,
  round INTEGER NOT NULL,
  PRIMARY KEY (session_id, pkey)
);
