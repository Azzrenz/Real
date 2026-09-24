//! sse/gate.rs 的测试外置（部门盘查：测试全部移出核心文件）
use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    fn emitter() -> (
        StreamEmitter,
        std::sync::mpsc::Receiver<(&'static str, Value)>,
    ) {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(&'static str, Value)>();
        let e = StreamEmitter::new("test.stream", tx);
        let (stx, srx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            while let Some(ev) = rx.blocking_recv() {
                let _ = stx.send(ev);
            }
        });
        (e, srx)
    }

    /// 断言"一段时间内没有新事件"。
    fn quiet_for_100ms(rx: &std::sync::mpsc::Receiver<(&'static str, Value)>) -> bool {
        matches!(
            rx.recv_timeout(std::time::Duration::from_millis(100)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        )
    }

    /// 窗口内连喂多个增量 → 合并成一条事件，内容按序拼接。
    #[test]
    fn deltas_within_window_coalesce_into_one() {
        let (e, rx) = emitter();
        e.feed("第一");
        e.feed("第二");
        e.feed("第三");
        // 窗口未到期，不该有任何发射（缓冲里攒着）
        assert!(
            quiet_for_100ms(&rx),
            "窗口内不得逐条发射（这正是 slow statement 的病根）"
        );
        assert_eq!(e.pending_chars(), 6, "三个 2 字词应全在缓冲里");
        e.flush();
        let ev = rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("flush 后必须收到合并事件");
        assert_eq!(ev.0, "test.stream");
        assert_eq!(ev.1["text"], "第一第二第三", "合并后必须按序拼接");
    }

    /// flush 契约：吐干缓冲，之后缓冲为空、无残留。
    #[test]
    fn flush_drains_buffer_completely() {
        let (e, rx) = emitter();
        e.feed("尾");
        assert_eq!(e.pending_chars(), 1);
        e.flush();
        assert_eq!(
            rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap().1["text"],
            "尾"
        );
        assert_eq!(e.pending_chars(), 0, "flush 后必须无残留");
        // 再次 flush 是 no-op，不得产生空事件
        e.flush();
        assert!(quiet_for_100ms(&rx), "空缓冲 flush 不得发射空事件");
    }

    /// reset 契约：丢弃缓冲但保留已发出的内容。
    #[test]
    fn reset_discards_pending_but_keeps_emitted() {
        let (e, rx) = emitter();
        e.feed("已发");
        e.flush();
        assert_eq!(
            rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap().1["text"],
            "已发"
        );
        e.feed("半截");
        e.reset();
        assert_eq!(e.pending_chars(), 0, "reset 必须清空缓冲");
        e.flush();
        assert!(quiet_for_100ms(&rx), "reset 后不得把半截流补发出去");
    }

    /// 超过字符上限要提前冲出，避免单帧过大导致前端跳字。
    #[test]
    fn oversized_buffer_emits_early() {
        let (e, rx) = emitter();
        let n = COALESCE_MAX_CHARS + 10;
        e.feed(&"x".repeat(n));
        let ev = rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("超过上限必须立即发射，不等窗口");
        assert_eq!(ev.1["text"].as_str().unwrap().chars().count(), n);
        assert_eq!(e.pending_chars(), 0, "冲出后缓冲必须清空");
    }

    #[test]
    fn empty_delta_is_ignored() {
        let (e, rx) = emitter();
        e.feed("");
        e.flush();
        assert!(quiet_for_100ms(&rx), "空增量不得产生事件");
    }

    /// Clone 必须共享同一份缓冲——编排里 emitter 会被 clone 进回调闭包，
    #[test]
    fn clone_shares_one_buffer() {
        let (e, rx) = emitter();
        let e2 = e.clone();
        e.feed("A");
        e2.feed("B");
        assert_eq!(e.pending_chars(), 2, "clone 必须看到同一份缓冲");
        e2.flush();
        let ev = rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("clone 的 flush 必须能冲出另一个句柄喂的增量");
        assert_eq!(ev.1["text"], "AB");
    }

    /// 停表之后不得再有冲刷：ticker 退场了，滞留缓冲不该被它带出去。
    #[test]
    fn shutdown_stops_ticker_from_flushing() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        rt.block_on(async {
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(&'static str, Value)>();
            let e = StreamEmitter::new("test.stream", tx);
            e.arm_ticker();
            // 喂一点但**不 flush**，只靠 ticker 冲出（窗 250ms，给足 1s）
            e.feed("滞留");
            let got = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv()).await;
            assert!(
                got.is_ok(),
                "ticker 应在窗口到期后把滞留缓冲冲出去（否则本测试前提不成立）"
            );

            e.shutdown().await;
            assert!(e.is_shutdown(), "shutdown 后必须留下退场标记");
            e.feed("退场后");
            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(600), rx.recv())
                    .await
                    .is_err(),
                "停表后不得再有任何自动冲刷"
            );
        });
    }

    #[test]
    fn channel_closes_after_all_handles_shutdown() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        rt.block_on(async {
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(&'static str, Value)>();
            let e = StreamEmitter::new("test.stream", tx);
            e.arm_ticker();
            let e2 = e.clone();
            e2.arm_ticker();

            e.shutdown().await;
            e2.shutdown().await;
            drop(e);
            drop(e2);

            // 所有发送端都已析构 ⇒ recv 立刻返回 None（而不是永远 pending）
            let got = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv()).await;
            assert!(
                matches!(got, Ok(None)),
                "全部句柄退场析构后 channel 必须关闭，否则 forwarder 永久挂起（本轮事故）"
            );
        });
    }

    /// shutdown 幂等：重复调用、未装配时调用，都不得挂起或 panic。
    #[test]
    fn shutdown_is_idempotent_and_safe_without_ticker() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        rt.block_on(async {
            let (tx, _rx) = tokio::sync::mpsc::unbounded_channel::<(&'static str, Value)>();
            let e = StreamEmitter::new("test.stream", tx);
            // 未 arm 就 shutdown
            tokio::time::timeout(std::time::Duration::from_secs(1), e.shutdown())
                .await
                .expect("未装配 ticker 时 shutdown 必须立即返回");
            e.arm_ticker();
            tokio::time::timeout(std::time::Duration::from_secs(2), e.shutdown())
                .await
                .expect("shutdown 不得挂起");
            tokio::time::timeout(std::time::Duration::from_secs(2), e.shutdown())
                .await
                .expect("重复 shutdown 必须立即返回");
        });
    }
}
