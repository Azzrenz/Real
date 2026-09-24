// Session store: sidebar session CRUD + current selection + detail refresh.

import { create } from 'zustand';
import { sessionsApi } from '../../../services/domains/sessions';
import { useChatStore } from '../../chat/store/chatStore';
import type { Message, Session, SessionDetail } from '../../../services/contracts';
import { t } from '../../../shared/i18n';

let sessionsRetries = 0;

const detailInflight = new Map<string, Promise<void>>();

/** The open task is remembered for the lifetime of the window, so a page reload comes back on the */
const CURRENT_KEY = 'real.current-session';

function rememberCurrent(id: string | null): void {
  try {
    if (id) sessionStorage.setItem(CURRENT_KEY, id);
    else sessionStorage.removeItem(CURRENT_KEY);
  } catch {
    /* storage unavailable: the selection simply does not survive a reload */
  }
}

function recallCurrent(): string | null {
  try {
    return sessionStorage.getItem(CURRENT_KEY);
  } catch {
    return null;
  }
}

/** True while `currentId` holds a value recalled from storage and not yet checked against the task */
let currentIsRecalled = false;

interface SessionState {
  sessions: Session[];
  currentId: string | null;
  details: Record<string, SessionDetail>;
  loading: boolean;
  error: string | null;

  fetchSessions: () => Promise<void>;
  createSession: (title?: string, systemPrompt?: string) => Promise<Session>;
  selectSession: (id: string) => Promise<void>;
  /** Back to the start page. The one way to drop the selection, so the remembered id goes with it. */
  clearCurrent: () => void;
  refreshDetail: (opts?: { replay?: boolean; force?: boolean }) => Promise<void>;
  appendLocalMessage: (sessionId: string, content: string, atts?: unknown[]) => void;
  deleteSession: (id: string) => Promise<void>;
  renameSession: (id: string, title: string) => Promise<void>;

  setSessionModel: (id: string, model: string) => void;
  /** Page one batch further back and replay. Returns whether anything was fetched. */
  loadEarlierEvents: () => Promise<boolean>;
/** Set by the start page: the first line to send in a freshly created task. Pinned to a */
  pendingFirst: { id: string; text: string; atts?: unknown[] } | null;
  setPendingFirst: (v: { id: string; text: string; atts?: unknown[] } | null) => void;
}

const recalledId = recallCurrent();
currentIsRecalled = !!recalledId;

