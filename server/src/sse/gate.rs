//! 流式发射器——SSE 输出协议归 SSE 域，编排域只管喂增量。

use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;

/// 合并窗口。取值依据：模型增量到达速率远快于此，窗口内通常攒到几十~上百字符，
const COALESCE_WINDOW: Duration = Duration::from_millis(250);

/// 缓冲上限：达到即提前冲出，避免长窗口下积压过一个 SSE 帧过大
const COALESCE_MAX_CHARS: usize = 2048;

/// 内部可变状态：缓冲 + 归属者标识（用于判断是否跨流）
struct Inner {
    buf: String,
    /// 仅用于调试与测试断言，不参与逻辑
    last_flush: Instant,
}

/// 定时冲刷任务的退场口：停止位 + 句柄。
struct Ticker {
    /// 置位后 ticker 下一拍即自行退出（不依赖 abort，故可跨运行时边界安全生效）
    stopped: Arc<AtomicBool>,
    /// 任务句柄：`shutdown()` 用它 abort + join，确保 `tx` 克隆真的析构了
    handle: Option<tokio::task::JoinHandle<()>>,
}

/// 流式发射器：一条流一个实例（kind = EV_REASONING / EV_MESSAGE）。
#[derive(Clone)]
pub struct StreamEmitter {
    tx: UnboundedSender<(&'static str, Value)>,
    kind: &'static str,
    inner: Arc<Mutex<Inner>>,
    /// 是否已经因为窗口到期而排空过（仅测试/观测用）
    drained: Arc<AtomicBool>,
    /// 定时冲刷任务的退场口（`arm_ticker` 装配，`shutdown` 拆解）
    ticker: Arc<Mutex<Ticker>>,
}

impl StreamEmitter {
    pub fn new(kind: &'static str, tx: UnboundedSender<(&'static str, Value)>) -> Self {
        Self {
            tx,
            kind,
            inner: Arc::new(Mutex::new(Inner {
                buf: String::new(),
                last_flush: Instant::now(),
            })),
            drained: Arc::new(AtomicBool::new(false)),
            ticker: Arc::new(Mutex::new(Ticker {
                stopped: Arc::new(AtomicBool::new(false)),
                handle: None,
            })),
        }
    }

    /// 挂定时冲刷：流式期间模型可能吐几个 token 后停顿数秒，
    pub fn arm_ticker(&self) {
        let mut t = self.ticker.lock().unwrap_or_else(|e| e.into_inner());
        if t.handle.is_some() || t.stopped.load(Ordering::Acquire) {
            return;
        }
        // 重新取一个未置位的停止位：允许"退场后再次装配"（重试场景会重新开流）。
        let stopped = Arc::new(AtomicBool::new(false));
        t.stopped = stopped.clone();
        let me = self.clone();
        let handle = tokio::spawn(async move {
            let mut iv = tokio::time::interval(COALESCE_WINDOW);
            iv.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            // 第一拍立即到期，跳过以免与 feed 抢首发
            iv.tick().await;
            loop {
                iv.tick().await;
                // 退场信号优先：置位即结束，不再碰 tx（这正是挂起的解药）
                if stopped.load(Ordering::Acquire) {
                    return;
                }
                // 缓冲空了就别做无谓加锁；flush 自身对空缓冲是 no-op
                if me.pending_chars() == 0 {
                    continue;
                }
                me.flush();
                me.drained.store(true, Ordering::Relaxed);
            }
        });
        t.handle = Some(handle);
    }

    /// 停表并等待任务真正结束。
    pub async fn shutdown(&self) {
        let handle = {
            let mut t = self.ticker.lock().unwrap_or_else(|e| e.into_inner());
            t.stopped.store(true, Ordering::Release);
            t.handle.take()
        };
        if let Some(h) = handle {
            // 停止位已置位，任务最多再走一拍（≤ 一个窗口）即自行退出；
            h.abort();
            let _ = h.await;
        }
    }

    /// 退场口是否已经置位（测试/观测用）。
    #[allow(dead_code)]
    pub fn is_shutdown(&self) -> bool {
        self.ticker
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .stopped
            .load(Ordering::Acquire)
    }

    /// 喂一个上游增量：写入缓冲，必要时合并发射。
    pub fn feed(&self, delta: &str) {
        if delta.is_empty() {
            return;
        }
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.buf.push_str(delta);

        let due = g.buf.chars().count() >= COALESCE_MAX_CHARS
            || g.last_flush.elapsed() >= COALESCE_WINDOW;
        if due {
            Self::drain_locked(&mut g, &self.tx, self.kind);
            self.drained.store(true, Ordering::Relaxed);
        }
    }

    /// 流收尾：把缓冲吐干。调用方依赖"flush 后不再有本段增量"这一契约。
    pub fn flush(&self) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        Self::drain_locked(&mut g, &self.tx, self.kind);
    }

    /// 丢弃半截流：清缓冲不清已发出的内容。
    pub fn reset(&self) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.buf.clear();
        g.last_flush = Instant::now();
    }

    /// 当前缓冲中待发的字符数（观测/测试用）。
    pub fn pending_chars(&self) -> usize {
        let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.buf.chars().count()
    }

    /// 取出缓冲并作为**一条**事件发射。调用方必须已持有锁。
    fn drain_locked(
        g: &mut Inner,
        tx: &UnboundedSender<(&'static str, Value)>,
        kind: &'static str,
    ) {
        if g.buf.is_empty() {
            g.last_flush = Instant::now();
            return;
        }
        let text = std::mem::take(&mut g.buf);
        g.last_flush = Instant::now();
        let _ = tx.send((kind, json!({ "text": text })));
    }
}

#[cfg(test)]
#[path = "gate_tests.rs"]
mod gate_tests;
