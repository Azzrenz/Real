import type { SseEvent } from '../../../../services/contracts';
import type { ChatStoreState, EvolutionCard, EvoItem, Round, SessionChat } from '../../types';
import type { ToolCard } from '../../../tools/types';
import { EMPTY_SESSION, nextBubbleId, nextEvoId, nextNarrationId } from '../../lib/nodeFactory';
import { narTail } from '../../narration/pipeline';
import { autoOpenArtifact } from '../../../../app/dock/autoOpen';

export interface ApplyCtx {
  set: (partial: Partial<ChatStoreState>) => void;
  get: () => ChatStoreState;
  sessionId: string;
  event: SseEvent;
  ensureLive: () => { live: Round; st: SessionChat } | null;
}



export function applyToolEvent(ctx: ApplyCtx): void {
  const { set, get, sessionId, event, ensureLive } = ctx;
  switch (event.kind) {
    case 'tool': {
          const tools = (((event.payload ?? {}) as { tools?: unknown[] }).tools ?? []) as Array<{
            step_id?: unknown; call_id?: unknown; name?: unknown; args?: unknown; path?: unknown;
            status?: unknown; duration_ms?: unknown; action?: unknown; result_summary?: unknown;
            reason?: unknown; exit_code?: unknown;
          }>;
          if (tools.length === 0) return;
          const r = ensureLive();
          if (!r) return;
          const { live, st } = r;
          const nodes = live.nodes.slice();
          for (const t of tools) {
            const sid = String(t.step_id ?? t.call_id ?? '');
            const card: ToolCard = {
              type: 'tool',
              stepId: sid || undefined,
              name: String(t.name ?? ''),
              path: typeof t.path === 'string' ? t.path : undefined,
              args: t.args,
              status: (t.status === 'success' && typeof t.exit_code === 'number' && t.exit_code > 0)
                ? 'warning'
                : ((t.status as ToolCard['status']) ?? 'success'),
              durationMs: typeof t.duration_ms === 'number' ? t.duration_ms : undefined,
              summary: typeof t.result_summary === 'string' ? t.result_summary : undefined,
              reason: typeof t.reason === 'string' ? t.reason : undefined,
              exitCode: typeof t.exit_code === 'number' ? t.exit_code : undefined,
              startedAt: event.ts,
            };
            // A written artifact lands -> the right dock claims it. Placed before the merge and
            // fed this beat's raw status, so the running beat is filtered out by LANDED and only
            // the beat that actually landed opens anything.
            // Never during a replay: loading a session re-applies its whole event log (`replay.ts`
            // sets `replaying[sessionId]`), and every past landed write would otherwise fire an
            // open -- that is the "restart / switch task and the dock pops out by itself" bug.
            // Auto-open is a live-event courtesy; the user can still open the dock by hand.
            if (!get().replaying?.[sessionId]) {
              autoOpenArtifact(sessionId, card.name, String(t.status ?? ''), card.path);
            }
            let idx = -1;
            if (sid) {
              for (let i = nodes.length - 1; i >= 0; i--) {
                const n = nodes[i];
                if (n.type === 'tool' && n.stepId === sid && n.status === 'running') { idx = i; break; }
              }
            }
            if (idx >= 0) {
              const prev = nodes[idx] as ToolCard;
              const merged: ToolCard = { ...prev, ...card };
              const incomingHasReason = typeof card.reason === 'string' && card.reason.trim().length > 0;
              const prevHasReason = typeof prev.reason === 'string' && prev.reason.trim().length > 0;
              if (!incomingHasReason && prevHasReason) merged.reason = prev.reason;
   else if (incomingHasReason && !prevHasReason) {/* nothing */}
              nodes[idx] = merged;
            } else {
              nodes.push(card);
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
    case 'user_interjection': {
          const text = String(((event.payload ?? {}) as { text?: unknown }).text ?? '').trim();
          if (!text) return;
          const r = ensureLive();
          if (!r) return;
          const { live, st } = r;
          const nodes = live.nodes.slice();
          const tail = narTail[sessionId] || '';
          if (tail) {
            const last = nodes[nodes.length - 1];
            if (last && last.type === 'narration') nodes[nodes.length - 1] = { ...last, text: last.text + tail };
            else nodes.push({ type: 'narration', id: nextNarrationId(), text: tail });
            delete narTail[sessionId];
          }
          nodes.push({ type: 'user_bubble', id: nextBubbleId(), text });
          set({
            sessions: {
              ...get().sessions,
              [sessionId]: { ...st, live: { ...live, nodes } },
            },
          });
          return;
        }
    case 'evolution.candidates': {
          const p = (event.payload ?? {}) as { items?: unknown; tool_calls?: unknown };
          const items = Array.isArray(p.items) ? (p.items as EvoItem[]) : [];
          if (items.length === 0) return;
          const cur = get().sessions[sessionId] ?? EMPTY_SESSION;
          const card: EvolutionCard = {
            type: 'evo',
            id: nextEvoId(),
            items,
            toolCalls: Number(p.tool_calls ?? 0),
          };
          if (cur.live) {
            set({
              sessions: {
                ...get().sessions,
                [sessionId]: { ...cur, live: { ...cur.live, nodes: [...cur.live.nodes, card] } },
              },
            });
            return;
          }
          const rounds = cur.rounds.slice();
          if (rounds.length === 0) return;
          const last = rounds[rounds.length - 1];
          rounds[rounds.length - 1] = { ...last, nodes: [...last.nodes, card] };
          set({ sessions: { ...get().sessions, [sessionId]: { ...cur, rounds } } });
          return;
        }
  }
}
