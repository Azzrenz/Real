//! 设备鉴权域：配对码 → 设备 token；SSE 一次性票据。
//!
//! 安全模型（本模块的边界，与 `routes/auth_guard.rs` 配套）：
//! - **回环来源免鉴权**：桌面 Tauri 壳打 `127.0.0.1` ⇒ 零变化（旧行为完全保留）。
//! - 非回环来源必须持有效设备 token（HttpOnly Cookie `real_device` 或 `Authorization: Bearer`）。
//! - 配对码**只能本机生成**（`POST /api/auth/pair/code` 属仅本机接口），远程攻击者无法自配对。
//! - token = 256-bit CSPRNG；明文只在配对响应里返回一次，服务端**只存 SHA-256 哈希**，
//!   校验走常量时间比较（防 timing oracle）。
//! - SSE 不能自定义 header（前端用 `new EventSource()`）⇒ 用**一次性短票据**（绑定会话 id，
//!   60 秒有效，用后即弃）；**长期 token 绝不放 URL**（会进日志/历史/代理）。

use crate::error::AppResult;
use rand::rngs::OsRng;
use rand::{Rng, RngCore};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, SqlitePool};
use std::collections::HashMap;
use std::sync::Mutex;

/// 配对码有效期（秒）：5 分钟
pub const PAIR_CODE_TTL_SECS: i64 = 300;
/// 配对码尝试上限：超限即作废（须本机重新生成）
pub const PAIR_CODE_MAX_ATTEMPTS: u32 = 5;
/// 配对接口限速窗口（秒）与窗口内上限——唯一的未鉴权写入口必须限速
pub const PAIR_RATE_WINDOW_SECS: i64 = 60;
pub const PAIR_RATE_MAX: u32 = 10;
/// SSE 一次性票据有效期（秒）
pub const SSE_TICKET_TTL_SECS: i64 = 60;
/// token 熵（字节）⇒ 256 bit
pub const TOKEN_BYTES: usize = 32;
/// 设备 token 的 Cookie 名（HttpOnly；`SameSite=Lax` 覆盖同站手机访问）
pub const DEVICE_COOKIE: &str = "real_device";
/// Cookie 有效期（秒，一年）
pub const DEVICE_COOKIE_MAX_AGE_SECS: i64 = 365 * 24 * 3600;
/// `last_seen` 写入节流间隔（毫秒）——避免每个鉴权请求都写一次库
pub const TOUCH_INTERVAL_MS: i64 = 60_000;

/// 配对设备（`token_hash` 永不外传）
#[derive(Debug, Clone, FromRow, serde::Serialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    /// token 的 SHA-256 十六进制摘要（**不是明文**）；序列化时剔除，杜绝哈希外泄
    #[serde(skip_serializing)]
    pub token_hash: String,
    pub created_at: String,
    /// 最近使用时刻（epoch 毫秒）
    pub last_seen: i64,
}

/// 配对失败原因（对外只暴露人读文案，不泄露内部状态）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairError {
    /// 本机尚未生成配对码
    NoCode,
    /// 配对码已过期
    Expired,
    /// 配对码不正确（已计入尝试）
    Rejected,
    /// 尝试次数用尽，配对码已作废
    Exhausted,
    /// 命中限速
    RateLimited,
}

impl PairError {
    pub fn message(self) -> &'static str {
        match self {
            PairError::NoCode => "本机尚未生成配对码（请在电脑上点「生成配对码」）",
            PairError::Expired => "配对码已过期，请在电脑上重新生成",
            PairError::Rejected => "配对码不正确",
            PairError::Exhausted => "配对码尝试次数已用尽，请在电脑上重新生成",
            PairError::RateLimited => "配对请求过于频繁，请稍后再试",
        }
    }
}

struct PairCode {
    code: String,
    expires_at: i64,
    attempts: u32,
}

struct SseTicket {
    session_id: String,
    expires_at: i64,
}

