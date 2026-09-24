
import type { Message, SseEvent, ToolCall } from '../../../services/contracts';
import type { ChatNode, ChatStoreState, Round } from '../types';
import type { AsstTool } from '../lib/replayHelpers';
import { mapToolStatus, parseArgs, toolSummary } from '../lib/replayHelpers';
import {
  EMPTY_SESSION,
  finalizeNodes,
  nextBubbleId,
  nextNarrationId,
  nextThinkId,
  trimTrailingAnswer,
} from '../lib/nodeFactory';
import { LEAD_PUNCT, narTail } from '../narration/pipeline';
import { t } from '../../../shared/i18n';

export function createReplay(
  set: (partial: Partial<ChatStoreState>) => void,
  get: () => ChatStoreState,
) {
  const replayEvents = (sessionId: string, events: SseEvent[]) => {
    if (!events || events.length === 0) return;
    delete narTail[sessionId];
    const seen = get().seenSeq[sessionId];
    if (seen && seen.size > 0 && events.every((ev) => seen.has(ev.seq))) return;
    set({
      sessions: { ...get().sessions, [sessionId]: EMPTY_SESSION },
      lastSeq: { ...get().lastSeq, [sessionId]: 0 },
      seenSeq: { ...get().seenSeq, [sessionId]: new Set<number>() },
      terminalSeq: { ...get().terminalSeq, [sessionId]: 0 },
      archived: { ...get().archived, [sessionId]: false },
      replaying: { ...get().replaying, [sessionId]: true },
    });
    try {
      for (const ev of events) {
        try {
          get().applyEvent(sessionId, ev);
        } catch (err) {
          console.warn("[replay] 事件还原失败，跳过", ev?.kind, err);
        }
      }
    } finally {
      set({ replaying: { ...get().replaying, [sessionId]: false } });
    }
  };

  const replayFromMessages = (sessionId: string, messages: Message[], toolCalls?: ToolCall[]) => {
    const cur = get().sessions[sessionId];
    if (cur && cur.rounds.length > 0) return;
    delete narTail[sessionId];
    const msgs = (messages ?? [])
      .slice()
      .sort((a, b) => a.created_at.localeCompare(b.created_at));
    const tcByStep: Record<string, ToolCall> = {};
    for (const tc of (toolCalls ?? [])) {
      if (tc && tc.step_id) tcByStep[tc.step_id] = tc;
    }

    const parseAssistant = (m: Message): { reasoning: string; narration: string; tools: AsstTool[] } => {
      let reasoning = typeof m.reasoning === 'string' ? m.reasoning : '';
      let narration = '';
      const tools: AsstTool[] = [];
      if (m.item_json) {
        try {
          const obj = JSON.parse(m.item_json) as {
            content?: Array<{ text?: unknown }>;
            tool_calls?: Array<{ call_id?: unknown; name?: unknown; arguments?: unknown }>;
          };
          const parts = Array.isArray(obj.content) ? obj.content : [];
          for (const p of parts) {
            if (p && typeof p.text === 'string') narration += p.text;
          }
          const tcs = Array.isArray(obj.tool_calls) ? obj.tool_calls : [];
          for (const t of tcs) {
            if (!t || typeof t.name !== 'string') continue;
            tools.push({
              callId: typeof t.call_id === 'string' ? t.call_id : '',
              name: t.name,
              args: typeof t.arguments === 'string' ? t.arguments : JSON.stringify(t.arguments ?? {}),
            });
          }
        } catch {
        }
      }
      if (!narration && typeof m.content === 'string') narration = m.content;
      return { reasoning, narration, tools };
    };

    const buildAssistantNodes = (m: Message): ChatNode[] => {
      const { reasoning, narration, tools } = parseAssistant(m);
      const nodes: ChatNode[] = [];
      if (reasoning.trim()) {
        nodes.push({ type: 'think', id: nextThinkId(), label: t('深度思考'), note: '', body: reasoning.trim(), status: 'done' });
      }
      const narText = narration.trim().replace(LEAD_PUNCT, '');
      if (narText) {
        nodes.push({ type: 'narration', id: nextNarrationId(), text: narText });
      }
      for (const t of tools) {
        const tc = t.callId ? tcByStep[t.callId] : undefined;
        nodes.push({
          type: 'tool',
          stepId: t.callId || undefined,
          name: tc?.name || t.name,
          // parseArgs 只收一个实参；原先多传的 t.args 被丢弃 —— 历史 tool_calls 的
          // arguments 为空时参数就丢了，而 item_json 里的 t.args 本来就是真实入参。
          args: parseArgs(tc?.arguments ?? t.args),
          status: mapToolStatus(tc?.status),
          durationMs: typeof tc?.duration_ms === 'number' ? tc.duration_ms : undefined,
          summary: tc ? toolSummary(tc.result_json) : undefined,
          startedAt: tc?.created_at,
        });
      }
      return nodes;
    };

    const rounds: Round[] = [];
    let seq = 0;
    let curNodes: ChatNode[] = [];
    let curStarted = '';
    let lastNarration = '';
    const flush = () => {
      seq += 1;
      const finalAnswer = lastNarration.trim();
      const nodes = trimTrailingAnswer(finalizeNodes(curNodes, 'success'), finalAnswer);
      rounds.push({
        seq,
        startedAt: curStarted || new Date().toISOString(),
        endedAt: curStarted || new Date().toISOString(),
        status: 'completed',
        nodes,
        answer: finalAnswer,
        metrics: {},
        usage: [],
        totals: { tool_count: nodes.filter((n) => n.type === 'tool').length },
      });
      curNodes = [];
      lastNarration = '';
      curStarted = '';
    };
    for (const m of msgs) {
      if (m.role === 'user') {
        let ij = false;
        let cancelled = false;
        try {
          const ij0 = m.item_json ? JSON.parse(m.item_json) : null;
          ij = ij0?.interjection === true;
          cancelled = ij0?.cancelled === true;
        } catch { /* malformed item_json: treat it as a plain user event */ }
        if (ij) {
          curNodes.push({
            type: 'user_bubble',
            id: nextBubbleId(),
            text: typeof m.content === 'string' ? m.content : '',
            ...(cancelled ? { cancelled: true } : {}),
          });
          continue;
        }
        if (curNodes.length > 0) flush();
        curStarted = m.created_at;
        continue;
      }
      if (m.role === 'assistant') {
        const ns = buildAssistantNodes(m);
        for (const n of ns) {
          if (n.type === 'narration') lastNarration = n.text;
          curNodes.push(n);
        }
        continue;
      }
      if (m.role === 'reasoning') {
        const body = typeof m.content === 'string' ? m.content : '';
        if (body.trim()) {
          curNodes.push({ type: 'think', id: nextThinkId(), label: t('深度思考'), note: '', body: body.trim(), status: 'done' });
        }
        continue;
      }
    }
    if (curNodes.length > 0) flush();

    if (rounds.length === 0) {
      set({
        sessions: { ...get().sessions, [sessionId]: EMPTY_SESSION },
        lastSeq: { ...get().lastSeq, [sessionId]: 0 },
        seenSeq: { ...get().seenSeq, [sessionId]: new Set<number>() },
        terminalSeq: { ...get().terminalSeq, [sessionId]: 0 },
        archived: { ...get().archived, [sessionId]: false },
      });
      return;
    }
    set({
      sessions: { ...get().sessions, [sessionId]: { rounds, live: null } },
      lastSeq: { ...get().lastSeq, [sessionId]: 0 },
      seenSeq: { ...get().seenSeq, [sessionId]: new Set<number>() },
      terminalSeq: { ...get().terminalSeq, [sessionId]: 0 },
      archived: { ...get().archived, [sessionId]: false },
    });
  };

  return { replayEvents, replayFromMessages };
}
