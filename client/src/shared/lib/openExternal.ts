import { invoke } from '@tauri-apps/api/core';

/** Open a link in the system default browser.
 *
 *  There is deliberately NO window.open fallback: inside the Tauri webview that does not spawn
 *  a window, it navigates the CURRENT one - the whole app UI is replaced by the target page and
 *  the user has no way back. Failing loudly is strictly better than silently destroying the
 *  session, so a refusal is logged instead of redirected. */
export async function openExternal(url: string): Promise<void> {
  if (!url || (!url.startsWith('http://') && !url.startsWith('https://'))) return;
  try {
    await invoke('plugin:opener|open_url', { url });
  } catch (e) {
    console.error('[openExternal] opener plugin refused to open', url, e);
  }
}

/** Open a local file with whichever program owns it (an Office document, typically).
 *
 *  Goes through the shell's own command rather than `plugin:opener|open_path`: the plugin's
 *  frontend command is gated by the capability's path scope, and the files this has to open are
 *  wherever the task happens to be (D:\proj one day, C:\Program Files\Cubase 15 the next).
 *  Keeping it in the shell means the ACL does not have to be opened to the whole disk.
 *  Failures are logged, not thrown -- this runs from an auto-open, where a missing association
 *  is not worth a toast over the agent's own output. */
export async function openPathExternal(path: string): Promise<void> {
  const target = path?.trim();
  if (!target) return;
  try {
    await invoke('open_external_path', { path: target });
  } catch (e) {
    console.error('[openExternal] shell refused to open', target, e);
  }
}

/** Reveal a file in the system file manager -- selects it, does not open it.
 *
 *  Also a shell command, for the same reason as `openPathExternal`: the dock offers this for
 *  whatever file is on screen, and that file is not confined to one directory. */
export async function revealInFolder(path: string): Promise<void> {
  const target = path?.trim();
  if (!target) return;
  try {
    await invoke('reveal_in_folder', { path: target });
  } catch (e) {
    console.error('[openExternal] shell refused to reveal', target, e);
  }
}