#[derive(Default)]
struct RateWindow {
    window_start: i64,
    count: u32,
}

/// 鉴权运行时状态：配对码 / SSE 票据为**内存态**（短时效，重启即失效无妨），
/// 设备档案落库（见 `create_device` 等）。
#[derive(Default)]
pub struct AuthStore {
    pairing: Mutex<Option<PairCode>>,
    tickets: Mutex<HashMap<String, SseTicket>>,
    pair_rate: Mutex<RateWindow>,
}

impl AuthStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 生成 6 位配对码并覆盖上一枚；返回 (码, 过期时刻)。
    pub fn issue_pair_code(&self, now: i64) -> (String, i64) {
        let code = six_digit_code();
        let expires_at = now + PAIR_CODE_TTL_SECS;
        *self.lock_pairing() = Some(PairCode {
            code: code.clone(),
            expires_at,
            attempts: 0,
        });
        (code, expires_at)
    }

    /// 配对接口限速（滑窗近似）。只有**未命中成功**的调用才该计数。
    pub fn allow_pair_attempt(&self, now: i64) -> bool {
        let mut w = self
            .pair_rate
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if now - w.window_start >= PAIR_RATE_WINDOW_SECS {
            w.window_start = now;
            w.count = 0;
        }
        if w.count >= PAIR_RATE_MAX {
            return false;
        }
        w.count += 1;
        true
    }

    /// 用配对码换 token（明文，**仅此一次**）。成功即作废配对码；错误尝试累计到上限即作废。
    pub fn redeem_pair_code(&self, input: &str, now: i64) -> Result<String, PairError> {
        let mut slot = self.lock_pairing();
        let Some(pc) = slot.as_mut() else {
            return Err(PairError::NoCode);
        };
        if now > pc.expires_at {
            *slot = None;
            return Err(PairError::Expired);
        }
        if constant_time_eq(input.trim().as_bytes(), pc.code.as_bytes()) {
            *slot = None; // 用后即弃
            return Ok(random_token());
        }
        pc.attempts += 1;
        if pc.attempts >= PAIR_CODE_MAX_ATTEMPTS {
            *slot = None; // 超限即作废
            return Err(PairError::Exhausted);
        }
        Err(PairError::Rejected)
    }

    /// 签发 SSE 一次性票据（绑定会话 id）；顺带清理过期票据。
    pub fn issue_sse_ticket(&self, session_id: &str, now: i64) -> (String, i64) {
        let ticket = random_token();
        let expires_at = now + SSE_TICKET_TTL_SECS;
        let mut map = self.lock_tickets();
        map.retain(|_, t| t.expires_at >= now);
        map.insert(
            ticket.clone(),
            SseTicket {
                session_id: session_id.to_string(),
                expires_at,
            },
        );
        (ticket, expires_at)
    }

    /// 消费 SSE 票据：一次性、未过期、绑定会话必须相符（取出即失效）。
    pub fn consume_sse_ticket(&self, ticket: &str, session_id: &str, now: i64) -> bool {
        let mut map = self.lock_tickets();
        let Some(t) = map.remove(ticket) else {
            return false;
        };
        t.expires_at >= now && t.session_id == session_id
    }

    fn lock_pairing(&self) -> std::sync::MutexGuard<'_, Option<PairCode>> {
        self.pairing.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn lock_tickets(&self) -> std::sync::MutexGuard<'_, HashMap<String, SseTicket>> {
        self.tickets.lock().unwrap_or_else(|e| e.into_inner())
    }
}

// ── 时钟 ────────────────────────────────────────────────────────────────
pub fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

// ── token 生成 / 摘要 / 比较 ─────────────────────────────────────────────
/// 256-bit CSPRNG token（十六进制 64 字符）。
fn random_token() -> String {
    let mut buf = [0u8; TOKEN_BYTES];
    OsRng.fill_bytes(&mut buf);
    hex(&buf)
}

