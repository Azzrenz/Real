-- 当轮帧存档：让「下一轮重建」能逐字节重现上一轮真正发出去的帧
--
-- 为什么单独一张表，而不是写进 messages：
--   · messages 的 user 行是「回合起点」——跨回合归档、轮日志 seq、前端用户气泡全按它划界，
--     多插一条会把一个回合切成两半（turn_logs.seq 直接对不上）；
--   · 帧必须按**锚点消息 id** 精确插回原位。插错一格，其后所有字节作废（缓存是前缀匹配）。
--
-- 生命周期：每轮一行（约 1–2KB），随会话增长；锚点消息被窗口裁掉后自然不再重放。
-- 不做外键级联：锚点不存在时读侧直接跳过，删会话时随 session 一并清即可。
CREATE TABLE IF NOT EXISTS round_frames (
    session_id        TEXT NOT NULL,   -- 所属会话
    anchor_message_id TEXT NOT NULL,   -- 该帧紧跟其后的那条 messages.id（当轮用户原话）
    item_json         TEXT NOT NULL,   -- 帧作为 InputItem 的序列化（重建时原样反序列化）
    created_at        TEXT NOT NULL,   -- RFC3339
    PRIMARY KEY (session_id, anchor_message_id)
);
