import { openInDock } from '../../shared/lib/openInDock';
import { openPathExternal } from '../../shared/lib/openExternal';
import { LANDED, WRITE_TOOLS } from '../../shared/lib/writeTools';

/** What the right dock can render itself: the markdown / html / image / pdf branches of
 *  `FilePreview` (md html htm txt csv), plus images and pdf -- the dock's image branch
 *  (with zoom) and pdf iframe are a better carrier than an external program. */
const DOCK_EXTS: ReadonlySet<string> = new Set([
  'md', 'markdown', 'html', 'htm', 'txt', 'csv',
  'png', 'jpg', 'jpeg', 'gif', 'webp', 'svg', 'bmp', 'pdf',
]);

/** Nothing in the dock can render these, so they go to whichever program owns the extension. */
const EXTERNAL_EXTS: ReadonlySet<string> = new Set([
  'doc', 'docx', 'xls', 'xlsx', 'ppt', 'pptx', 'odt', 'ods', 'odp',
]);

/** One-shot, persisted marker: has the dock EVER auto-opened itself once?
 *
 *  Auto-open is a once-ever courtesy. The user's rule: the dock may open itself the first time a
 *  file lands, but after that it must never wake up on its own again -- not on a task switch, not
 *  on an app restart, not on the next write. It is a separate OS window, and re-summoning it every
 *  time history is replayed (`ChatView` re-applies a task's whole event log on switch, and a
 *  restart reloads the last task the same way) kept yanking the user out of whatever they were
 *  doing. To see it again, open it by hand: the edge hotzone, or click any path in the chat --
 *  both live paths stay untouched. */
const ONESHOT_KEY = 'real.dock-auto-opened';

function autoOpenedBefore(): boolean {
  try {
    return localStorage.getItem(ONESHOT_KEY) === '1';
  } catch {
    return false;
  }
}

function rememberAutoOpened(): void {
  try {
    localStorage.setItem(ONESHOT_KEY, '1');
  } catch {
    void 0;
  }
}

/**
 * A written artifact lands -> send it to the right dock (or hand it to the system).
 *
 * The extension alone decides where it goes. Code files (ts / js / rs / py ...) are deliberately
 * left alone -- an editor popping up every round while code is being written is pure noise.
 * Clicking a path in the chat still opens anything, code included.
 *
 * Auto-open is gated by the one-shot marker above: it fires at most once, ever (see `ONESHOT_KEY`).
 */
export function autoOpenArtifact(_taskId: string, tool: string, status: string, path?: string): void {
  if (!path || !WRITE_TOOLS.has(tool) || !LANDED.has(status)) return;
  const ext = path.split('.').pop()?.toLowerCase() ?? '';
  const toDock = DOCK_EXTS.has(ext);
  if (!toDock && !EXTERNAL_EXTS.has(ext)) return;

  // One-shot, persisted: the dock auto-opens at most once in this install. Everything after --
  // another write, a task switch, a restart -- is a no-op; the user opens it by hand from here on.
  if (autoOpenedBefore()) return;
  rememberAutoOpened();

  // No focus for an auto-open: at that moment the user is most likely typing or reading elsewhere,
  // and a window pulled to the front is an interruption -- they never asked for this one. Clicking
  // a path in the chat keeps the default, because that click IS the request.
  if (toDock) void openInDock(path, { focus: false });
  else void openPathExternal(path);
}
