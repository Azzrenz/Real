-- 0014: 会话级模型选择（模型绑定到会话，而非全局）
--
-- 此前模型只存在全局 settings.model，切换一次会让所有会话跟着转。
-- 本列留 NULL = 跟随全局默认（settings.model）：新建会话自动用它，
-- 只有用户在某个会话里显式选过，才在本会话落库。
ALTER TABLE sessions ADD COLUMN model TEXT;
