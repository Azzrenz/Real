//! 定时触发调度器：cron/自然语言 → 心跳到点执行（复用 run_agent 全流水线）

use crate::agent::orchestration::run_agent;
use crate::db::repos;
use crate::error::AppResult;
use crate::state::AppState;
use chrono::{DateTime, Datelike, Duration as ChronoDuration, NaiveDate, TimeZone, Timelike, Utc};
use serde::Serialize;
use sqlx::sqlite::SqlitePool;
use sqlx::FromRow;
use std::time::Duration as StdDuration;

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct ScheduledJob {
    pub id: String,
    pub name: String,
    pub prompt: String,
    pub cron_expr: String,
    pub natural_lang: String,
    pub workspace: Option<String>,
    pub enabled: bool,
    pub last_run_at: Option<String>,
    pub last_status: Option<String>,
    pub last_result: Option<String>,
    pub next_run_at: Option<String>,
    pub created_at: String,
}

// Cron 解析（最小 5 字段：分 时 日 月 周）

#[derive(Debug, Clone)]
pub struct CronSpec {
    minutes: Vec<u32>,
    hours: Vec<u32>,
    days: Vec<u32>,
    months: Vec<u32>,
    weekdays: Vec<u32>,
}

fn parse_field(field: &str, min: u32, max: u32) -> Result<Vec<u32>, String> {
    let mut out = Vec::new();
    for part in field.split(',') {
        if part.is_empty() {
            return Err(format!("空字段段: {field}"));
        }
        let (range, step) = if let Some(idx) = part.find('/') {
            (&part[..idx], &part[idx + 1..])
        } else {
            (part, "1")
        };
        let step: u32 = step.parse().map_err(|_| format!("步长非法: {part}"))?;
        if step == 0 {
            return Err(format!("步长不能为 0: {part}"));
        }
        let (lo, hi) = if range == "*" {
            (min, max)
        } else if let Some(d) = range.find('-') {
            let a: u32 = range[..d]
                .parse()
                .map_err(|_| format!("范围起点非法: {range}"))?;
            let b: u32 = range[d + 1..]
                .parse()
                .map_err(|_| format!("范围终点非法: {range}"))?;
            (a, b)
        } else {
            let v: u32 = range.parse().map_err(|_| format!("数值非法: {range}"))?;
            (v, v)
        };
        if lo < min || hi > max || lo > hi {
            return Err(format!("字段越界 [{lo},{hi}] 超出 {min}-{max}: {field}"));
        }
        let mut v = lo;
        while v <= hi {
            out.push(v);
            v += step;
        }
    }
    out.sort_unstable();
    out.dedup();
    if out.is_empty() {
        return Err(format!("字段无合法值: {field}"));
    }
    Ok(out)
}

/// 解析标准 5 字段 cron。返回错误文本便于直接透传给前端校验。
pub fn parse_cron(expr: &str) -> Result<CronSpec, String> {
    let f: Vec<&str> = expr.split_whitespace().collect();
    if f.len() != 5 {
        return Err(format!(
            "cron 必须是 5 字段（分 时 日 月 周），收到 {} 字段: {expr}",
            f.len()
        ));
    }
    let minutes = parse_field(f[0], 0, 59)?;
    let hours = parse_field(f[1], 0, 23)?;
    let days = parse_field(f[2], 1, 31)?;
    let months = parse_field(f[3], 1, 12)?;
    let mut weekdays = parse_field(f[4], 0, 7)?;
    weekdays = weekdays
        .into_iter()
        .map(|v| if v == 7 { 0 } else { v })
        .collect();
    weekdays.sort_unstable();
    weekdays.dedup();
    Ok(CronSpec {
        minutes,
        hours,
        days,
        months,
        weekdays,
    })
}

/// 字段内「>= cur 的最小允许值」，无则回绕到最小值并标记 wrapped。
fn next_in_field(values: &[u32], cur: u32) -> (u32, bool) {
    for &v in values {
        if v >= cur {
            return (v, false);
        }
    }
    (values[0], true)
}

fn at_midnight(y: i32, m: u32, d: u32) -> chrono::NaiveDateTime {
    NaiveDate::from_ymd_opt(y, m, d)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap()
}

