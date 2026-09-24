import type { LLMCall, SseEvent } from '../../../../services/contracts';
import type { ChatStoreState, Round, SessionChat } from '../../types';
import { EMPTY_SESSION, finalizeNodes, nextNarrationId, trimTrailingAnswer } from '../../lib/nodeFactory';
import { narTail } from '../../narration/pipeline';
import { t } from '../../../../shared/i18n';

export interface ApplyCtx {
  set: (partial: Partial<ChatStoreState>) => void;
  get: () => ChatStoreState;
  sessionId: string;
  event: SseEvent;
  ensureLive: () => { live: Round; st: SessionChat } | null;
}


export function applyCostEvent(ctx: ApplyCtx): void {
  const { set, get, sessionId, event, ensureLive } = ctx;
  switch (event.kind) {
    case 'llm.usage': {
          const p = (event.payload ?? {}) as Partial<LLMCall> & { model?: string; cost_yuan?: number };
          if (typeof p.input !== 'number' && typeof p.output !== 'number') return;
          const r = ensureLive();
          if (!r) return;
          const { live, st } = r;
          const call: LLMCall = {
            label: String(p.label ?? p.model ?? t('LLM 调用')),
            input: Number(p.input ?? 0),
            cached: Number(p.cached ?? 0),
            output: Number(p.output ?? 0),
            cost: typeof p.cost_yuan === 'number' ? p.cost_yuan : typeof p.cost === 'number' ? p.cost : 0,
            peak: String(p.peak ?? 'offpeak'),
            cost_miss: typeof (p as { cost_miss_yuan?: unknown }).cost_miss_yuan === 'number' ? (p as { cost_miss_yuan: number }).cost_miss_yuan : undefined,
            cost_cached: typeof (p as { cost_cached_yuan?: unknown }).cost_cached_yuan === 'number' ? (p as { cost_cached_yuan: number }).cost_cached_yuan : undefined,
            cost_output: typeof (p as { cost_output_yuan?: unknown }).cost_output_yuan === 'number' ? (p as { cost_output_yuan: number }).cost_output_yuan : undefined,
          };
          set({
            sessions: {
              ...get().sessions,
              [sessionId]: { ...st, live: { ...live, usage: [...live.usage, call] } },
            },
          });
          return;
        }
    case 'complete': {
          const p = (event.payload ?? {}) as {
            answer?: unknown; tool_count?: unknown; llm_calls?: unknown;
            cost_yuan?: unknown; peak_tag?: unknown; cache_hit_ratio?: unknown; model?: unknown;
          };
          const st = get().sessions[sessionId] ?? EMPTY_SESSION;
          const live = st.live;
          if (!live) {
   // cancel path already archived this run: a late complete (sent by the
   // backend before the cancel landed) must not spawn a phantom round.
            if (get().archived[sessionId]) return;
            const lastRound = st.rounds[st.rounds.length - 1];
            if (lastRound && lastRound.status !== 'running') {
              set({ terminalSeq: { ...get().terminalSeq, [sessionId]: event.seq } });
              return;
            }
            if (typeof p.answer === 'string' && p.answer.length > 0) {
              const direct: Round = {
                seq: event.seq,
                startedAt: event.ts ?? new Date().toISOString(),
                endedAt: event.ts,
                status: 'completed',
                nodes: [],
                answer: p.answer,
                metrics: {},
                usage: [],
                totals: {
                  tool_count: Number(p.tool_count ?? 0),
                  llm_calls: Number(p.llm_calls ?? 0),
                  cost_yuan: typeof p.cost_yuan === 'number' ? p.cost_yuan : undefined,
                  peak_tag: typeof p.peak_tag === 'string' ? p.peak_tag : undefined,
                  cache_hit_ratio: typeof p.cache_hit_ratio === 'number' ? p.cache_hit_ratio : undefined,
                  model: typeof p.model === 'string' ? p.model : undefined,
                },
              };
              set({
                sessions: {
                  ...get().sessions,
                  [sessionId]: { ...st, rounds: [...st.rounds, direct], live: null },
                },
                terminalSeq: { ...get().terminalSeq, [sessionId]: event.seq },
              });
            } else {
              set({
                sessions: {
                  ...get().sessions,
                  [sessionId]: { ...st, live: null },
                },
                terminalSeq: { ...get().terminalSeq, [sessionId]: event.seq },
              });
            }
            return;
          }
          const finalAnswer = String(p.answer ?? live.answer ?? '');
   // flush leftover half-sentence into the last narration node before archiving.
          const tail = narTail[sessionId] || '';
          let liveNodes = live.nodes;
          if (tail) {
            const ns = liveNodes.slice();
            const last = ns[ns.length - 1];
            if (last && last.type === 'narration') ns[ns.length - 1] = { ...last, text: last.text + tail };
            else ns.push({ type: 'narration', id: nextNarrationId(), text: tail });
            liveNodes = ns;
            delete narTail[sessionId];
          }
          const done: Round = {
            ...live,
            status: 'completed',
            endedAt: event.ts ?? live.endedAt,
            nodes: trimTrailingAnswer(finalizeNodes(liveNodes, 'success'), finalAnswer),
            answer: finalAnswer,
            metrics: {
              runMs: live.startedAt ? Math.max(0, Date.now() - new Date(live.startedAt).getTime()) : undefined,
            },
            totals: {
              tool_count: Number(p.tool_count ?? live.nodes.filter((n) => n.type === 'tool').length),
              llm_calls: Number(p.llm_calls ?? 0),
              cost_yuan: typeof p.cost_yuan === 'number' ? p.cost_yuan : undefined,
              peak_tag: typeof p.peak_tag === 'string' ? p.peak_tag : undefined,
              cache_hit_ratio: typeof p.cache_hit_ratio === 'number' ? p.cache_hit_ratio : undefined,
            },
          };
          set({
            sessions: {
              ...get().sessions,
              [sessionId]: { rounds: [...st.rounds, done], live: null },
            },
            terminalSeq: { ...get().terminalSeq, [sessionId]: event.seq },
          });
          return;
        }
  }
}
