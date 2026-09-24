//! Real Agent 后端入口

mod agent;
mod backbone;
mod config;
mod confirm;
mod cost;
mod db;
mod domains;
mod error;
mod facts;
mod mcp;
mod model;
mod path;
mod pricing;
mod routes;
mod scheduler;
mod sse;
mod state;
mod tools;
mod wt;

use crate::config::Config;
use crate::mcp::client::McpServerConfigRef;
use crate::state::AppState;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ---- 数据根（一次性迁移旧散落点 → 新结构；先于配置/DB 连接） ----
    path::data_root::migrate_legacy();
    // 常用脚本库播种（幂等）：把"高频脚本"从"每轮现写"变成"直接调用"——
    path::data_root::seed_scripts();
    // 厂商档案说明播种（幂等）：告诉用户"接入新公司 = 往 providers/ 丢一个 JSON"，
    path::data_root::seed_providers_readme();

    // ---- 配置（fail-fast） ----
    let cfg = Config::from_env()?;

    // ---- 日志 ----
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,real_server=debug")),
        )
        .with_target(false)
        .init();
    tracing::info!(
        mode = ?cfg.llm_mode,
        model = %cfg.llm_model,
        // 版本名（DeepSeek V4.1 Flash）——日志里只打 API 名会让人看不出实际跑的是哪版模型
        model_label = %model::catalog::display_name_of(&cfg.llm_model),
        "Real Server 启动"
    );
    // 模型目录自检：有条目没登记版本名就告警（界面会退回显示 API 名，用户看不出是哪版）
    model::catalog::warn_on_missing_labels();
    tracing::info!(
        reasoning_placeholder = agent::history::reasoning_placeholder_enabled(),
        drop_prev_tool_outputs = agent::history::drop_prev_tool_outputs_enabled(),
        // 历史窗口低线——**必须走 `history_window_low()` 同源取值**。
        history_window_low = agent::history::history_window_low(),
        cache_probe = std::env::var("REAL_CACHE_DEBUG").unwrap_or_else(|_| "on".into()),
        "实验开关状态"
    );

    // ---- 数据库 ----
    let pool = db::init_pool(&cfg).await?;
    // 清理 confirm 瞬时事件（防回放触发遮罩）
    match db::repos::purge_confirm_events(&pool).await {
        Ok(n) => {
            if n > 0 {
                tracing::info!(cleaned = n, "已清理历史 confirm 事件");
            }
        }
        Err(e) => tracing::warn!(error = %e, "清理 confirm 事件失败（不影响启动）"),
    }
    // 启动自愈：残留运行态会话（崩溃/强杀残留）→ 重置 idle，
    match db::repos::reset_stale_running_sessions(&pool).await {
        Ok(n) => {
            if n > 0 {
                tracing::info!(reset = n, "已重置遗留运行态会话");
            }
        }
        Err(e) => tracing::warn!(error = %e, "重置遗留会话失败（不影响启动）"),
    }
    // 启动清理：spill 临时证据分两档保留 —— 真会话 7 天 / 孤儿（无对应会话的目录、
    match db::repos::all_session_ids(&pool).await {
        Ok(known) => {
            let removed = mcp::spill::cleanup_expired_with(
                mcp::spill::SPILL_RETENTION_DAYS,
                mcp::spill::SPILL_ORPHAN_RETENTION_DAYS,
                &known,
            );
            if removed > 0 {
                tracing::info!(
                    removed,
                    sessions = known.len(),
                    "启动期清理过期 spill 条目完成"
                );
            }
        }
        Err(e) => tracing::warn!(error = %e, "取会话名单失败，启动期 spill 清理按单档跳过"),
    }
    // tmp/ 里的过程脚本同口径清理（执行层无会话身份，只能按天）
    let tmp_removed = crate::path::data_root::cleanup_tmp(mcp::spill::SPILL_RETENTION_DAYS);
    if tmp_removed > 0 {
        tracing::info!(removed = tmp_removed, "启动期清理过期过程脚本完成");
    }

    // 会话退场：启动即扫一轮（服务可能停了很久，重启时才等得到这一轮）。
    {
        let warm = config::settings::tuning()
            .retire_warm_secs
            .load(std::sync::atomic::Ordering::Relaxed);
        let cold = config::settings::tuning()
            .retire_cold_secs
            .load(std::sync::atomic::Ordering::Relaxed);
        match agent::history::sweep(&pool, warm, cold).await {
            Ok(rep) if rep.summarized > 0 || rep.purged > 0 => tracing::info!(
                summarized = rep.summarized, purged = rep.purged,
                purged_rows = rep.purged_rows, failures = rep.failures,
                "启动期会话退场扫描完成"
            ),
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "启动期会话退场扫描失败（不影响启动）"),
        }
    }

    // ---- LLM（运行时设置：环境变量为默认，DB settings 表为覆盖项） ----
    let (llm, settings) = model::llm::build_llm(&cfg);
    // 收敛/记忆调优参数：env 默认 → DB settings 表覆盖（agent 层只读全局原子，热生效）
    config::settings::init_tuning_from_env();
    {
        let db_settings = db::repos::list_settings(&pool).await?;
        // 启动期一次性纠正：DB 里若存着**已退役的模型名**，写回新名。
        if let Some(("model", old)) = db_settings
            .iter()
            .find(|(k, _)| k == "model")
            .map(|(k, v)| (k.as_str(), v.as_str()))
        {
            let fixed = config::normalize_model_name(old);
            if fixed != old {
                match db::repos::set_setting(&pool, "model", &fixed).await {
                    Ok(_) => tracing::info!(
                        from = %old, to = %fixed,
                        "DB 中的退役模型名已写回新名（下次启动不再需要纠正）"
                    ),
                    Err(e) => tracing::warn!(
                        error = %e, from = %old,
                        "退役模型名写回失败（本次运行仍按归一化值生效，仅DB未更新）"
                    ),
                }
            }
        }
        let mut rt = settings.write().await;
        rt.apply_overrides(db_settings.clone());
        tracing::info!(
            mode = ?rt.llm_mode,
            model = %rt.model,
            model_label = %model::catalog::display_name_of(&rt.model),
            "运行时设置已生效（env 默认 + DB 覆盖）"
        );
        config::settings::apply_tuning_overrides(&db_settings);
    }
    {
        let slot = crate::tools::web_tools::SEARCH_KEY_SETTING;
        let from_slot = db::repos::get_setting(&pool, slot).await?;
        let legacy = match model::catalog::web_search_provider() {
            Some(p) => db::repos::get_setting(&pool, &model::catalog::key_slot(&p.id)).await?,
            None => None,
        };
        let effective = from_slot
            .as_deref()
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .map(str::to_string)
            .or_else(|| {
                legacy
                    .as_deref()
                    .map(str::trim)
                    .filter(|k| !k.is_empty())
                    .map(str::to_string)
            });
        match effective {
            Some(k) => {
                let src = if from_slot.is_some() { "独立设置" } else { "厂商档案(兼容旧配置)" };
                tools::web_tools::set_search_key(Some(k));
                tracing::info!(source = src, "web_search 凭据已回填（与模型解耦）");
            }
            None => {
                tracing::info!("web_search 无凭据：该工具不出现在工具面（配好设置后重启生效）");
            }
        }
    }

    // ---- MCP 工具注册表 ----
    let mcp_configs: Vec<McpServerConfigRef> = {
        let db_cfg = db::repos::get_setting(&pool, "mcp_servers")
            .await?
            .and_then(|raw| match serde_json::from_str::<Vec<config::McpServerConfig>>(&raw) {
                Ok(v) if !v.is_empty() => Some(v),
                Ok(_) => {
                    tracing::info!("DB mcp_servers 为空，回退 env 配置");
                    None
                }
                Err(e) => {
                    tracing::warn!(error = %e, "DB mcp_servers 解析失败，回退 env 配置");
                    None
                }
            });
        let src = db_cfg.unwrap_or_else(|| cfg.mcp_servers.clone());
        src.iter()
            .map(|s| McpServerConfigRef {
                name: s.name.clone(),
                transport: s.transport.clone(),
                command: s.command.clone(),
                args: s.args.clone(),
                url: s.url.clone(),
                headers: s.headers.clone(),
            })
            .collect()
    };
    let registry =
        Arc::new(mcp::registry::Registry::new(&mcp_configs, cfg.result_truncate_chars).await?);
    tracing::info!(tools = ?registry.tool_names(), "工具注册表就绪");

    // ---- 状态 ----
    let state = AppState {
        cfg: cfg.clone(),
        pool,
        llm,
        settings,
        registry,
        hub: sse::EventHub::new(),
        cancels: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        confirm_gate: Arc::new(crate::confirm::ConfirmGate::new()),
    };

    // ---- 定时触发调度器（Cron / Heartbeat）----
    let scheduler_state = state.clone();
    crate::scheduler::start(scheduler_state);

    // ---- 会话退场扫描器（缓慢退出清场）----
    crate::agent::history::spawn_retirement_sweeper(state.pool.clone());

    // ---- spill 清理器（全文层的回收）----
    crate::mcp::spill::spawn_spill_sweeper(state.pool.clone());

    // ---- 路由 ----
    let app = routes::router().with_state(state);
    let addr = format!("{}:{}", cfg.host, cfg.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(addr = %addr, "HTTP 服务已监听");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.expect("监听 Ctrl-C 失败");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("监听 SIGTERM 失败")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => tracing::info!("收到 Ctrl-C，优雅退出"),
        _ = terminate => tracing::info!("收到终止信号，优雅退出"),
    }
}
