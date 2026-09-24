//! 全局应用状态：连接池 + LLM + 工具注册表 + SSE Hub + 取消令牌 + 运行时设置

use crate::config::Config;
use crate::mcp::registry::Registry;
use crate::model::llm::Llm;
use crate::config::settings::SettingsRef;
use crate::sse::EventHub;
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::atomic::AtomicI64;
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

/// 自改进步骤序号（tool 事件 step_id 用，保证前端 append 顺序唯一）
pub static STEP_SEQ: AtomicI64 = AtomicI64::new(1);

#[derive(Clone)]
pub struct RunHandle {
    pub run_id: String,
    pub cancel: CancellationToken,
}

/// 生成 run 身份：8 位十六进制（会话内唯一足够——同会话并发 run 只有个位数）
pub fn new_run_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..8].to_string()
}

#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<Config>,
    pub pool: SqlitePool,
    pub llm: Arc<dyn Llm>,
    pub settings: SettingsRef,
    pub registry: Arc<Registry>,
    pub hub: EventHub,
    /// session_id → 当前活跃 run（run_id + 取消令牌）。用户取消/删除会话时触发（坑位 C7）
    pub cancels: Arc<Mutex<HashMap<String, RunHandle>>>,
    /// 确认门 v2（重建）：危险操作执行前请求用户确认的挂起表。
    pub confirm_gate: Arc<crate::confirm::ConfirmGate>,
}

impl AppState {
    /// 本会话实际生效的模型：会话选定值优先，未选过才回退全局默认。
    pub async fn effective_model(&self, session_id: &str) -> String {
        match crate::db::repos::get_session_model(&self.pool, session_id).await {
            Ok(Some(m)) => m,
            _ => self.settings.read().await.model.clone(),
        }
    }

    /// 开启一次 run：生成身份 + 登记取消令牌。
    pub fn open_run(&self, session_id: &str) -> RunHandle {
        let handle = RunHandle {
            run_id: new_run_id(),
            cancel: CancellationToken::new(),
        };
        {
            let mut map = self.cancels.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(prev) = map.get(session_id) {
                tracing::warn!(
                    session = session_id,
                    prev_run = %prev.run_id,
                    new_run = %handle.run_id,
                    "新 run 顶替未收尾的旧 run（旧轮事件将按 run_id 隔离，不再落到新轮）"
                );
            }
            map.insert(session_id.to_string(), handle.clone());
        }
        handle
    }

    /// 取消当前活跃 run；返回被取消的 run_id（无活跃 run = None）
    pub async fn cancel_session(&self, session_id: &str) -> Option<String> {
        let slot = self
            .cancels
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(session_id)
            .cloned();
        let active = slot?;
        active.cancel.cancel();
        // 状态置 cancelled：会话层面"已停止"（侧栏/会话列表据此显示）；
        let _ = crate::db::repos::update_session_status(&self.pool, session_id, "cancelled").await;
        // 停止回收未消费插话：pending 插话标 cancelled + 对应消息作废，
        match crate::db::repos::cancel_pending_interjections(&self.pool, session_id).await {
            Ok(n) if n > 0 => {
                tracing::info!(session = session_id, cancelled = n, "停止：已回收未消费插话");
            }
            _ => {}
        }
        Some(active.run_id)
    }

    /// 只清自己的坑（会话已被更新的 run 接管时不动别人的令牌）
    pub fn remove_cancel_if(&self, session_id: &str, run_id: &str) {
        let mut map = self.cancels.lock().unwrap_or_else(|e| e.into_inner());
        if map.get(session_id).map(|h| h.run_id.as_str()) == Some(run_id) {
            map.remove(session_id);
        }
    }

    /// 无条件清坑：会话已删除（整条会话的生命周期结束，坑位无意义）
    pub fn forget_run(&self, session_id: &str) {
        self.cancels
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(session_id);
    }
}

/// RAII 取消令牌守卫：run_agent 开头创建，**无论正常返回还是 Err 提前退出都自动清理**，
pub struct CancelGuard {
    state: AppState,
    session_id: String,
    run_id: String,
    active: bool,
}

impl CancelGuard {
    pub fn new(state: &AppState, session_id: &str, run_id: &str) -> Self {
        Self {
            state: state.clone(),
            session_id: session_id.to_string(),
            run_id: run_id.to_string(),
            active: true,
        }
    }
}

impl Drop for CancelGuard {
    fn drop(&mut self) {
        if self.active {
            self.state.remove_cancel_if(&self.session_id, &self.run_id);
        }
    }
}