/// 计算 after 之后的下一次触发时间（不含 after 本身）。上限约 4 年，超时返回 None。
pub fn next_occurrence(spec: &CronSpec, after: &DateTime<Utc>) -> Option<DateTime<Utc>> {
    let mut t = (*after + ChronoDuration::minutes(1)).naive_utc();
    t = t.with_second(0).unwrap().with_nanosecond(0).unwrap();
    let dom_restricted = spec.days.len() < 31;
    let dow_restricted = spec.weekdays.len() < 7;
    let cap: i64 = 4 * 366 * 24 * 60;
    for _ in 0..cap {
        // 月
        if !spec.months.contains(&t.month()) {
            let (m, wrapped) = next_in_field(&spec.months, t.month());
            if wrapped {
                t = at_midnight(t.year() + 1, 1, 1);
            } else {
                t = at_midnight(t.year(), m, 1);
            }
            continue;
        }
        // 日：cron 语义——日/周任一受限时取「或」，两者都 * 时恒真
        let wd = t.weekday().num_days_from_sunday() as u32;
        let dom_ok = spec.days.contains(&t.day());
        let dow_ok = spec.weekdays.contains(&wd);
        let day_match = match (dom_restricted, dow_restricted) {
            (false, false) => true,
            (true, false) => dom_ok,
            (false, true) => dow_ok,
            (true, true) => dom_ok || dow_ok,
        };
        if !day_match {
            t = (t + ChronoDuration::days(1))
                .with_hour(0)
                .unwrap()
                .with_minute(0)
                .unwrap()
                .with_second(0)
                .unwrap()
                .with_nanosecond(0)
                .unwrap();
            continue;
        }
        // 时
        if !spec.hours.contains(&t.hour()) {
            let (h, wrapped) = next_in_field(&spec.hours, t.hour());
            if wrapped {
                t = (t + ChronoDuration::days(1))
                    .with_hour(0)
                    .unwrap()
                    .with_minute(0)
                    .unwrap()
                    .with_second(0)
                    .unwrap()
                    .with_nanosecond(0)
                    .unwrap();
            } else {
                t = t
                    .with_hour(h)
                    .unwrap()
                    .with_minute(0)
                    .unwrap()
                    .with_second(0)
                    .unwrap()
                    .with_nanosecond(0)
                    .unwrap();
            }
            continue;
        }
        // 分
        if !spec.minutes.contains(&t.minute()) {
            let (mi, wrapped) = next_in_field(&spec.minutes, t.minute());
            if wrapped {
                t = (t + ChronoDuration::hours(1))
                    .with_minute(0)
                    .unwrap()
                    .with_second(0)
                    .unwrap()
                    .with_nanosecond(0)
                    .unwrap();
            } else {
                t = t
                    .with_minute(mi)
                    .unwrap()
                    .with_second(0)
                    .unwrap()
                    .with_nanosecond(0)
                    .unwrap();
            }
            continue;
        }
        return Some(Utc.from_utc_datetime(&t));
    }
    None
}

/// 由 cron 表达式直接算下次触发（解析失败返回 None）。供建表/启停补算 next_run_at。
pub fn next_occurrence_cron(expr: &str, after: &DateTime<Utc>) -> Option<DateTime<Utc>> {
    parse_cron(expr)
        .ok()
        .and_then(|spec| next_occurrence(&spec, after))
}

// 自然语言 → cron 快捷解析（覆盖常见中文/英文表达）