/// 6 位数字配对码（CSPRNG，均匀分布；前导零保留）。
fn six_digit_code() -> String {
    format!("{:06}", OsRng.gen_range(0..1_000_000u32))
}

fn hex(bytes: &[u8]) -> String {
    const H: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(H[(b >> 4) as usize] as char);
        s.push(H[(b & 0x0f) as usize] as char);
    }
    s
}

/// token → SHA-256 十六进制摘要（**落库的只有它**）。
pub fn hash_token(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}

/// 常量时间字节比较（长度不同直接 false——长度是公开信息，不是秘密）。
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    // black_box：防止优化器把逐字节比较改回短路比较
    std::hint::black_box(diff) == 0
}

/// 设备 Cookie（HttpOnly + SameSite=Lax）。
/// 注意：局域网明文 HTTP 下**不能**加 `Secure`（加了浏览器就不落盘，手机连不上）。
pub fn device_cookie(token: &str) -> String {
    format!(
        "{DEVICE_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={DEVICE_COOKIE_MAX_AGE_SECS}"
    )
}

// ── 设备档案（落库） ─────────────────────────────────────────────────────
fn sanitize_name(raw: &str) -> String {
    let cleaned: String = raw
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(64)
        .collect();
    if cleaned.is_empty() {
        "未命名设备".to_string()
    } else {
        cleaned
    }
}

/// 新建设备档案：传明文 token，落库只存哈希。
pub async fn create_device(pool: &SqlitePool, name: &str, token: &str) -> AppResult<Device> {
    let device = Device {
        id: uuid::Uuid::new_v4().to_string(),
        name: sanitize_name(name),
        token_hash: hash_token(token),
        created_at: chrono::Utc::now().to_rfc3339(),
        last_seen: now_ms(),
    };
    sqlx::query(
        "INSERT INTO devices (id, name, token_hash, created_at, last_seen) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )
    .bind(&device.id)
    .bind(&device.name)
    .bind(&device.token_hash)
    .bind(&device.created_at)
    .bind(device.last_seen)
    .execute(pool)
    .await?;
    Ok(device)
}

/// 校验设备 token：按哈希精确取（唯一索引），再用常量时间比较复核。
/// 注：按哈希做索引查找不会泄露 token（SHA-256 原像不可逆），复核比较只为不出现短路比较。
pub async fn verify_token(pool: &SqlitePool, token: &str) -> AppResult<Option<Device>> {
    let digest = hash_token(token);
    let row = sqlx::query_as::<_, Device>(
        "SELECT id, name, token_hash, created_at, last_seen FROM devices WHERE token_hash = ?1",
    )
    .bind(&digest)
    .fetch_optional(pool)
    .await?;
    Ok(row.filter(|d| constant_time_eq(d.token_hash.as_bytes(), digest.as_bytes())))
}

pub async fn list_devices(pool: &SqlitePool) -> AppResult<Vec<Device>> {
    Ok(sqlx::query_as::<_, Device>(
        "SELECT id, name, token_hash, created_at, last_seen FROM devices ORDER BY created_at",
    )
    .fetch_all(pool)
    .await?)
}

/// 踢掉设备；返回是否命中（未命中 ⇒ 调用方回 404）。
pub async fn delete_device(pool: &SqlitePool, id: &str) -> AppResult<bool> {
    let affected = sqlx::query("DELETE FROM devices WHERE id = ?1")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();
    Ok(affected > 0)
}

/// 节流更新 `last_seen`：距上次不足 `TOUCH_INTERVAL_MS` 则不写。
pub async fn touch_device(pool: &SqlitePool, id: &str, now: i64) -> AppResult<()> {
    sqlx::query("UPDATE devices SET last_seen = ?2 WHERE id = ?1 AND (?2 - last_seen) >= ?3")
        .bind(id)
        .bind(now)
        .bind(TOUCH_INTERVAL_MS)
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
#[path = "auth_tests.rs"]
mod auth_tests;
