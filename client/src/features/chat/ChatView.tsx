
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { chatApi } from '../../services/domains/chat';
import { eventsApi, SSE_BASE_URL } from '../../services/domains/events';
import { useChatStore, feedEvent, type Round } from './store/chatStore';
import { useSessionStore } from '../sessions/store/sessionStore';
import { useToastStore } from '../../shared/store/toastStore';
import { Icon } from '../../shared/ui/icons';
import { useT } from '../../shared/i18n';
import { SkillDialog } from '../../app/SkillDialog';
import { AssistantRound } from './components/AssistantRound';
import { Composer } from './components/Composer';
import { HeadDock } from './components/HeadDock';
import type { ChatIndexItem } from './components/ChatHistory';
import { UserBubbleText } from './components/UserBubbleText';
import {
  parseMessageAttachments,
  toPayload,
  type Attachment,
} from './lib/attachments';
import { useDraftSaver, readDraft, clearDraft, takeStaleDraft } from './hooks/useDraft';
import { useChatScroll } from './hooks/useChatScroll';
import { fmtTime } from '../../shared/lib/format';
import { CopyButton } from '../../shared/ui/CopyButton';
import type { Message, SseEvent } from '../../services/contracts';

interface Props {
  sessionId: string;
}

interface ChatItem {
  ts: string;
  kind: 'user' | 'round';
  msg?: Message;
  round?: Round;
  running?: boolean;
}

const EMPTY: never[] = [];

/* ---------- panel ---------- */


