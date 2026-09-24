import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { RefObject } from 'react';
import type { Round } from '../store/chatStore';
import { useSessionStore } from '../../sessions/store/sessionStore';

// The panel keeps the newest RENDER_WINDOW items mounted; "load earlier" grows
const RENDER_WINDOW = 60;

// Scroll ownership of the chat body: sticky bottom, "pin the newest user bubble"
export function useChatScroll<TItem>({
  bodyRef,
  flowRef,
  items,
  live,
  lastUserMsgId,
}: {
  bodyRef: RefObject<HTMLDivElement | null>;
  flowRef: RefObject<HTMLDivElement | null>;
  items: TItem[];
  live: Round | null;
  lastUserMsgId: string;
}) {
  const stickRef = useRef(true);
  const pinHeldRef = useRef(false);

  const pinPadRef = useRef(0);
  const pinAtRef = useRef(0);
  // 「折叠保持」：思考窗口收起时内容会突然变矮。若视口正贴着底，浏览器随即把 scrollTop
  // 夹到新的最大值 —— 屏幕上已有的内容整体下坠一截，看到的就是"又跳回最底端了"。
  // 做法：把被收掉的高度补成底部空白，总高不变 ⇒ 视口原地不动；此后内容每长高一点就释放
  // 一点补白，新内容正好从视口下沿冒出来；补白放完时内容已长回折叠前的高度，此刻视口仍是
  // 贴底的，于是无缝交还给常规吸底 —— 全程没有一次位移。
  const foldHoldRef = useRef(0); // 还没释放掉的补白高度
  const foldBaseRef = useRef(0); // 折叠刚发生时的"自然高度"（不含补白）
  const clearPinPad = () => {
    foldHoldRef.current = 0;
    if (pinPadRef.current !== 0) {
      pinPadRef.current = 0;
      if (flowRef.current) flowRef.current.style.paddingBottom = '';
    }
  };

  useLayoutEffect(() => {
    if (!pendingPinRef.current || !lastUserMsgId) return;
    stickRef.current = false;
    const el = bodyRef.current;
    const target = el?.querySelector<HTMLElement>('.msg-block.user-block.pin-target');
    if (!el || !target) return;
    const c = el.getBoundingClientRect();
    const b = target.getBoundingClientRect();
    const bubbleDocY = el.scrollTop + (b.top - c.top);
    const targetScrollTop = bubbleDocY - 10;
    const maxScrollTop = el.scrollHeight - el.clientHeight;
    if (targetScrollTop > maxScrollTop && flowRef.current) {
      const need = targetScrollTop - maxScrollTop + 80;
      flowRef.current.style.paddingBottom = `${need}px`;
      pinPadRef.current = need;
    }
    el.scrollTop = targetScrollTop;
    pinAtRef.current = performance.now();
    if (!lastUserMsgId.startsWith('local-')) pendingPinRef.current = false;
  }, [lastUserMsgId]);

  const [expanded, setExpanded] = useState(0);
  const loadingEarlierRef = useRef(false);
  const pendingPinRef = useRef(false);
  const renderStart = Math.max(0, items.length - RENDER_WINDOW * (1 + expanded));
  // Mirrored into a ref: loadEarlier is a stable callback and would otherwise close
  // over the value from its first render.
  const renderStartRef = useRef(0);
  renderStartRef.current = renderStart;
  const expandedRef = useRef(0);
  expandedRef.current = expanded;
  const itemsLenRef = useRef(0);
  itemsLenRef.current = items.length;
  const visibleItems = renderStart > 0 ? items.slice(renderStart) : items;

/** Grow the render window until `idx` is mounted, so a jump target exists in the */
  const ensureIndexVisible = useCallback((idx: number) => {
    const pages = Math.ceil((itemsLenRef.current - idx) / RENDER_WINDOW) - 1;
    if (pages <= expandedRef.current) return true;
    setExpanded(pages);
    return false;
  }, []);

  const loadEarlier = useCallback(async () => {
    if (loadingEarlierRef.current) return;
    loadingEarlierRef.current = true;
    const prevH = bodyRef.current?.scrollHeight ?? 0;
    const keepAnchor = () => {
      requestAnimationFrame(() => {
        const el = bodyRef.current;
        if (el && el.scrollHeight !== prevH) el.scrollTop += el.scrollHeight - prevH;
      });
    };
    try {
      const st = useSessionStore.getState();
      const d = st.currentId ? st.details[st.currentId] : undefined;
      if (d?.events_has_earlier) {
        if (await st.loadEarlierEvents()) keepAnchor();
        return;
      }
      if (renderStartRef.current > 0) {
        setExpanded((s) => s + 1);
        keepAnchor();
      }
    } finally {
      loadingEarlierRef.current = false;
    }
  }, [bodyRef]);

  const [showJumpDown, setShowJumpDown] = useState(false);
  const onBodyScroll = () => {
    const el = bodyRef.current;
    if (!el) return;
    const dist = el.scrollHeight - el.scrollTop - el.clientHeight;
    const prog = performance.now() - lastProgScrollRef.current < 120;
    if (!prog) {
      const nearBottom = dist < 80;
      stickRef.current = nearBottom;
      if (nearBottom) {
        pinHeldRef.current = false;
        clearPinPad();
      }
    }
    setShowJumpDown(dist > 320);
    // Touching the top pages back once more. loadEarlier decides whether that means
    // growing the window or hitting the server - and owns the re-entry guard itself,
    // because this step can now involve a network round trip.
    if (el.scrollTop < 60 && !loadingEarlierRef.current) void loadEarlier();
  };
  const liveLen = useMemo(() => {
    if (!live) return 0;
    let n = (live.answer ?? '').length + live.nodes.length;
    for (const nd of live.nodes) {
      if (nd.type === 'think') n += (nd as { body?: string }).body?.length ?? 0;
      else if (nd.type === 'narration') n += (nd as { text?: string }).text?.length ?? 0;
    }
    return n;
  }, [live]);
  const lastProgScrollRef = useRef(0);
  const stickToBottom = () => {
    const el = bodyRef.current;
    if (!el) return;
    lastProgScrollRef.current = performance.now();
    el.scrollTop = el.scrollHeight;
  };
  const maybeTakeoverPin = () => {
    if (!pinHeldRef.current) return;
    if (performance.now() - pinAtRef.current < 1200) return;
    const el = bodyRef.current;
    if (!el) return;
    const units = el.querySelectorAll<HTMLElement>('.chat-flow .assistant-unit');
    const lastUnit = units && units.length > 0 ? units[units.length - 1] : null;
    if (!lastUnit) return;
    const vb = el.getBoundingClientRect();
    const ub = lastUnit.getBoundingClientRect();
    const remainBelow = vb.bottom - ub.bottom;
    if (remainBelow < 24) {
      pinHeldRef.current = false;
      pendingPinRef.current = false;
      stickRef.current = true;
      clearPinPad();
    }
  };
  useEffect(() => {
    maybeTakeoverPin();
    if (pinHeldRef.current && !live) {
      pinHeldRef.current = false;
      pendingPinRef.current = false;
      stickRef.current = true;
      clearPinPad();
    }
    if (!stickRef.current) return;
    stickToBottom();
    let guard = 0;
    const settle = () => {
      const el = bodyRef.current;
      if (!el || !stickRef.current) return;
      if (el.scrollHeight - el.scrollTop - el.clientHeight > 2 && guard++ < 5) {
        stickToBottom();
        requestAnimationFrame(settle);
      }
    };
    requestAnimationFrame(settle);
  }, [items.length, liveLen, !!live]);

  useEffect(() => {
    const flow = flowRef.current;
    const body = bodyRef.current;
    if (!flow || typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(() => {
      // 折叠补白随内容生长逐寸释放：总高不变 ⇒ 视口静止，新内容从下沿长出。
      // 放完（内容已长回折叠前的高度）就交还常规吸底 —— 此刻视口恰好仍是贴底的，无跳变。
      if (foldHoldRef.current > 0) {
        const f = flowRef.current;
        const b = bodyRef.current;
        if (f) {
          const natural = f.scrollHeight - foldHoldRef.current;
          const grew = Math.max(0, natural - foldBaseRef.current);
          const left = Math.max(0, foldHoldRef.current - grew);
          foldHoldRef.current = left;
          pinPadRef.current = left;
          f.style.paddingBottom = left > 0 ? `${left}px` : '';
          if (left === 0) {
            if (b) {
              // 内容已长回原高度，视口此刻仍是贴底的 —— 恢复吸底不会产生位移。
              stickRef.current = true;
              if (b.scrollHeight - b.scrollTop - b.clientHeight > 2) stickToBottom();
            }
          }
        }
      }
      maybeTakeoverPin();
      if (stickRef.current) stickToBottom();
    });
    ro.observe(flow);
    // The composer can be dragged taller, which shrinks this scroll viewport while the content
    // itself is unchanged - the flow observer above never fires, so the newest message slides
    // out of sight even though the user was pinned to the bottom. Watch the viewport as well.
    if (body) ro.observe(body);
    return () => ro.disconnect();
  }, []);

  // 思考块（DeepThinkRow）收起时派发的同步事件。补白顶住被收掉的高度，视口因此留在原处，
  // 不回落到底部；补白由上面的 ResizeObserver 随内容生长逐步释放。
  useEffect(() => {
    const onCollapse = () => {
      const el = bodyRef.current;
      const flow = flowRef.current;
      if (!el || !flow) return;
      // 用户已经离开底部在翻上面的记录：折叠不该打扰他，这里什么都不做。
      if (!stickRef.current && foldHoldRef.current === 0) return;
      const overflow = el.scrollTop + el.clientHeight - el.scrollHeight;
      if (overflow <= 0) return;
      const pad = pinPadRef.current + overflow;
      pinPadRef.current = pad;
      foldHoldRef.current = pad;
      flow.style.paddingBottom = `${pad}px`;
      foldBaseRef.current = flow.scrollHeight - pad;
      // 脱开吸底：否则下一次 ResizeObserver 会把这块补白当成"新底部"，把视口推到空白上。
      stickRef.current = false;
    };
    window.addEventListener('chat:think-collapsed', onCollapse);
    return () => window.removeEventListener('chat:think-collapsed', onCollapse);
  }, []);

  const resetScrollState = () => {
    setShowJumpDown(false);
    pinHeldRef.current = false;
    pendingPinRef.current = false;
    stickRef.current = true;
    clearPinPad();
  };

  const jumpToBottom = () => {
    pinHeldRef.current = false;
    stickRef.current = true;
    clearPinPad();
    const el = bodyRef.current;
    if (el) el.scrollTo({ top: el.scrollHeight, behavior: 'smooth' });
  };

  return {
    stickRef,
    pinHeldRef,
    pendingPinRef,
    clearPinPad,
    renderStart,
    visibleItems,
    loadEarlier,
    ensureIndexVisible,
    onBodyScroll,
    showJumpDown,
    resetScrollState,
    jumpToBottom,
  };
}
