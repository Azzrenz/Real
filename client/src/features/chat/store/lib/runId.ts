import type { SseEvent } from '../../../../services/contracts';

/**
 * The run identity an event carries (stamped backend-side by `sse::with_run`).
 *
 * run = one orchestration execution; it is the coordinate system of events:
 * a terminal event (cancelled/error/complete) settles only the round of its own
 * run. Empty string = no identity (session-level / legacy event) -> ownership is
 * not judged and the old compatible path is used.
 *
 * Lives in its own file (same reason as `lib/live.ts`): both the store and the
 * event handlers need it, and importing it from applyEvent.ts would create a
 * store -> handlers -> store import cycle.
 */
export function runIdOf(event: SseEvent): string {
  const v = (event.payload as { run_id?: unknown } | undefined)?.run_id;
  return typeof v === 'string' ? v : '';
}