/// 从文本抽取「时:分」。返回 (hour_opt, minute)。
fn parse_time(s: &str) -> (Option<u32>, u32) {
    let lower = s.to_lowercase();
    let chars: Vec<char> = lower.chars().collect();
    let mut hour: Option<u32> = None;
    let mut minute: u32 = 0;

    // "HH:MM" / "H:MM"（允许前缀文字，如「每周三15:30」——只取 ':' 两侧紧邻数字）
    if let Some(pos) = chars.iter().position(|&c| c == ':') {
        let mut i = pos;
        while i > 0 && chars[i - 1].is_ascii_digit() {
            i -= 1;
        }
        let h_str: String = chars[i..pos].iter().collect();
        if let Ok(h) = h_str.parse::<u32>() {
            if h <= 23 {
                hour = Some(h);
            }
        }
        let mut j = pos + 1;
        while j < chars.len() && chars[j].is_ascii_digit() {
            j += 1;
        }
        let m_str: String = chars[pos + 1..j].iter().collect();
        if let Ok(m) = m_str.parse::<u32>() {
            if m <= 59 {
                minute = m;
            }
        }
    }

    // "X点" / "X时" / "X点半" / "X点30分"（仅在未从 HH:MM 解析到小时时）
    if hour.is_none() {
        let mut i = 0;
        while i < chars.len() {
            if chars[i].is_ascii_digit() {
                let mut j = i;
                while j < chars.len() && chars[j].is_ascii_digit() {
                    j += 1;
                }
                let num: String = chars[i..j].iter().collect();
                if let Ok(h) = num.parse::<u32>() {
                    if h <= 23 && j < chars.len() && (chars[j] == '点' || chars[j] == '时') {
                        hour = Some(h);
                        if j + 1 < chars.len() {
                            if chars[j + 1] == '半' {
                                minute = 30;
                            } else if chars[j + 1].is_ascii_digit() {
                                let mut k = j + 1;
                                while k < chars.len() && chars[k].is_ascii_digit() {
                                    k += 1;
                                }
                                let mn: String = chars[j + 1..k].iter().collect();
                                if let Ok(m) = mn.parse::<u32>() {
                                    if m <= 59 {
                                        minute = m;
                                    }
                                }
                            }
                        }
                        break;
                    }
                }
            }
            i += 1;
        }
    }
    (hour, minute)
}

fn parse_nl_to_cron(nl: &str) -> Option<String> {
    let lower = nl.to_lowercase();
    let (hour, minute) = parse_time(nl);
    let h = hour.unwrap_or(9);
    let m = minute;

    if lower.contains("每小时") || lower.contains("hourly") || lower.contains("每隔一小时")
    {
        return Some("0 * * * *".to_string());
    }

    let weekdays = [
        ("周日", 0),
        ("星期天", 0),
        ("周天", 0),
        ("星期日", 0),
        ("周一", 1),
        ("星期一", 1),
        ("周二", 2),
        ("星期二", 2),
        ("周三", 3),
        ("星期三", 3),
        ("周四", 4),
        ("星期四", 4),
        ("周五", 5),
        ("星期五", 5),
        ("周六", 6),
        ("星期六", 6),
        ("sunday", 0),
        ("monday", 1),
        ("tuesday", 2),
        ("wednesday", 3),
        ("thursday", 4),
        ("friday", 5),
        ("saturday", 6),
    ];
    let mut dow: Option<u32> = None;
    for (kw, val) in weekdays {
        if lower.contains(kw) {
            dow = Some(val);
            break;
        }
    }

    if dow.is_some() || lower.contains("每周") || lower.contains("weekly") {
        return Some(format!("{} {} * * {}", m, h, dow.unwrap_or(1)));
    }
    if lower.contains("每月") || lower.contains("monthly") {
        return Some(format!("{} {} 1 * *", m, h));
    }
    if lower.contains("每天") || lower.contains("每日") || lower.contains("daily") {
        return Some(format!("{} {} * * *", m, h));
    }
    if hour.is_some() {
        return Some(format!("{} {} * * *", m, h));
    }
    None
}

/// 归一化调度表达式：先尝试标准 cron，失败再试自然语言；返回 (cron, natural_lang)。
pub fn normalize_schedule(schedule: &str) -> Result<(String, String), String> {
    let s = schedule.trim();
    let fields: Vec<&str> = s.split_whitespace().collect();
    let is_cron = fields.len() == 5
        && fields
            .iter()
            .all(|f| f.chars().all(|c| c.is_alphanumeric() || "*,-/".contains(c)));
    if is_cron {
        parse_cron(s)?;
        return Ok((s.to_string(), String::new()));
    }
    if let Some(cron) = parse_nl_to_cron(s) {
        return Ok((cron, s.to_string()));
    }
    Err(format!(
        "无法解析调度表达式：{s}（请使用标准 5 字段 cron 或自然语言如「每天9点」「每小时」「每周一」）"
    ))
}

// 数据访问（scheduled_jobs 表）

