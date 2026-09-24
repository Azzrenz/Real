import { invoke } from '@tauri-apps/api/core';
import { t } from '../i18n';
import { useToastStore } from '../store/toastStore';

/** Send a file to the right-dock panel and open it there.
 *
 *  Failures are surfaced rather than swallowed. The first version ended in
 *  `.catch(() => undefined)`, which made a path that does not exist look exactly like a path the
 *  app never recognised at all -- the user clicks, nothing happens, and there is no way to tell
 *  which of the two it was.
 *
 *  `focus: false` shows the dock without pulling it in front. Auto-open uses it: the dock is a
 *  separate OS window, and yanking it forward mid-run interrupts whatever the user is typing or
 *  reading. A click on a path keeps the default -- that one asked for it. */
export async function openInDock(path: string, opts?: { focus?: boolean }): Promise<void> {
  const target = path.trim();
  if (!target) return;
  try {
    await invoke('open_dock_window', { path: target, focus: opts?.focus ?? true });
  } catch (e) {
    useToastStore.getState().show(t('无法在右侧栏打开：{err}', { err: String(e) }), 'error');
  }
}
