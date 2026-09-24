import type { SseEvent } from '../../../../services/contracts';
import type { ChatStoreState, Round, SessionChat, ThinkRow } from '../../types';
import { nextNarrationId, nextThinkId } from '../../lib/nodeFactory';
import { narTail, LEAD_PUNCT, isNarrationNoise, isVerbalPrelude } from '../../narration/pipeline';
import { t } from '../../../../shared/i18n';

export interface ApplyCtx {
  set: (partial: Partial<ChatStoreState>) => void;
  get: () => ChatStoreState;
  sessionId: string;
  event: SseEvent;
  ensureLive: () => { live: Round; st: SessionChat } | null;
}



export function applyChatEvent(ctx: ApplyCtx): void {
  const { set, get, sessionId, event, ensureLive } = ctx;
  switch (event.kind) {
    case 'reasoning': {
          const p = (event.payload ?? {}) as { text?: unknown; omitted?: unknown; from?: unknown; to?: unknown; len?: unknown };
          if (p.omitted) {
            const r = ensureLive();
            if (!r) return;
            const { live, st } = r;
            const nodes = live.nodes.slice();
            const lazy = {
              from: Number(p.from ?? event.seq),
              to: Number(p.to ?? event.seq),
              len: typeof p.len === 'number' ? p.len : undefined,
            };
            let idx = -1;
            for (let i = nodes.length - 1; i >= 0; i--) {
              const n = nodes[i];
              if (n.type === 'think' && !n.body && !n.lazy) { idx = i; break; }
            }
            if (idx >= 0) {
              const th = nodes[idx] as ThinkRow;
              nodes[idx] = { ...th, lazy, status: 'done' };
            } else {
              nodes.push({ type: 'think', id: nextThinkId(), label: t('深度思考'), note: '', body: '', status: 'done', lazy });
            }
            set({
              sessions: {
                ...get().sessions,
                [sessionId]: { ...st, live: { ...live, nodes } },
              },
            });
            return;
          }
          const text = String(p.text ?? '');
          if (!text) return;
          const r = ensureLive();
          if (!r) return;
          const { live, st } = r;
          const nodes = live.nodes.slice();
          let idx = -1;
          for (let i = nodes.length - 1; i >= 0; i--) {
            const n = nodes[i];
            if (n.type === 'think' && n.status === 'thinking') { idx = i; break; }
          }
          if (idx >= 0) {
            const th = nodes[idx] as ThinkRow;
            const prev = th.body;
            let delta = text;
            if (prev.length > 0 && text.length > prev.length && text.startsWith(prev)) {
              delta = text.slice(prev.length);
            }
            if (delta) nodes[idx] = { ...th, body: th.body + delta };
          } else {
            const last = nodes[nodes.length - 1];
            if (last && last.type === 'think') {
              const th = last as ThinkRow;
              nodes[nodes.length - 1] = { ...th, body: th.body + text };
            } else {
              nodes.push({ type: 'think', id: nextThinkId(), label: t('深度思考'), note: '', body: text, status: 'thinking' });
            }
          }
          set({
            sessions: {
              ...get().sessions,
              [sessionId]: { ...st, live: { ...live, nodes } },
            },
          });
          return;
        }
    case 'thinking': {
          const p = (event.payload ?? {}) as { status?: unknown; label?: unknown; summary?: unknown };
          const status = String(p.status ?? 'end');
          const label = String(p.label ?? t('深度思考'));
          const r = ensureLive();
          if (!r) return;
          const { live, st } = r;
          const nodes = live.nodes.slice();
          if (status === 'start') {
            const carry = narTail[sessionId] ?? '';
            if (carry) {
              narTail[sessionId] = '';
              if (!isVerbalPrelude(carry)) {
                const lastN = nodes[nodes.length - 1];
                if (lastN && lastN.type === 'narration') {
                  nodes[nodes.length - 1] = { ...lastN, text: lastN.text + carry };
                } else {
                  nodes.push({ type: 'narration', id: nextNarrationId(), text: carry });
                }
              }
            }
            const last = nodes[nodes.length - 1];
            if (last && last.type === 'think' && last.status === 'thinking') {
              nodes[nodes.length - 1] = { ...last, label, status: 'thinking' };
            } else {
              nodes.push({ type: 'think', id: nextThinkId(), label, note: '', body: '', status: 'thinking' });
            }
          } else {
            const endSummary = typeof p.summary === 'string' ? p.summary : '';
            for (let i = nodes.length - 1; i >= 0; i--) {
              const n = nodes[i];
              if (n.type === 'think' && n.status === 'thinking') {
                if (!(n.body || '').trim() && !n.lazy) {
                  nodes.splice(i, 1);
                } else {
                  nodes[i] = { ...n, status: 'done', note: endSummary };
                }
              }
            }
          }
          set({
            sessions: {
              ...get().sessions,
              [sessionId]: { ...st, live: { ...live, nodes } },
            },
          });
          return;
        }
    case 'message': {
          const text = String(((event.payload ?? {}) as { text?: unknown }).text ?? '').trim();
          if (!text) return;
          const r = ensureLive();
          if (!r) return;
          const { live, st } = r;
          const prev = live.answer;
          let delta = text;
          if (prev.length > 0 && text.length > prev.length && text.startsWith(prev)) {
            delta = text.slice(prev.length);
          }
          if (isNarrationNoise(delta)) {
            set({
              sessions: {
                ...get().sessions,
                [sessionId]: { ...st, live: { ...live, answer: text } },
              },
            });
            return;
          }
          if (isVerbalPrelude(delta)) {
            const carry = (narTail[sessionId] ?? '') + delta;
            narTail[sessionId] = carry.length > 2000 ? '' : carry;
            set({
              sessions: {
                ...get().sessions,
                [sessionId]: { ...st, live: { ...live, answer: text } },
              },
            });
            return;
          }
          let carried = narTail[sessionId] ?? '';
          narTail[sessionId] = '';
          if (carried.length > 2000) carried = '';
          if (carried && delta.startsWith(carried.trim())) carried = '';
          const full = carried + delta;
          const nodes = live.nodes.slice();
          const last = nodes[nodes.length - 1];
          if (last && last.type === 'narration') {
            nodes[nodes.length - 1] = { ...last, text: last.text + full };
          } else {
            const clean = full.replace(LEAD_PUNCT, '');
            if (clean) nodes.push({ type: 'narration', id: nextNarrationId(), text: clean });
          }
          narTail[sessionId] = '';
          set({
            sessions: {
              ...get().sessions,
              [sessionId]: { ...st, live: { ...live, nodes, answer: text } },
            },
          });
          return;
        }
  }
}
