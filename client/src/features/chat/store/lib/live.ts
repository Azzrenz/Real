import type { Round } from '../../types';

export function staleForLive(live: Pick<Round, 'startedAt'> | null, eventTs?: string): boolean {
  if (!live?.startedAt || !eventTs) return false;
  const lt = Date.parse(live.startedAt);
  const et = Date.parse(eventTs);
  return !Number.isNaN(lt) && !Number.isNaN(et) && et < lt;
}