const COLS: &str = "id,name,prompt,cron_expr,natural_lang,workspace,enabled,last_run_at,last_status,last_result,next_run_at,created_at";

pub async fn create_job(
    pool: &SqlitePool,
    name: &str,
    prompt: &str,
    cron: &str,
    nl: &str,
    workspace: Option<&str>,
) -> AppResult<ScheduledJob> {
    let id = uuid::Uuid::new_v4().to_string();
    let ts = repos::now();
    let next = next_occurrence_cron(cron, &Utc::now()).map(|t| t.to_rfc3339());
    sqlx::query(
        "INSERT INTO scheduled_jobs (id,name,prompt,cron_expr,natural_lang,workspace,enabled,next_run_at,created_at)
         VALUES (?1,?2,?3,?4,?5,?6,1,?7,?8)",
    )
    .bind(&id)
    .bind(name)
    .bind(prompt)
    .bind(cron)
    .bind(nl)
    .bind(workspace)
    .bind(&next)
    .bind(&ts)
    .execute(pool)
    .await?;
    Ok(get_job(pool, &id).await?.expect("刚插入应可取回"))
}

pub async fn get_job(pool: &SqlitePool, id: &str) -> AppResult<Option<ScheduledJob>> {
    Ok(sqlx::query_as::<_, ScheduledJob>(&format!(
        "SELECT {COLS} FROM scheduled_jobs WHERE id = ?1"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await?)
}

pub async fn list_jobs(pool: &SqlitePool) -> AppResult<Vec<ScheduledJob>> {
    Ok(sqlx::query_as::<_, ScheduledJob>(&format!(
        "SELECT {COLS} FROM scheduled_jobs ORDER BY created_at DESC"
    ))
    .fetch_all(pool)
    .await?)
}

pub async fn list_enabled_with_next(pool: &SqlitePool) -> AppResult<Vec<ScheduledJob>> {
    Ok(sqlx::query_as::<_, ScheduledJob>(&format!(
        "SELECT {COLS} FROM scheduled_jobs WHERE enabled = 1 AND next_run_at IS NOT NULL"
    ))
    .fetch_all(pool)
    .await?)
}

pub async fn delete_job(pool: &SqlitePool, id: &str) -> AppResult<bool> {
    let r = sqlx::query("DELETE FROM scheduled_jobs WHERE id = ?1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(r.rows_affected() > 0)
}

pub async fn set_enabled(pool: &SqlitePool, id: &str, enabled: bool) -> AppResult<bool> {
    let r = sqlx::query("UPDATE scheduled_jobs SET enabled = ?2 WHERE id = ?1")
        .bind(id)
        .bind(enabled as i64)
        .execute(pool)
        .await?;
    Ok(r.rows_affected() > 0)
}

pub async fn reschedule(pool: &SqlitePool, id: &str, next_rfc3339: &str) -> AppResult<()> {
    sqlx::query("UPDATE scheduled_jobs SET next_run_at = ?2 WHERE id = ?1")
        .bind(id)
        .bind(next_rfc3339)
        .execute(pool)
        .await?;
    Ok(())
}

/// 清空 next_run_at（置 NULL，任务移出轮询但保留 enabled）。
pub async fn clear_next_run(pool: &SqlitePool, id: &str) -> AppResult<()> {
    sqlx::query("UPDATE scheduled_jobs SET next_run_at = NULL WHERE id = ?1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn mark_running(pool: &SqlitePool, id: &str, now_rfc3339: &str) -> AppResult<()> {
    sqlx::query(
        "UPDATE scheduled_jobs SET last_run_at = ?2, last_status = 'running' WHERE id = ?1",
    )
    .bind(id)
    .bind(now_rfc3339)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn update_run_result(
    pool: &SqlitePool,
    id: &str,
    status: &str,
    result: &str,
) -> AppResult<()> {
    sqlx::query("UPDATE scheduled_jobs SET last_status = ?2, last_result = ?3 WHERE id = ?1")
        .bind(id)
        .bind(status)
        .bind(result)
        .execute(pool)
        .await?;
    Ok(())
}

// 执行与心跳

/// 执行单个定时任务：建会话 → 复用 run_agent（与人工发消息同流水线）→ 落结果。
pub async fn run_job(state: AppState, job: ScheduledJob) {
    let session = match repos::create_session(&state.pool, &format!("[定时] {}", job.name), "")
        .await
    {
        Ok(s) => s,
        Err(e) => {
            let _ =
                update_run_result(&state.pool, &job.id, "error", &format!("建会话失败: {e}")).await;
            return;
        }
    };
    if let Some(ws) = &job.workspace {
        if !ws.trim().is_empty() {
            let _ = repos::set_session_workspace(&state.pool, &session.id, ws).await;
        }
    }
    // 定时任务同样按 run 记账（身份与取消令牌一致；终态事件自带 run_id）
    let run = state.open_run(&session.id);
    match run_agent(&state, &session.id, &job.prompt, Vec::new(), run).await {
        Ok(o) => {
            let summary: String = o.answer.chars().take(500).collect();
            let _ = update_run_result(&state.pool, &job.id, "ok", &summary).await;
            tracing::info!(job = %job.id, name = %job.name, "定时任务执行完成");
        }
        Err(e) => {
            let _ = update_run_result(&state.pool, &job.id, "error", &format!("{e}")).await;
            tracing::warn!(job = %job.id, error = %e, "定时任务执行失败");
        }
    }
}

/// 单次轮询：找出到期任务，先重排下次时间（防重触发），再异步 spawn 执行。
pub async fn tick(state: &AppState) {
    let jobs = match list_enabled_with_next(&state.pool).await {
        Ok(j) => j,
        Err(e) => {
            tracing::warn!(error = %e, "加载定时任务失败");
            return;
        }
    };
    let now = Utc::now();
    for job in jobs {
        let due = match &job.next_run_at {
            Some(nr) => chrono::DateTime::parse_from_rfc3339(nr)
                .map(|t| t.with_timezone(&Utc) <= now)
                .unwrap_or(false),
            None => false,
        };
        if !due {
            continue;
        }
        // 立即重排下次时间，避免本 tick 内被重复拾取
        match next_occurrence_cron(&job.cron_expr, &now) {
            Some(next) => {
                let _ = reschedule(&state.pool, &job.id, &next.to_rfc3339()).await;
            }
            None => {
                // 4 年窗口内算不出下次触发：cron 永不满足（如 `0 0 31 2 *` 2月31日）或
                let _ = clear_next_run(&state.pool, &job.id).await;
                let _ = update_run_result(
                    &state.pool,
                    &job.id,
                    "error",
                    "cron 无法计算下次触发（表达式可能永不满足或周期超过 4 年）",
                )
                .await;
                tracing::warn!(job = %job.id, cron = %job.cron_expr, "定时任务 cron 无法计算下次触发，已移出轮询（保留启用状态）");
                continue;
            }
        }
        let _ = mark_running(&state.pool, &job.id, &now.to_rfc3339()).await;
        let st = state.clone();
        let j = job.clone();
        tokio::spawn(async move {
            run_job(st, j).await;
        });
    }
}

/// 启动调度心跳：补全空 next_run_at，然后每 30s 轮询一次。
pub fn start(state: AppState) {
    tokio::spawn(async move {
        if let Ok(jobs) = list_enabled_with_next(&state.pool).await {
            let now = Utc::now();
            for job in jobs {
                if job.next_run_at.is_none() {
                    match next_occurrence_cron(&job.cron_expr, &now) {
                        Some(next) => {
                            let _ = reschedule(&state.pool, &job.id, &next.to_rfc3339()).await;
                        }
                        None => {
                            // 同 tick：不静默禁用，移出轮询 + 标 error + 告警（P1）
                            let _ = clear_next_run(&state.pool, &job.id).await;
                            let _ = update_run_result(
                                &state.pool,
                                &job.id,
                                "error",
                                "cron 无法计算下次触发（表达式可能永不满足或周期超过 4 年）",
                            )
                            .await;
                            tracing::warn!(job = %job.id, cron = %job.cron_expr, "定时任务 cron 无法计算下次触发，已移出轮询（保留启用状态）");
                        }
                    }
                }
            }
        }
        loop {
            tick(&state).await;
            tokio::time::sleep(StdDuration::from_secs(30)).await;
        }
    });
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod mod_tests;
