
import { create } from 'zustand';
import { sessionsApi } from '../../../services/domains/sessions';
import { eventsApi } from '../../../services/domains/events';
import { chatApi } from '../../../services/domains/chat';
import type { ChatNode, ChatStoreState, Round, SessionChat, ThinkRow } from '../types';
import { narTail } from '../narration/pipeline';
import { EMPTY_SESSION, emptyRound, finalizeNodes, nextNarrationId } from '../lib/nodeFactory';
import { createApplyEvent } from './applyEvent';
import { createReplay } from './replay';
import { initPersistence } from './persist';
import { bindStreamCtx } from './runtime';

export const useChatStore = create<ChatStoreState>()((set, get) => {
  const applyEvent = createApplyEvent(set, get);
  const { replayEvents, replayFromMessages } = createReplay(set, get);
  return {
    sessions: {},
    lastSeq: {},
    seenSeq: {},
    terminalSeq: {},
    archived: {},
    replaying: {},
    confirm: null,

    applyEvent,
    replayEvents,
    replayFromMessages,

  archiveLive: (sessionId, status = 'cancelled') => {
    const st = get().sessions[sessionId] ?? EMPTY_SESSION;
    if (!st.live) {
      set({
        sessions: { ...get().sessions, [sessionId]: { ...st, live: null } },
      });
      return;
    }
    const tBuf = narTail[sessionId];
    let liveNodes = st.live.nodes;
    if (tBuf) {
      const ns = liveNodes.slice();
      const last = ns[ns.length - 1];
      if (last && last.type === 'narration') ns[ns.length - 1] = { ...last, text: last.text + tBuf };
      else ns.push({ type: 'narration', id: nextNarrationId(), text: tBuf });
      liveNodes = ns;
      delete narTail[sessionId];
    }
    const live: Round = {
      ...st.live,
      status,
      nodes: finalizeNodes(
        liveNodes,
        status === 'completed'
          ? 'success'
          : status === 'cancelled' || status === 'interrupted'
            ? 'cancelled'
            : 'error',
      ),
    };
    set({
      sessions: {
        ...get().sessions,
        [sessionId]: { rounds: [...st.rounds, live], live: null },
      },
      terminalSeq: { ...get().terminalSeq, [sessionId]: get().lastSeq[sessionId] ?? 0 },
      archived: { ...get().archived, [sessionId]: true },
    });
  },

  beginRun: (sessionId, runId) => {
 // fresh run from the user: lift the cancel boundary + archived flag
    const archived = { ...get().archived };
    delete archived[sessionId];
    const terminalSeq = { ...get().terminalSeq };
    delete terminalSeq[sessionId];
    const st = get().sessions[sessionId];
    const patch: Partial<SessionChat> = {};
    if (!st?.live) {
      const seqBase = Math.max(
        get().lastSeq[sessionId] ?? 0,
        ...st.rounds.map((r) => r.seq ?? 0),
      );
      // bound to that run from birth (run_id from the send response): no foreign
      // terminal event can ever land on this round
      const live = emptyRound(seqBase + 1, new Date().toISOString());
      patch.live = runId ? { ...live, runId } : live;
    }
    set({
      archived,
      terminalSeq,
      ...(Object.keys(patch).length
        ? { sessions: { ...get().sessions, [sessionId]: { ...(st ?? EMPTY_SESSION), ...patch } } }
        : {}),
    });
  },

  fetchIncremental: async (sessionId) => {
    const after = get().lastSeq[sessionId] ?? 0;
    const events = await eventsApi.poll(sessionId, after);
    // delivery only: round terminal state has ONE owner (applySessionEvent).
    // A second archiver used to run here ("last event of the batch is terminal"),
    // racing the handler and killing fresh rounds on late foreign terminals. Removed.
    for (const ev of events) get().applyEvent(sessionId, ev);
  },

  loadThinking: async (sessionId, thinkId) => {
    const st = get().sessions[sessionId];
    if (!st) return;
    const allNodes: ChatNode[] = [
      ...(st.live ? st.live.nodes : []),
      ...st.rounds.flatMap((r) => r.nodes),
    ];
    const hit = allNodes.find((n) => n.type === 'think' && n.id === thinkId);
    if (!hit || hit.type !== 'think' || !hit.lazy || hit.loading) return;
    const { from, to } = hit.lazy;
    const updateThink = (updater: (t: ThinkRow) => ThinkRow) => {
      const cur = get().sessions[sessionId];
      if (!cur) return;
      const patchNodes = (nodes: ChatNode[]): ChatNode[] | null => {
        let changed = false;
        const next = nodes.map((n) => {
          if (n.type === 'think' && n.id === thinkId) {
            changed = true;
            return updater(n);
          }
          return n;
        });
        return changed ? next : null;
      };
      if (cur.live) {
        const nodes = patchNodes(cur.live.nodes);
        if (nodes) {
          set({
            sessions: {
              ...get().sessions,
              [sessionId]: { ...cur, live: { ...cur.live, nodes } },
            },
          });
          return;
        }
      }
      const rounds = cur.rounds.map((r) => {
        const nodes = patchNodes(r.nodes);
        return nodes ? { ...r, nodes } : r;
      });
      set({
        sessions: {
          ...get().sessions,
          [sessionId]: { ...cur, rounds },
        },
      });
    };
    updateThink((t) => ({ ...t, loading: true }));
    try {
      const res = await sessionsApi.thinking(sessionId, from, to);
      updateThink((t) => ({ ...t, body: res.text ?? '', loading: false, lazy: undefined }));
    } catch {
      updateThink((t) => ({ ...t, loading: false }));
    }
  },

  respondConfirm: async (approved, trustSession, selected, note) => {
    const cur = get().confirm;
    if (!cur) return;
 // optimistic close: the gate timeout-rejects after 120s, the modal must not linger
    set({ confirm: null });
    try {
      await chatApi.confirm(cur.sessionId, cur.requestId, approved, trustSession, selected, note);
    } catch {
 /* best effort: gate already closed optimistically; backend falls back to deny */
    }
  },

  reset: (sessionId) => {
    const st = get().sessions[sessionId];
    const keepLive = st?.live != null;
    set({
      sessions: {
        ...get().sessions,
        [sessionId]: keepLive ? st : EMPTY_SESSION,
      },
      lastSeq: { ...get().lastSeq, [sessionId]: 0 },
      seenSeq: { ...get().seenSeq, [sessionId]: new Set<number>() },
      terminalSeq: { ...get().terminalSeq, [sessionId]: 0 },
      archived: { ...get().archived, [sessionId]: false },
    });
  },

  removeSession: (sessionId) => {
    const sessions = { ...get().sessions };
    delete sessions[sessionId];
    const lastSeq = { ...get().lastSeq };
    delete lastSeq[sessionId];
    const seenSeq = { ...get().seenSeq };
    delete seenSeq[sessionId];
    const terminalSeq = { ...get().terminalSeq };
    delete terminalSeq[sessionId];
    const archived = { ...get().archived };
    delete archived[sessionId];
    const confirm = get().confirm;
    set({
      sessions, lastSeq, seenSeq, terminalSeq, archived,
      confirm: confirm?.sessionId === sessionId ? null : confirm,
    });
  },
  };
});

bindStreamCtx({
  applyEvent: (sid, ev) => useChatStore.getState().applyEvent(sid, ev),
  peek: () => useChatStore.getState(),
});

initPersistence(useChatStore);

export { feedEvent } from '../lib/microbatch';

export type { ChatNode, Round, ThinkRow } from '../types';
export type { ToolCard } from '../../tools/types';
