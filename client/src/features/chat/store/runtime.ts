
import type { SseEvent } from '../../../services/contracts';
import type { ChatStoreState } from '../types';

interface StreamCtx {
  applyEvent: (sid: string, ev: SseEvent) => void;
  peek: () => ChatStoreState;
}

let ctx: StreamCtx | null = null;

export function bindStreamCtx(c: StreamCtx): void {
  ctx = c;
}

export function applyNow(sid: string, ev: SseEvent): void {
  ctx?.applyEvent(sid, ev);
}

export function peekState(): ChatStoreState | null {
  return ctx ? ctx.peek() : null;
}
