import type { SseEvent } from '../../../services/contracts';
import type { ChatStoreState, Round, SessionChat } from '../types';
import { EMPTY_SESSION, emptyRound } from '../lib/nodeFactory';
import { useSettings } from '../../settings/store/settingsStore';
import { applyChatEvent } from './handlers/chat';
import { applyCostEvent } from './handlers/cost';
import type { ApplyCtx } from './handlers/cost';
import { applyToolEvent } from './handlers/tool';
import { applySessionEvent } from './handlers/session';
import { runIdOf } from './lib/runId';
import { t } from '../../../shared/i18n';

export function createApplyEvent(
  set: (partial: Partial<ChatStoreState>) => void,
  get: () => ChatStoreState,
) {
  return (sessionId: string, event: SseEvent) => {
    if (get().archived[sessionId]) return;
    if (
      event.kind === 'confirm.request' ||
      event.kind === 'confirm.cancelled' ||
      event.kind === 'confirm.resolved'
    ) {
      if (event.kind === 'confirm.request') {
        const p = (event.payload ?? {}) as Record<string, unknown>;
        const requestId = String(p.request_id ?? '');
        if (!requestId) return;
        const cur = get().confirm;
        if (cur && cur.requestId === requestId) return;
        set({
          confirm: {
            sessionId,
            requestId,
            action: String(p.action ?? ''),
            target: String(p.target ?? ''),
            impact: String(p.impact ?? ''),
            riskLabel: String(p.risk_label ?? 'danger'),
            workspaceCandidates: Array.isArray(p.workspace_candidates)
              ? (p.workspace_candidates as unknown[]).map(String)
              : undefined,
            options: Array.isArray(p.options)
              ? (p.options as Record<string, unknown>[])
                  .map((o) => ({
                    label: String(o.label ?? ''),
                    detail: String(o.detail ?? ''),
                    recommended: o.recommended === true,
                  }))
                  .filter((o) => o.label)
              : undefined,
          },
        });
      } else {
        const p = (event.payload ?? {}) as { request_id?: unknown };
        const cur = get().confirm;
        if (cur && String(p.request_id ?? '') === cur.requestId) set({ confirm: null });
      }
      return;
    }
    if (event.kind === 'progress') {
      const p = (event.payload ?? {}) as { step_id?: unknown; elapsed_ms?: unknown };
      const sid = typeof p.step_id === 'string' ? p.step_id : '';
      if (!sid) return;
      const cur = get().sessions[sessionId];
      const live = cur?.live;
      if (!live) return;
      const nodes = live.nodes.slice();
      let hit = false;
      for (let i = nodes.length - 1; i >= 0; i--) {
        const n = nodes[i];
        if (n.type === 'tool' && n.stepId === sid && n.status === 'running') {
          nodes[i] = { ...n, elapsedMs: typeof p.elapsed_ms === 'number' ? p.elapsed_ms : undefined };
          hit = true;
          break;
        }
      }
      if (hit) {
        set({
          sessions: {
            ...get().sessions,
            [sessionId]: { ...cur, live: { ...live, nodes } },
          },
        });
      }
      return;
    }
    const s = get();
    const seen = s.seenSeq[sessionId];
    if (seen && seen.has(event.seq)) {
 // duplicate (reconnect replay): keep lastSeq current so ?after= stays fresh
      if (event.seq > (s.lastSeq[sessionId] ?? 0)) {
        set({ lastSeq: { ...s.lastSeq, [sessionId]: event.seq } });
      }
      return;
    }
    set({
      lastSeq: { ...s.lastSeq, [sessionId]: Math.max(event.seq, s.lastSeq[sessionId] ?? 0) },
      seenSeq: { ...s.seenSeq, [sessionId]: (seen ?? new Set<number>()).add(event.seq) },
    });
 // Round ownership (single rule): an event belongs only to the run that produced it.
 // Cancel is async -- the old run only stops at its next boundary, so its terminal
 // event arrives AFTER the new message already started. Guessing ownership by
 // timestamp then marks the new round as "cancelled" (the duplicate-cancel report).
 // Events carrying a run_id that is not the live round's run are dropped here.
 // Events without a run_id (session-level / legacy) keep the old behaviour.
    const evRun = runIdOf(event);
    const liveRun = get().sessions[sessionId]?.live?.runId ?? '';
    if (evRun && liveRun && evRun !== liveRun) return;
 // late events after a terminal boundary (cancel/complete) are ignored
    if (event.seq <= (get().terminalSeq[sessionId] ?? 0)) return;

    const ensureLive = (): { live: Round; st: SessionChat } | null => {
 // Always read FRESH state: the per-event `st` snapshot captured before
 // ensureLive ran would spread the stale `running: false` back over the
 // live round (sim-proven: running stayed false for the whole stream,
 // corrupting send gate / watchdog / busy), freezing UI state machines.
      const cur = get().sessions[sessionId] ?? EMPTY_SESSION;
      if (cur.live) {
        // bind the run identity on the first event (session attach / restart
        // mid-run: there is no send-response run_id to rely on)
        const needRun = !cur.live.runId && !!evRun;
        if ((needRun || (cur.live.nodes.length === 0 && !cur.live.answer)) && event.ts) {
          const fixed: Round = {
            ...cur.live,
            ...(needRun ? { runId: evRun } : {}),
            ...(cur.live.nodes.length === 0 && !cur.live.answer
              ? {
                  startedAt: event.ts,
                  totals: { ...cur.live.totals, model: cur.live.totals.model ?? useSettings.getState().model },
                }
              : {}),
          };
          set({
            sessions: {
              ...get().sessions,
              [sessionId]: { ...cur, live: fixed },
            },
          });
          return { live: fixed, st: cur };
        }
        return { live: cur.live, st: cur };
      }
      // no live round: open one for this run, stamped with its own run identity
      const live: Round = evRun
        ? { ...emptyRound(event.seq, event.ts), runId: evRun }
        : emptyRound(event.seq, event.ts);
      const st: SessionChat = { rounds: cur.rounds, live };
      set({
        sessions: {
          ...get().sessions,
          [sessionId]: st,
        },
      });
      return { live, st };
    };

    // 轮内瞬时提示的生命周期（2026-09-23）：只有 `llm.retry` 会挂 `notice`，
    // **任何其它轮内事件都意味着"这一波过去了"**（模型续写/工具结果回来了）——
    // 统一清掉，否则提示会残留成"假故障"，那比不提示更糟。
    if (event.kind !== 'llm.retry') {
      const curRound = get().sessions[sessionId]?.live;
      if (curRound?.notice) {
        const rest: Round = { ...curRound };
        delete rest.notice;
        set({
          sessions: {
            ...get().sessions,
            [sessionId]: { ...get().sessions[sessionId]!, live: rest },
          },
        });
      }
    }

    const ctx: ApplyCtx = { set, get, sessionId, event, ensureLive };

    switch (event.kind) {
      case 'reasoning':
      case 'thinking':
      case 'message':
        return applyChatEvent(ctx);
      case 'tool':
      case 'user_interjection':
      case 'evolution.candidates':
        return applyToolEvent(ctx);
      case 'llm.usage':
      case 'complete':
        return applyCostEvent(ctx);
      case 'llm.retry': {
        // 上游繁忙·自动重试（2026-09-23）：**不是失败**——后端正在退避重试，马上继续。
        // 所以它只往 `live.notice` 挂一句话，不终结轮次（不置 status、不 finalizeNodes、
        // 不动 terminalSeq）。此前这段等待在界面上是一片空白，用户只能看到最后那记红叉。
        const ensured = ensureLive();
        if (!ensured) return;
        const { live } = ensured;
        const p = (event.payload ?? {}) as {
          attempt?: unknown;
          max?: unknown;
          backoff_ms?: unknown;
        };
        const secs = Math.max(1, Math.round(Number(p.backoff_ms ?? 0) / 1000));
        const notice = t('上游繁忙·正在自动重试（第 {n}/{m} 次，约 {s}s 后重来）')
          .replace('{n}', String(Number(p.attempt ?? 0)))
          .replace('{m}', String(Number(p.max ?? 0)))
          .replace('{s}', String(secs));
        set({
          sessions: {
            ...get().sessions,
            [sessionId]: { ...get().sessions[sessionId]!, live: { ...live, notice } },
          },
        });
        return;
      }
      case 'interrupted':
      case 'error':
      case 'cancelled':
      case 'session.title':
        return applySessionEvent(ctx);
      default:
        return;
    }
  };
}