export function ChatView({ sessionId }: Props) {
  const detail = useSessionStore((s) => s.details[sessionId]);
  const refreshDetail = useSessionStore((s) => s.refreshDetail);
  const sessionTitle = useSessionStore((s) => {
    const hit = s.sessions.find((x) => x.id === sessionId);
    return hit?.title || '';
  });
  const t = useT();
  const [editingTitle, setEditingTitle] = useState(false);
  const sess = useChatStore((s) => s.sessions[sessionId]);
  const rounds = sess?.rounds ?? EMPTY;
  const live = sess?.live ?? null;
  const busy = !!live;

  const [input, setInput] = useState('');
  const [sending, setSending] = useState(false);
  const [skillsOpen, setSkillsOpen] = useState(false);
  const [attachments, setAttachments] = useState<Attachment[]>([]);
  const [pickedSkill, setPickedSkill] = useState<string[]>([]);
  const [phaseHint, setPhaseHint] = useState('正在准备…');
  const bodyRef = useRef<HTMLDivElement>(null);
  const flowRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const esRef = useRef<EventSource | null>(null);
  const lastEventAtRef = useRef(0);
  const reconnectTriesRef = useRef(0);
 // unmount guard: pending probe/reconnect timers must not spawn orphan streams
  const aliveRef = useRef(true);
  useEffect(() => () => { aliveRef.current = false; }, []);

  const startEvents = (sid: string, force = false) => {
    if (!aliveRef.current) return;
    if (!force && esRef.current && esRef.current.readyState !== EventSource.CLOSED) return;
    if (esRef.current) esRef.current.close();
    const lastSeq = useChatStore.getState().lastSeq[sid] ?? 0;
    const es = new EventSource(`${SSE_BASE_URL}${eventsApi.streamUrl(sid, lastSeq)}`);
    esRef.current = es;
    es.onmessage = (e) => {
      lastEventAtRef.current = Date.now();
      try {
        const ev = JSON.parse(e.data) as SseEvent;
        feedEvent(sid, ev);
      } catch {
 /* ignore bad frame */
      }
    };
    es.onerror = () => {
      es.close();
      if (esRef.current === es) esRef.current = null;
      const backoff = Math.min(30_000, 1_000 * 2 ** Math.min(reconnectTriesRef.current, 5));
      reconnectTriesRef.current += 1;
      window.setTimeout(() => {
        if (aliveRef.current) startEvents(sid, true);
      }, backoff);
    };
    es.onopen = () => {
      reconnectTriesRef.current = 0;
    };
  };
  const closeEvents = () => {
    esRef.current?.close();
    esRef.current = null;
  };

  useEffect(() => {
    let alive = true;
    let timer = 0;
    let waits = 0;
    const pollOnce = async () => {
      // 历史重建（refreshDetail）落地前**不要**轮询：那时 lastSeq 还是 0，
      // fetchIncremental 会把整个历史重放一遍，而这条路径 replaying=false ——
      // 一撞上历史里的 error 就把 archived 置真，applyEvent 入口随后吞掉**全部**
      // 后续事件（AI 正文随之消失，与上面 effect 是同一个坑的另一条路）。
      // 详情就绪后 lastSeq 已被回放推到最大 seq，轮询才是真正的"增量"。
      // 详情拿不到（接口失败）时最多等 6 秒就放行，别把轮询永久卡死。
      if (!useSessionStore.getState().details[sessionId] && waits < 20) {
        waits += 1;
        timer = window.setTimeout(pollOnce, 300);
        return;
      }
      const before = useChatStore.getState().lastSeq[sessionId] ?? 0;
      try {
        await useChatStore.getState().fetchIncremental(sessionId);
      } catch {
 /* transient: next tick retries */
      }
      if (!alive) return;
      const advanced = (useChatStore.getState().lastSeq[sessionId] ?? 0) > before;
      const st = useChatStore.getState().sessions[sessionId];
      const busyNow = !!st?.live;
      const es = esRef.current;
      const esOpen = !!es && es.readyState === EventSource.OPEN;
      if (busyNow && esOpen && advanced && Date.now() - lastEventAtRef.current > 15000) {
        closeEvents();
        startEvents(sessionId, true);
      }
      timer = window.setTimeout(pollOnce, busyNow ? 200 : (esOpen ? 15000 : 4000));
    };
    void pollOnce();
    return () => {
      alive = false;
      window.clearTimeout(timer);
    };
  }, [sessionId]);
 // Open the live stream AFTER state is rebuilt from detail (refreshDetail aligns
 // Open the live stream AFTER state is ready: on a local cache hit (rounds persisted
 // per session_id) we only fetch detail for messages (replay:false) and let SSE
 // no full 200-round re-replay. Without a cache: one-time full rebuild.
  useDraftSaver(sessionId, input, attachments);

  /** Set when a first line arrived from the start page, so the send effect knows to fire it. */
  const autoSendRef = useRef(false);

  useEffect(() => {
    resetScrollState();
    const hasRounds = (useChatStore.getState().sessions[sessionId]?.rounds?.length ?? 0) > 0;
    // 缓存命中但没有可信续点（lastSeq=0）时也必须走 replay 重建：否则 SSE 会从 0 全量
    // 重放，replaying=false，历史里第一条 error 就把 archived 置真、吞掉其后全部正文
    // （面板只剩 messages 表的用户气泡、AI 正文整段消失）。replay 走的 replayEvents
    // 有 replaying 保护，历史 error 不会污染 archived。
    const noResumePoint = (useChatStore.getState().lastSeq[sessionId] ?? 0) === 0;
    // 顺序在这里是**生死攸关**的：先重建历史，再开流（这段注释原本就这么写，实现反了）。
    //
    // 反过来时会发生什么（实测，会话 87485b26）：
    //   重启 ⇒ lastSeq=0 ⇒ SSE 从 0 重放整个历史，而重放走的**不是** replayEvents，
    //   于是 replaying=false ⇒ 一撞上历史里的 error/cancelled，session 处理器就把
    //   archived[sessionId] 置真，而 applyEvent 入口那道闸门会把它之后的**所有**事件
    //   全部 return 掉。该会话首条 error (#612040) 之后还有 145791 条事件（97.2%），
    //   AI 正文全在里面 ⇒ 面板上只剩用户气泡（来自 messages 表，独立渲染），回复整段消失。
    //
    // 修正后：refreshDetail 先跑完（replayEvents 会把 replaying 置真，error 不再污染
    // archived，并把 lastSeq 推到回放到的最大 seq），随后 SSE 从 lastSeq 起连 —— 只收增量。
    void useSessionStore
      .getState()
      .refreshDetail({ replay: !hasRounds || noResumePoint })
      .finally(() => {
        if (aliveRef.current) startEvents(sessionId, true);
      });
    // Start-page hand-off outranks the stored draft, and BOTH writes live here on purpose:
    // split across two effects, whichever ran last won — and when the draft write ran last it
    // erased the first line, the send never fired, and the hand-off died silently.
    // The pendingFirst effect further down is NOT a duplicate: it covers the case where the
    // value lands after this mount. Do not collapse them into one.
    const first = useSessionStore.getState().pendingFirst;
    if (first && first.id === sessionId) {
      useSessionStore.getState().setPendingFirst(null);
      autoSendRef.current = true;
      setInput(first.text);
      // Attachments handed off from the start page ride along; drop this line and a file dragged
      // onto the start page is erased the moment the task mounts and the hand-off is half-lost.
      setAttachments(Array.isArray(first.atts) ? (first.atts as Attachment[]) : []);
    } else {
      const d = readDraft(sessionId);
      setInput(d?.text ?? '');
      setAttachments(Array.isArray(d?.atts) ? d.atts : []);
    }
    return () => {
      closeEvents();
    };
  }, [sessionId]);

  useEffect(() => {
    const users = (detail?.messages ?? []).filter((m) => m.role === 'user');
    const lastUser = users.length > 0 ? users[users.length - 1] : undefined;
    if (!lastUser?.created_at) return;
    const t = Date.parse(lastUser.created_at);
    if (!Number.isFinite(t)) return;
    const d = takeStaleDraft(sessionId, t);
    if (!d) return;
    setInput((cur) => (cur === (d.text ?? '') ? '' : cur));
    setAttachments((cur) =>
      cur.length > 0 && (d.atts?.length ?? 0) > 0 && cur[0]?.dataUrl === d.atts[0]?.dataUrl
        ? []
        : cur,
    );
  }, [detail, sessionId]);

  /** 任务运行中带附件的那条消息：暂存在这里，busy 落回后自动发出（见下方 effect）。 */
  const queuedSendRef = useRef<{ text: string; atts: Attachment[] } | null>(null);

  const submitChat = async (content: string, atts: Attachment[] = []) => {
    setSending(true);
    let runId = '';
    try {
      const res = await chatApi.send(sessionId, content, toPayload(atts));
      runId = typeof res?.run_id === 'string' ? res.run_id : '';
    } catch (e) {
      useToastStore.getState().show((e as Error).message, 'error');
      setSending(false);
      return;
    }
    useSessionStore.getState().appendLocalMessage(
      sessionId,
      content,
      atts && atts.length > 0 ? (atts as unknown[]) : undefined,
    );
    // run identity comes back with the send response: this round knows its own run's
    // events from birth (no foreign terminal can settle it)
    useChatStore.getState().beginRun(sessionId, runId);
    closeEvents();
    startEvents(sessionId);
    void refreshDetail({ force: true });
    pendingPinRef.current = true;
    pinHeldRef.current = true;
    stickRef.current = false;
    setSending(false);
  };

  const send = async () => {
    const raw = input.trim();
    const prefix = pickedSkill.map((n) => `/${n}`).join(' ');
    const text = prefix ? `${prefix} ${raw}`.trim() : raw;
    if ((!raw && attachments.length === 0 && pickedSkill.length === 0) || sending) return;
    if (busy) {
      // 插话通道的入参里**没有附件**（`chatApi.interject` 只吃文本），
      // 所以带附件的消息绝不能走进来——走过去就是"输入框清空、图无声消失"
      // （2026-09-22 实测事故：用户带图发送，界面像发出去了，模型全程没收到）。
      // 这里明确拦住并**保留输入框与附件**，等本轮结束后原样再发即可。
      if (attachments.length > 0) {
        // 任务运行中带附件：**不再硬拦**（2026-09-23 修）。
        // 旧行为是弹一句"插话只能带文字"然后什么都不做 —— 对用户的感受就是
        // "带图的消息永远发不出去，只能发文字"（连续两次反馈的现场）。
        // 现在连文带图一起**排队**，本轮一结束自动原样发出（见下方 queuedSend effect）。
        // 先接管输入框：消息已存进队列，留着草稿只会让人以为没发出去而重复发送。
        queuedSendRef.current = { text, atts: attachments };
        setInput('');
        setAttachments([]);
        setPickedSkill([]);
        clearDraft(sessionId);
        useToastStore.getState().show(
          t('任务运行中：这条带 {n} 个附件的消息已排队，本轮一结束自动发送', {
            n: attachments.length,
          }),
          'ok',
        );
        return;
      }
      if (!text) {
        useToastStore.getState().show(t('任务运行中：文字插话才能入队（附件请等本轮结束再发）'), 'error');
        return;
      }
      try {
        await chatApi.interject(sessionId, text, 'append');
        useToastStore.getState().show(t('插话已入队：下一轮边界生效，气泡将出现在插入点'), 'ok');
        setInput('');
      } catch (e) {
        useToastStore.getState().show((e as Error).message, 'error');
      }
      return;
    }
    const atts = attachments;
    setInput('');
    setAttachments([]);
    setPickedSkill([]);
    clearDraft(sessionId);
    setPhaseHint(atts.length > 0 ? '正在读取附件…' : '正在准备…');
    await submitChat(text, atts);
  };

  // 排队发送：任务运行期间带附件的那条消息，等 busy 落回 false 后自动原样发出。
  // 只在"闲下来且真的排着队"时动手，取走即清空 —— 重复渲染不会重复发送。
  const submitChatRef = useRef(submitChat);
  submitChatRef.current = submitChat;
  useEffect(() => {
    if (busy || sending) return;
    const q = queuedSendRef.current;
    if (!q) return;
    queuedSendRef.current = null;
    void submitChatRef.current(q.text, q.atts);
  }, [busy, sending]);

 // Real regeneration: resend the latest user message as a new round.
  const regenerate = async () => {
    if (sending || busy) return;
    const msgs = useSessionStore.getState().details[sessionId]?.messages ?? [];
    const lastUser = [...msgs].reverse().find((m) => m.role === 'user');
    if (!lastUser?.content) {
      useToastStore.getState().show(t('没有可重新生成的消息'), 'info');
      return;
    }
    setPhaseHint('正在准备…');
    await submitChat(lastUser.content);
  };
  const regenerateRef = useRef(regenerate);
  regenerateRef.current = regenerate;
  const stableRegenerate = useCallback(() => regenerateRef.current(), []);

  const cancel = async () => {
    useChatStore.getState().archiveLive(sessionId, 'cancelled');
    pinHeldRef.current = false;
    pendingPinRef.current = false;
    stickRef.current = true;
    clearPinPad();
    requestAnimationFrame(() => {
      const el = bodyRef.current;
      if (el) el.scrollTop = el.scrollHeight;
    });
    closeEvents();
    try {
      await chatApi.cancel(sessionId);
    } catch {
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    // IME guard: while a Chinese/Japanese candidate window is open, Enter picks a candidate
    // and must not send — at that moment the typed text has not reached `input` yet, so
    // sending would silently no-op and look like the send button is dead. React's synthetic
    // event does not carry isComposing reliably, hence the native check plus the legacy keyCode.
    if (e.nativeEvent.isComposing || e.keyCode === 229) return;
 // Enter sends, Shift+Enter inserts a newline (multi-line input is the point of a 3-row textarea)
    if (e.key === 'Enter' && !e.shiftKey && !e.ctrlKey && !e.metaKey) {
      e.preventDefault();
      void send();
    }
  };

  const bubbleTexts = useMemo(() => {
    const s = new Set<string>();
    for (const r of (live ? [...rounds, live] : rounds)) {
      for (const n of r.nodes) {
        if (n.type === 'user_bubble' && n.text.trim()) s.add(n.text.trim());
      }
    }
    return s;
  }, [rounds, live]);

  // ★ 跨 Agent 消息（A2A）：正文常被“系统包装”裹住（【本轮用户输入】+ 项目上下文/长期记忆/…）。
  //   气泡只该显示信封 + 真正的人话 —— 否则一坠 2000+ 字符的提示噪音，看不出是谁在说话。
  const a2aOf = (content: string) => {
    const i = content.indexOf('【A2A ');
    if (i < 0) return null;
    const s = content.slice(i);
    const m = /^【A2A ([^】]+)】\s*(\S+)\s*→\s*(\S+)\s*·\s*([^·]+?)\s*·\s*session=(\S*)/.exec(s);
    if (!m) return null;
    let body = s.slice(m[0].length);
    const cut = body.search(/\n?【(本轮用户输入|项目上下文|长期记忆|目标锚|历史任务记忆|最近对话脉络|工作日志与长期记忆|可用技能|常用脚本库|记忆使用指引|用户偏好|用户凭证)/);
    if (cut >= 0) body = body.slice(0, cut);
    const t = body.trim();
    // ★ 剥完变成空 ⇒ 信封后面直接跟的是系统提示（本侧落库把系统包装拼在信封前/后）。
    //   此时绝不能显空气泡 —— 回退 null，让渲染层显原文（宁可露出信头，也不能把话丢了）。
    if (!t) return null;
    return { id: m[1], from: m[2], to: m[3], at: m[4].trim(), body: t };
  };

  // ★ 系统注入包装：真人的那句后面跟了整坨上下文（项目上下文/长期记忆/最近对话脉络/工作日志…）。
  //   落在消息里就是一条 5000+ 字符的 user 消息 —— 重启一次就多一个巨气泡，里面还打包着前几轮对话。
  //   气泡只显真话：取《本轮用户输入》之后、下一个【系统段】之前。
  const localUserOf = (content: string) => {
    const t = content ?? '';
    const MARK = '【本轮用户输入】';
    const i = t.indexOf(MARK);
    if (i < 0) return t;                       // 不是包装消息 → 原样
    let body = t.slice(i + MARK.length);
    const cut = body.search(/\n?【(项目上下文|长期记忆|目标锚|历史任务记忆|最近对话脉络|工作日志与长期记忆|可用技能|常用脚本库|记忆使用指引|用户偏好|用户凭证|钩子命中的历史主题|此前相关回合回灌|对话主题|本轮用户输入)/);
    if (cut >= 0) body = body.slice(0, cut);
    return body.trim() || t;
  };

  const userMsgs = useMemo<Message[]>(() => {
    const out: Message[] = [];
    for (const m of detail?.messages ?? []) {
      if (m.role !== 'user') continue;
      let isInterjection = false;
      let isCancelled = false;
      try {
        const o = JSON.parse(m.item_json ?? '{}') as { interjection?: unknown; cancelled?: unknown };
        isInterjection = o.interjection === true;
        isCancelled = o.cancelled === true;
      } catch {
      }
      if (isCancelled) continue;
      if (!isInterjection || !bubbleTexts.has((m.content ?? '').trim())) out.push(m);
    }
    return out;
  }, [detail, bubbleTexts]);
  const userMsgPos = useMemo(() => {
    const map = new Map<string, number>();
    userMsgs.forEach((m, i) => map.set(m.id, i));
    return map;
  }, [userMsgs]);

  const lastUserMsgId = userMsgs.length > 0 ? userMsgs[userMsgs.length - 1].id : '';
  const attachByMsg = useMemo(() => {
    const map: Record<string, Attachment[]> = {};
    for (const m of userMsgs) {
      const list = parseMessageAttachments(m.item_json);
      if (list.length > 0) map[m.id] = list;
    }
    return map;
  }, [userMsgs]);
  const items: ChatItem[] = useMemo(() => {
    const arr: ChatItem[] = [];
    const liveItem: ChatItem | null = live
      ? { ts: live.startedAt, kind: 'round' as const, round: live, running: true }
      : null;
    if (!live && rounds.length === userMsgs.length && rounds.length > 0) {
      for (let k = 0; k < userMsgs.length; k++) {
        arr.push({ ts: userMsgs[k].created_at, kind: 'user' as const, msg: userMsgs[k] });
        arr.push({ ts: rounds[k].startedAt, kind: 'round' as const, round: rounds[k], running: false });
      }
      return arr;
    }
    let i = 0;
    let j = 0;
    let lastUserTs = '';
    while (i < userMsgs.length && j < rounds.length) {
      const uTs = userMsgs[i].created_at;
      const raw = rounds[j].startedAt;
      const rTs = lastUserTs && raw < lastUserTs ? lastUserTs : raw;
      if (uTs <= rTs) {
        arr.push({ ts: uTs, kind: 'user' as const, msg: userMsgs[i] });
        lastUserTs = uTs;
        i += 1;
      } else {
        arr.push({ ts: raw, kind: 'round' as const, round: rounds[j], running: false });
        j += 1;
      }
    }
    for (; i < userMsgs.length; i += 1) {
      arr.push({ ts: userMsgs[i].created_at, kind: 'user' as const, msg: userMsgs[i] });
      lastUserTs = userMsgs[i].created_at;
    }
    for (; j < rounds.length; j += 1) arr.push({ ts: rounds[j].startedAt, kind: 'round' as const, round: rounds[j], running: false });
    if (liveItem) arr.push(liveItem);
    // ★ 隔离（2026-09-16）：跨 Agent（A2A）投递进来的消息不属于「任务」。
    //   它只是唤醒对方模型的一条通道，落到本会话里不该以任务气泡出现 —— 整轮隐藏（连带它触发的那一轮回复）。
    const visible: ChatItem[] = [];
    for (let k = 0; k < arr.length; k += 1) {
      const it = arr[k];
      if (it.kind === 'user' && a2aOf(it.msg?.content ?? '')) {
        if (k + 1 < arr.length && arr[k + 1].kind === 'round') k += 1;
        continue;
      }
      visible.push(it);
    }
    return visible;
  }, [userMsgs, rounds, live]);

  // Mirrored into a ref: jumpToMessage retries across renders, where the memoised
  // items array would still be the one captured when the jump started.
  const itemsRef = useRef<ChatItem[]>([]);
  itemsRef.current = items;

  const {
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
  } = useChatScroll({ bodyRef, flowRef, items, live, lastUserMsgId });

  // Start-page hand-off: the session already exists by the time this mounts, so the
  // first line travels the ordinary send path. Pinned to this session id — a value left
  // over from a failed launch must not fire inside whichever task the user opens next.
  // Covers the case where the start page's first line lands AFTER this view mounted. An effect
  // keyed on sessionId alone would run before the value arrives and then never run again, so
  // subscribe to the value itself. The effect near the top is NOT a duplicate of this one: it
  // handles the value already being present at mount time. Do not collapse the two.
  const pendingFirst = useSessionStore((s) => s.pendingFirst);
  useEffect(() => {
    if (!pendingFirst || pendingFirst.id !== sessionId) return;
    useSessionStore.getState().setPendingFirst(null);
    autoSendRef.current = true;
    setInput(pendingFirst.text);
    // Same hand-off as the mount branch above: attachments must land with the text, or only half
    // of the message arrives.
    setAttachments(Array.isArray(pendingFirst.atts) ? (pendingFirst.atts as Attachment[]) : []);
  }, [sessionId, pendingFirst]);

  useEffect(() => {
    // Plain text OR attachments-only must both auto-send: keying on input.trim() alone leaves a
    // dragged-in image with no text stuck in the box forever.
    if (!autoSendRef.current) return;
    if (!input.trim() && attachments.length === 0) return;
    autoSendRef.current = false;
    void send();
  }, [input, attachments]);

  const chatIndex: ChatIndexItem[] = useMemo(
    () =>
      userMsgs
        .filter((m) => !a2aOf(m.content ?? ''))   // ★ 隔离：A2A 消息不进任务侧的跳转索引
        .map((m) => ({
          id: m.id,
          text: (m.content || '').trim() || t('（提问）'),
          time: m.created_at,
        })),
    [userMsgs, t],
  );

  /** Jump to one of the user's own questions. Two things can be missing over there: the
   *  round behind it (older events not fetched yet) and the node itself (outside the
   *  render window). Pull the data first, then grow the window, then scroll. */
  const jumpToMessage = (msgId: string) => {
    const target = chatIndex.find((c) => c.id === msgId);
    if (!target) return;
    // Detach from sticky-bottom, or the stream keeps yanking the view back down.
    stickRef.current = false;
    let tries = 0;
    const attempt = () => {
      const list = itemsRef.current;
      let firstRoundAt = '';
      for (const it of list) {
        if (it.kind === 'round' && it.round) {
          firstRoundAt = it.round.startedAt;
          break;
        }
      }
      if (firstRoundAt && target.time < firstRoundAt && tries < 12) {
        tries += 1;
        const st = useSessionStore.getState();
        const d = st.currentId ? st.details[st.currentId] : undefined;
        if (d?.events_has_earlier) {
          void st.loadEarlierEvents().then(() => requestAnimationFrame(attempt));
          return;
        }
      }
      const idx = list.findIndex((it) => it.kind === 'user' && it.msg?.id === msgId);
      if (idx < 0) return;
      if (!ensureIndexVisible(idx)) {
        requestAnimationFrame(attempt);
        return;
      }
      const el = bodyRef.current?.querySelector<HTMLElement>(`[data-msg="${msgId}"]`);
      if (el) el.scrollIntoView({ behavior: 'smooth', block: 'start' });
    };
    requestAnimationFrame(attempt);
  };

  return (
    <div className="chat">
      <div className="chat-head">
        {editingTitle ? (
          <input
            className="chat-title-input"
            defaultValue={sessionTitle}
            autoFocus
            onFocus={(e) => e.currentTarget.select()}
            onBlur={() => setEditingTitle(false)}
            onKeyDown={(e) => {
              if (e.nativeEvent.isComposing || e.keyCode === 229) return;
              if (e.key === 'Enter') {
                const v = e.currentTarget.value.trim();
                if (v && v !== sessionTitle) void useSessionStore.getState().renameSession(sessionId, v);
                setEditingTitle(false);
              }
              if (e.key === 'Escape') setEditingTitle(false);
            }}
            onClick={(e) => e.stopPropagation()}
          />
        ) : (
          <span
            className="chat-title"
            title={sessionTitle}
            onClick={() => setEditingTitle(true)}
          >
            {sessionTitle || t('新任务')}
          </span>
        )}
        <span className="spacer" />
        <HeadDock chatIndex={chatIndex} onJumpChat={jumpToMessage} />
      </div>

      <div className="chat-body" ref={bodyRef} onScroll={onBodyScroll}>
        <div className="chat-flow" ref={flowRef}>
          {!detail && (
            <div className="chat-restore" role="status">
              <Icon name="refresh" size={16} className="chat-restore-icon" />
              {t('正在恢复会话…')}
            </div>
          )}
          {renderStart > 0 && (
            <button
              className="load-earlier"
              onClick={loadEarlier}
              aria-label={t('加载更早记录')}
            >
              {t('↑ 更早记录（{n} 条）· 点击或滚到顶部加载', { n: renderStart })}
            </button>
          )}
          {visibleItems.map((it, idx) => {
            if (it.kind === 'user') {
              const m = it.msg!;
              const isNewestUser = m.id === lastUserMsgId;
              const uKey = userMsgPos.get(m.id) ?? `fallback-${m.id}`;
              const a2a = a2aOf(m.content ?? '');
              return (
                <div
                  key={`u-${uKey}`}
                  data-msg={m.id}
                  className={`msg-block ${a2a ? 'a2a-block' : 'user-block'}${isNewestUser ? ' pin-target' : ''}`}
                >
                  {a2a ? (
                    <>
                      <div
                        className="msg-a2a-from"
                        style={{ fontSize: '11px', color: 'var(--dim, #8a8a8a)', margin: '0 0 3px 2px', letterSpacing: '0.3px' }}
                      >
                        来自 {a2a.from} · {a2a.id}
                      </div>
                      <div
                        className="msg msg-agent"
                        style={{
                          alignSelf: 'flex-start',
                          background: 'var(--panel2, #26282c)',
                          border: '1px solid var(--line2, #33363b)',
                          borderRadius: '4px',
                          padding: '6px 9px',
                          maxWidth: '88%',
                        }}
                      >
                        <UserBubbleText text={a2a.body || (m.content ?? '')} />
                      </div>
                    </>
                  ) : (
                    <div className="msg msg-user"><UserBubbleText text={localUserOf(m.content)} /></div>
                  )}
                  {(attachByMsg[m.id]?.length ?? 0) > 0 && (
                    <div className="msg-attach-row">
                      {attachByMsg[m.id].map((a) => (
                        <span className="msg-attach-chip" key={a.id} title={a.name}>
                          {a.kind === 'image' && a.dataUrl ? (
                            <img className="msg-attach-img" src={a.dataUrl} alt={a.name} />
                          ) : (
                            <span className="msg-attach-file">
                              <Icon name="doc" size={13} />
                            </span>
                          )}
                          <span className="attach-name">{a.name}</span>
                        </span>
                      ))}
                    </div>
                  )}
                  <div className="msg-actions">
                    <CopyButton text={m.content} className="msg-action" size={14} />
                    <button
                      className="msg-action"
                      onClick={() => {
                        setInput(m.content);
                        setAttachments(parseMessageAttachments(m.item_json));
                        requestAnimationFrame(() => inputRef.current?.focus());
                      }}
                      title={t('编辑（载入输入框）')}
                      aria-label={t('编辑')}
                    >
                      <Icon name="pencil" size={14} />
                    </button>
                    <span className="msg-time">{fmtTime(m.created_at)}</span>
                  </div>
                </div>
              );
            }
            const r = it.round;
            if (!r) return null;
            return (
              <AssistantRound
                key={`r-${r.seq ?? idx}`}
                round={r}
                running={it.kind === 'round' && !!it.running}
                live={r === live}
                phaseHint={phaseHint}
                sessionId={sessionId}
                onRegen={stableRegenerate}
              />
            );
          })}
        </div>
      </div>

      {skillsOpen && <SkillDialog onClose={() => setSkillsOpen(false)} />}
      <Composer
        value={input}
        onChange={setInput}
        attachments={attachments}
        onAttachmentsChange={setAttachments}
        onSend={() => {
          void send();
        }}
        onCancel={() => void cancel()}
        onKeyDown={handleKeyDown}
        inputRef={inputRef}
        busy={busy}
        sending={sending}
        showJumpDown={showJumpDown}
        onManageSkills={() => setSkillsOpen(true)}
        pickedSkill={pickedSkill}
        onPickSkill={(n) => setPickedSkill((prev) => (prev.includes(n) ? prev : [...prev, n]))}
        onRemoveSkill={(n) => setPickedSkill((prev) => prev.filter((x) => x !== n))}
        onJumpToBottom={jumpToBottom}
      />
    </div>
  );
}
