-- 0012: interjections ↔ messages 关联（cancel 时按 message_id 作废未消费插话的消息）
ALTER TABLE interjections ADD COLUMN message_id TEXT;
