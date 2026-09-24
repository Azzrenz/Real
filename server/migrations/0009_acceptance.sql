-- 外部验收配置（2026-08-27）：Fix 任务收敛的硬靶子。
-- acceptance_cmd/cwd 在会话创建时由调用方（bench/harness/操作者）配置；
-- 配置后该会话 Fix 任务的 done 必须 = 验收命令 exit 0（后端确定性执行），
-- 或模型显式声明「无法完成修复」（complete 打 fix_landed=false）。
ALTER TABLE sessions ADD COLUMN acceptance_cmd TEXT;
ALTER TABLE sessions ADD COLUMN acceptance_cwd TEXT;
