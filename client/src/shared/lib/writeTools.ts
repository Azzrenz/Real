/** "Did this call touch the disk?" -- one answer, two consumers: the auto-open path */


export const WRITE_TOOLS: ReadonlySet<string> = new Set(['write', 'edit', 'modify']);

/** Statuses where the write actually landed. A failure did not land and a running call has not */
export const LANDED: ReadonlySet<string> = new Set(['success', 'warning']);
