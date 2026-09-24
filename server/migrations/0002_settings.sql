-- 运行时设置表（环境变量为启动默认值，本表为运行期覆盖项）
CREATE TABLE IF NOT EXISTS settings (
    key         TEXT PRIMARY KEY,   -- llm_mode | api_key | base_url | model | default_system_prompt
    value       TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);