export const useSessionStore = create<SessionState>((set, get) => ({
  sessions: [],
  currentId: recalledId,
  details: {},
  loading: false,
  error: null,
  pendingFirst: null,

  setPendingFirst: (pendingFirst) => set({ pendingFirst }),

  fetchSessions: async () => {
    set({ loading: true, error: null });
    try {
      const res = await sessionsApi.list();
      const id = get().currentId;
      // A task restored after a reload may have been deleted while this window was away. Drop the
      // selection then, rather than leaving a dangling id for the caller to resolve into whichever
      // task happened to be touched last.
      const restoredGone = currentIsRecalled && !!id && !res.sessions.some((s) => s.id === id);
      currentIsRecalled = false;
      if (restoredGone) rememberCurrent(null);
      set({
        sessions: res.sessions,
        loading: false,
        ...(restoredGone ? { currentId: null } : {}),
      });
      sessionsRetries = 0;
    } catch (e) {
      set({ loading: false, error: (e as Error).message });
      if (sessionsRetries < 5) {
        sessionsRetries += 1;
        window.setTimeout(() => get().fetchSessions(), 3_000);
      }
    }
  },

  createSession: async (title = t('新任务'), systemPrompt = '') => {
    const res = await sessionsApi.create(title, systemPrompt);
    await get().fetchSessions();
    return res.session;
  },

  selectSession: async (id: string) => {
 // Detail fetch + replay live in ChatPanel's mount effect (single channel).
 // Doing it here too ran a second full replay racing the SSE stream.
    currentIsRecalled = false;
    rememberCurrent(id);
    set({ currentId: id });
  },

  clearCurrent: () => {
    currentIsRecalled = false;
    rememberCurrent(null);
    set({ currentId: null });
  },

  refreshDetail: async (opts?: { replay?: boolean; force?: boolean }) => {
    const id = get().currentId;
    if (!id) return;
    const inflight = detailInflight.get(id);
    if (inflight) {
      if (!opts?.force) return inflight;
      await inflight;
      return get().refreshDetail(opts);
    }
    const p = (async () => {
      try {
        const detail = await sessionsApi.detail(id, !!opts?.replay);
        // The title/area may just have changed server-side (auto-naming + area tagging on the
        // first file-changing round): sync both into the list, or the sidebar keeps showing
        // the stale ones.
        set((s) => ({
          details: { [id]: detail },
          sessions: s.sessions.map((x) =>
            x.id === id ? { ...x, title: detail.session.title, area: detail.session.area } : x
          ),
        }));
        if (opts?.replay) {
          if (detail.events && detail.events.length > 0) {
            useChatStore.getState().replayEvents(id, detail.events);
          } else {
            useChatStore.getState().replayFromMessages(id, detail.messages, detail.tool_calls);
          }
        }
      } catch (e) {
        set({ error: (e as Error).message });
      }
    })();
    detailInflight.set(id, p);
    try {
      await p;
    } finally {
      detailInflight.delete(id);
    }
  },

  setSessionModel: (id, model) => {
    const st = get();
    const d = st.details[id];
    set({
      sessions: st.sessions.map((s) => (s.id === id ? { ...s, model } : s)),
      details: d ? { ...st.details, [id]: { ...d, session: { ...d.session, model } } } : st.details,
    });
  },

  loadEarlierEvents: async () => {
    const id = get().currentId;
    if (!id) return false;
    const d = get().details[id];
    const loaded = d?.events ?? [];
    // Nothing older exists (already at the true start, or older events were dropped by
    // the event cap): retire the entry point instead of re-requesting on every scroll.
    if (!d || !d.events_has_earlier || loaded.length === 0) {
      if (d?.events_has_earlier) {
        set({ details: { ...get().details, [id]: { ...d, events_has_earlier: false } } });
      }
      return false;
    }
    const earliest = loaded[0]?.seq ?? 0;
    if (!earliest) return false;
    const r = await sessionsApi.events(id, earliest);
    const merged = [...r.events, ...loaded];
    set({
      details: {
        ...get().details,
        [id]: { ...d, events: merged, events_has_earlier: r.has_earlier },
      },
    });
    // Replay the whole loaded span: applyEvent only appends, so an earlier batch can
    // only land in front by re-running from the start. The span is bounded (one page
    // per request) and paging back is rare, so the cost stays acceptable.
    useChatStore.getState().replayEvents(id, merged);
    return r.events.length > 0;
  },

  appendLocalMessage: (sessionId: string, content: string, atts?: unknown[]) => {
    const d = get().details[sessionId];
    const base: SessionDetail = d ?? {
      session: {
        id: sessionId,
        title: '',
        status: 'idle',
        system_prompt: '',
        created_at: new Date().toISOString(),
        updated_at: new Date().toISOString(),
      },
      messages: [],
      tool_calls: [],
      plans: [],
      events: [],
    };
    const msg: Message = {
      id: `local-${Date.now()}`,
      session_id: sessionId,
      role: 'user',
      content,
      item_json: atts && atts.length > 0 ? JSON.stringify({ attachments: atts }) : null,
      created_at: new Date().toISOString(),
    };
    set({ details: { ...get().details, [sessionId]: { ...base, messages: [...base.messages, msg] } } });
  },

  deleteSession: async (id: string) => {
    await sessionsApi.remove(id);
    if (get().currentId === id) {
      get().clearCurrent();
    }
    const d = { ...get().details };
    delete d[id];
    set({ details: d });
    useChatStore.getState().removeSession(id);
    await get().fetchSessions();
  },

  renameSession: async (id: string, title: string) => {
    const t = title.trim();
    if (!t) return;
    await sessionsApi.rename(id, t);
    await get().fetchSessions();
  },
}));
