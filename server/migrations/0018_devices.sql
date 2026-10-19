-- 设备鉴权：远程设备（手机）经配对码换取的长期 token（**只存 SHA-256 哈希**）
--
-- 动机：现有安全模型只有「只绑回环」这一条（物理隔离）——代码里连"要不要鉴权"的骨架都没有。
-- 要支持手机远程访问，必须给远程调用一个可撤销的身份。
-- 边界（与 routes/auth_guard.rs 配套）：
--   回环来源（127.0.0.1 / ::1）免鉴权 → 桌面 Tauri 壳零变化；
--   非回环来源必须持本表登记的有效 token（HttpOnly Cookie 或 Authorization: Bearer）。
CREATE TABLE IF NOT EXISTS devices (
    id          TEXT    PRIMARY KEY,          -- 设备 id（uuid v4）
    name        TEXT    NOT NULL,             -- 人读设备名（配对时由调用方提供，已清洗）
    token_hash  TEXT    NOT NULL UNIQUE,      -- token 的 SHA-256 十六进制摘要（**明文不落库**）
    created_at  TEXT    NOT NULL,             -- RFC3339
    last_seen   INTEGER NOT NULL              -- 最近使用时刻（epoch 毫秒；整数便于节流比较）
);
