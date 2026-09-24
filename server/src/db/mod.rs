//! 数据库连接与迁移（WAL）

pub mod repos;

use crate::config::Config;
use crate::error::AppResult;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use std::str::FromStr;
use std::sync::Arc;

pub async fn init_pool(cfg: &Arc<Config>) -> AppResult<SqlitePool> {
    let opts = SqliteConnectOptions::from_str(&cfg.database_url)
        .map_err(|e| crate::error::AppError::Config(format!("数据库 URL 非法: {e}")))?
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(std::time::Duration::from_secs(5));

    let pool = SqlitePoolOptions::new()
        .max_connections(cfg.db_pool_max)
        .connect_with(opts)
        .await
        .map_err(|e| crate::error::AppError::Config(format!("数据库连接失败: {e}")))?;

    // 编译期嵌入迁移，启动自动执行
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .map_err(|e| crate::error::AppError::Config(format!("数据库迁移失败: {e}")))?;

    // 修复（用户反馈"会话消失/数据不稳定"）
    if let Ok(row) = sqlx::query("PRAGMA database_list").fetch_one(&pool).await {
        if let Ok(file) = row.try_get::<String, _>("file") {
            tracing::info!(db = %file, "SQLite 数据库已连接");
        }
    }
    match sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(&pool)
        .await
    {
        Ok(_) => tracing::info!("WAL checkpoint 完成（WAL 已刷入主库）"),
        Err(e) => tracing::warn!(error = %e, "WAL checkpoint 失败（不影响启动）"),
    }

    Ok(pool)
}
