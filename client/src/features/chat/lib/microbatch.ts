
import type { SseEvent } from '../../../services/contracts';
import { applyNow, peekState } from '../store/runtime';

const TEXT_EVENT_KINDS = new Set(['reasoning', 'message']);
interface PendingBatch {
  ev: SseEvent;
  seqs: number[];
  kind: string;
}
const pendingBuf: Record<string, PendingBatch[]> = {};
let pendingTimer: ReturnType<typeof setTimeout> | null = null;

function flushPendingBatch(sid: string) {
  const arr = pendingBuf[sid];
  if (!arr || arr.length === 0) return;
  delete pendingBuf[sid];
  const st = peekState();
  for (const b of arr) {
    for (let i = 0; i < b.seqs.length - 1; i++) {
      st?.seenSeq[sid]?.add(b.seqs[i]);
    }
    applyNow(sid, b.ev);
  }
}

export function feedEvent(sid: string, ev: SseEvent) {
  if (!TEXT_EVENT_KINDS.has(ev.kind)) {
    flushPendingBatch(sid);
    applyNow(sid, ev);
    return;
  }
  const text = String((ev.payload as { text?: unknown } | null)?.text ?? '');
  if (!text) return;
  const list = pendingBuf[sid];
  if (list && list.length > 0) {
    const last = list[list.length - 1];
    if (last.kind === ev.kind) {
      const cur = last.ev.payload as { text?: unknown } | null;
      last.ev = {
        ...last.ev,
        seq: ev.seq,
        payload: { ...(last.ev.payload ?? {}), text: String(cur?.text ?? '') + text },
      };
      last.seqs.push(ev.seq);
    } else {
      list.push({ ev, seqs: [ev.seq], kind: ev.kind });
    }
  } else {
    pendingBuf[sid] = [{ ev, seqs: [ev.seq], kind: ev.kind }];
  }
  if (!pendingTimer) {
    pendingTimer = setTimeout(() => {
      pendingTimer = null;
      for (const k of Object.keys(pendingBuf)) flushPendingBatch(k);
    }, 60);
  }
}
