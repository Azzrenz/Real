
import type { ChatStoreState, Round, SessionChat } from '../types';


export function deriveResumePoints(data: {
  lastSeq?: Record<string, number>;
  terminalSeq?: Record<string, number>;
}): Record<string, number> {
  const out: Record<string, number> = {};
  const keys = new Set([...Object.keys(data.lastSeq ?? {}), ...Object.keys(data.terminalSeq ?? {})]);
  for (const k of keys) out[k] = Math.max(data.lastSeq?.[k] ?? 0, data.terminalSeq?.[k] ?? 0);
  return out;
}

interface StoreLike {
  getState: () => ChatStoreState;
  setState: (partial: Partial<ChatStoreState>) => void;
  subscribe: (fn: () => void) => () => void;
}

export function initPersistence(store: StoreLike) {
const CACHE_KEY = 'real-chat-cache';
const CACHE_MAX_SESSIONS = 15;
const CACHE_FLUSH_MS = 500;

function lastRoundTs(rounds: Round[]): number {
  let t = 0;
  for (const r of rounds) {
    const rt = r.endedAt ? new Date(r.endedAt).getTime() : r.startedAt ? new Date(r.startedAt).getTime() : 0;
    if (rt > t) t = rt;
  }
  return t;
}

function serializeCache(state: ChatStoreState): string {
  const sessions: Record<string, SessionChat> = {};
  for (const [sid, sc] of Object.entries(state.sessions)) {
    sessions[sid] = { rounds: sc.rounds, live: null };
  }
  const entries = Object.entries(sessions);
  if (entries.length > CACHE_MAX_SESSIONS) {
    entries.sort((a, b) => lastRoundTs(b[1].rounds) - lastRoundTs(a[1].rounds));
    for (const [sid] of entries.slice(CACHE_MAX_SESSIONS)) delete sessions[sid];
  }
  return JSON.stringify({
    state: { sessions, lastSeq: state.lastSeq, terminalSeq: state.terminalSeq },
    version: 1,
  });
}

let cacheTimer: ReturnType<typeof setTimeout> | null = null;
function flushCache() {
  cacheTimer = null;
  if (typeof localStorage === 'undefined') return;
  const hasLive = Object.values(store.getState().sessions).some((s) => s.live != null);
  if (hasLive) {
    scheduleCacheWrite();
    return;
  }
  try {
    localStorage.setItem(CACHE_KEY, serializeCache(store.getState()));
  } catch {
 /* overflow / quota: skip cache write */
  }
}
function scheduleCacheWrite() {
  if (cacheTimer != null) clearTimeout(cacheTimer);
  cacheTimer = setTimeout(flushCache, CACHE_FLUSH_MS);
}

function renumberRounds(rounds: Round[]): Round[] {
  return rounds.map((r, k) => (r.seq === k + 1 ? r : { ...r, seq: k + 1 }));
}

function hydrateCache() {
  if (typeof localStorage === 'undefined') return;
  let raw: string | null = null;
  try {
    raw = localStorage.getItem(CACHE_KEY);
  } catch {
    return;
  }
  if (!raw) return;
  try {
    const parsed = JSON.parse(raw) as {
      state?: { sessions?: Record<string, SessionChat>; lastSeq?: Record<string, number>; terminalSeq?: Record<string, number> };
      sessions?: Record<string, SessionChat>;
      lastSeq?: Record<string, number>;
      terminalSeq?: Record<string, number>;
    };
    const data = parsed.state ?? parsed;
    const sessions: Record<string, SessionChat> = {};
    for (const [sid, sc] of Object.entries(data.sessions ?? {})) {
      if (!sc || !Array.isArray(sc.rounds)) continue;
      sessions[sid] = { rounds: renumberRounds(sc.rounds), live: null };
    }
    store.setState({
      sessions,
      lastSeq: deriveResumePoints(data),
      terminalSeq: data.terminalSeq ?? {},
    });
  } catch {
 /* corrupt cache: ignore, rebuild from server */
  }
}
hydrateCache();
store.subscribe(() => scheduleCacheWrite());
if (typeof window !== 'undefined') {
  window.addEventListener('pagehide', flushCache);
  window.addEventListener('beforeunload', flushCache);
}
}
