
import type { ChatNode, Round, SessionChat } from '../types';

export const EMPTY_SESSION: SessionChat = { rounds: [], live: null };


let THINK_ID = 0;
export function nextThinkId(): number {
  THINK_ID += 1;
  return THINK_ID;
}

let NARR_ID = 0;
export function nextNarrationId(): number {
  NARR_ID += 1;
  return NARR_ID;
}

let BUBBLE_ID = 0;
export function nextBubbleId(): number {
  BUBBLE_ID += 1;
  return BUBBLE_ID;
}

let EVO_ID = 0;
export function nextEvoId(): number {
  EVO_ID += 1;
  return EVO_ID;
}

export function emptyRound(seq: number, ts?: string): Round {
  return {
    seq,
    startedAt: ts ?? new Date().toISOString(),
    status: 'running',
    nodes: [],
    answer: '',
    metrics: {},
    usage: [],
    totals: {},
  };
}

export function finalizeNodes(nodes: ChatNode[], toolStatus: 'success' | 'error' | 'cancelled'): ChatNode[] {
  const out: ChatNode[] = [];
  for (const n of nodes) {
    if (n.type === 'tool' && n.status === 'running') {
      out.push({ ...n, status: toolStatus });
      continue;
    }
    if (n.type === 'think' && n.status === 'thinking') {
      if ((n.body ?? '').trim() || n.lazy) out.push({ ...n, status: 'done' });
      continue;
    }
    out.push(n);
  }
  return out;
}

export function trimTrailingAnswer(nodes: ChatNode[], answer: string): ChatNode[] {
  if (!answer) return nodes;
  const normAnswer = answer.replace(/\s+/g, '');
  if (!normAnswer) return nodes;
  const out = nodes.slice();
  for (let i = out.length - 1; i >= 0; i--) {
    const n = out[i];
    if (n.type !== 'narration') continue;
    const text = n.text;
    const idxMap: number[] = [];
    let buf = '';
    for (let j = 0; j < text.length; j++) {
      if (/\s/.test(text[j])) continue;
      buf += text[j];
      idxMap.push(j);
    }
    let cutNorm = -1;
    const headMarker = normAnswer.slice(0, 24);
    if (headMarker.length >= 12) {
      const q = buf.indexOf(headMarker);
      if (q >= 0) cutNorm = q;
    }
    if (cutNorm < 0) {
      let k = Math.min(buf.length, normAnswer.length);
      while (k >= 40 && !buf.endsWith(normAnswer.slice(normAnswer.length - k))) k--;
      if (k >= 40) cutNorm = buf.length - k;
    }
    if (cutNorm >= 8) {
      const startOrig = idxMap[cutNorm] ?? text.length;
      const lead = text.slice(0, startOrig).trimEnd();
      if (lead) out[i] = { ...n, text: lead };
      else out.splice(i, 1);
    }
    break;
  }
  return out;
}
