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
    //
    // 正式软件的口径：迁移只进不退，但**旧程序允许在新库上启动**。
    // 场景：用户先用过更新版本的 Real，再打开旧版本（降级）——旧版本代码里没有
    // 比库更新的那些迁移文件，sqlx 默认会报
    // "migration N was previously applied but is missing in the resolved migrations" 并拒绝启动。
    // 处理：忽略代码中缺失的已应用迁移，同时明确告警，让程序可用而不是直接罢工。
    let mut migrator = sqlx::migrate!("./migrations");
    let known_max = migrator.migrations.iter().map(|m| m.version).max().unwrap_or(0);
    let applied_max = applied_max_version(&pool).await;
    let db_ahead = applied_max > known_max;

    migrator.set_ignore_missing(true);
    migrator
        .run(&pool)
        .await
        .map_err(|e| crate::error::AppError::Config(format!("数据库迁移失败: {e}")))?;

    if db_ahead {
        tracing::warn!(
            db_version = applied_max,
            app_version = known_max,
            "数据目录由更新版本的 Real 使用过：已忽略本版本不认识的迁移，程序可正常启动；\
             新版写入的数据在本版本中不会显示，需要完整体验请升级到最新版本"
        );
    }

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

/// 读取库内已应用的最高迁移版本；库或表不存在时返回 0（视为全新库）。
async fn applied_max_version(pool: &SqlitePool) -> i64 {
    match sqlx::query_scalar::<_, i64>("SELECT COALESCE(MAX(version), 0) FROM _sqlx_migrations")
        .fetch_one(pool)
        .await
    {
        Ok(v) => v,
        // 表不存在（全新库）或查询失败：按 0 处理，交给 migrate 正常建表
        Err(_) => 0,
    }
}
