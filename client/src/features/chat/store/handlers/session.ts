import type { SseEvent } from '../../../../services/contracts';
import type { ChatStoreState, Round, SessionChat } from '../../types';
import { EMPTY_SESSION, finalizeNodes } from '../../lib/nodeFactory';
import { staleForLive } from '../lib/live';
import { runIdOf } from '../lib/runId';
import { t } from '../../../../shared/i18n';

export interface ApplyCtx {
  set: (partial: Partial<ChatStoreState>) => void;
  get: () => ChatStoreState;
  sessionId: string;
  event: SseEvent;
  ensureLive: () => { live: Round; st: SessionChat } | null;
}

/**
 * Settling a round on a terminal event (cancelled / interrupted / error).
 * ONE settle path, and:
 *  - `answer` carries the MODEL's answer only -- never a status sentence like
 *    "task cancelled" (that renders as a fake reply under the round, which is the
 *    "two competing cancel notices" the user reported);
 *  - the status sentence goes to `errorText` (diagnostics), the visible terminal
 *    state is the round head label (cancelled / interrupted);
 *  - an event settles only the round of its own run (run_id), never a foreign one.
 */
export function applySessionEvent(ctx: ApplyCtx): void {
  const { set, get, sessionId, event } = ctx;
  const evRun = runIdOf(event);
  /** round belongs to this event's run (legacy rounds without identity pass) */
  const mine = (r: Round | undefined): boolean => !r || !r.runId || !evRun || r.runId === evRun;
  switch (event.kind) {
    case 'interrupted': {
          const p = (event.payload ?? {}) as { message?: unknown };
          const st = get().sessions[sessionId] ?? EMPTY_SESSION;
          const live = st.live;
          if (!live) return;
          const done: Round = {
            ...live,
            status: 'interrupted',
            endedAt: event.ts ?? live.endedAt,
            nodes: finalizeNodes(live.nodes, 'cancelled'),
            errorText: String(p.message ?? t('应用重启，任务被中断')),
            metrics: {
              runMs: live.startedAt ? Math.max(0, Date.now() - new Date(live.startedAt).getTime()) : undefined,
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
    case 'error': {
          const p = (event.payload ?? {}) as { error?: unknown; message?: unknown };
          let errText = String(p.error ?? p.message ?? t('任务失败'));
          // legacy: older builds emitted cancels as error events (now: cancelled only)
          const cancelLike = errText.includes('编排被用户取消') || errText === t('任务已被取消');
          if (cancelLike) errText = t('任务已被取消');
          const st = get().sessions[sessionId] ?? EMPTY_SESSION;
          const live = st.live;
          if (live && staleForLive(live, event.ts)) {
            set({ terminalSeq: { ...get().terminalSeq, [sessionId]: event.seq } });
            return;
          }
          if (live) {
            const done: Round = {
              ...live,
              status: cancelLike ? 'cancelled' : 'error',
              endedAt: event.ts ?? live.endedAt,
              nodes: finalizeNodes(live.nodes, cancelLike ? 'cancelled' : 'error'),
              errorText: errText,
              metrics: {
                runMs: live.startedAt ? Math.max(0, Date.now() - new Date(live.startedAt).getTime()) : undefined,
              },
            };
            set({
              sessions: {
                ...get().sessions,
                [sessionId]: { rounds: [...st.rounds, done], live: null },
              },
              terminalSeq: { ...get().terminalSeq, [sessionId]: event.seq },
              // 刻意**不**在 error 里置 archived：error 归档的是「一轮」，不是「整个会话」。
              // 否则历史重放（SSE 从 0 全量重放、replaying=false）撞上一条旧 error 就把
              // archived 置真，applyEvent 入口随即吞掉该会话的全部后续事件 —— AI 正文整段
              // 消失、只剩用户气泡。late complete 不建幽灵轮改由两处更细的判据兜住：
              // ① cost.ts：无 live 时先看 archived（取消路径仍会置真），再看 lastRound.status；
              // ② 本文件：归档后 terminalSeq 边界会挡掉 seq <= 它的事件。
            });
          } else {
            set({ terminalSeq: { ...get().terminalSeq, [sessionId]: event.seq } });
          }
          return;
        }
    case 'cancelled': {
          const p = (event.payload ?? {}) as { message?: unknown };
          const st = get().sessions[sessionId] ?? EMPTY_SESSION;
          const live = st.live;
          if (live && staleForLive(live, event.ts)) {
            set({ terminalSeq: { ...get().terminalSeq, [sessionId]: event.seq } });
            return;
          }
          if (!live) {
            const rounds = st.rounds;
            const last = rounds[rounds.length - 1];
            // legacy pairing: an error round followed by cancelled of the SAME run -> reclassify
            if (last && last.status === 'error' && mine(last)) {
              const fixed: Round = {
                ...last,
                status: 'cancelled',
                errorText: String(p.message ?? t('任务已被取消')),
              };
              set({
                sessions: {
                  ...get().sessions,
                  [sessionId]: { ...st, rounds: [...rounds.slice(0, -1), fixed] },
                },
              });
            }
            return;
          }
          const done: Round = {
            ...live,
            status: 'cancelled',
            endedAt: event.ts ?? live.endedAt,
            nodes: finalizeNodes(live.nodes, 'cancelled'),
            errorText: String(p.message ?? t('任务被取消')),
            metrics: {
              runMs: live.startedAt ? Math.max(0, Date.now() - new Date(live.startedAt).getTime()) : undefined,
            },
          };
          // replay and live share ONE rule: a cancel freezes and archives the round
          set({
            sessions: {
              ...get().sessions,
              [sessionId]: { rounds: [...st.rounds, done], live: null },
            },
            terminalSeq: { ...get().terminalSeq, [sessionId]: event.seq },
          });
          return;
        }
    case 'session.title': {
          const t = String(((event.payload ?? {}) as { title?: unknown }).title ?? '').trim();
          if (!t) return;
          void import('../../../sessions/store/sessionStore').then((m) => {
            const ss = m.useSessionStore.getState();
            m.useSessionStore.setState({
              sessions: ss.sessions.map((s) => (s.id === sessionId ? { ...s, title: t } : s)),
            });
          });
          return;
        }
  }
}
