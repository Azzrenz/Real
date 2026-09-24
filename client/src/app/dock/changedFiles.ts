import { useEffect, useMemo, useRef, useState } from 'react';
import { emit, emitTo, listen } from '@tauri-apps/api/event';
import { useChatStore } from '../../features/chat/store/chatStore';
import { useSessionStore } from '../../features/sessions/store/sessionStore';
import { useLang, type Lang } from '../../shared/i18n';
import { LANDED, WRITE_TOOLS } from '../../shared/lib/writeTools';
import { inTauri } from '../../shared/lib/tauri';
import { useSettings, type RealTheme } from '../../features/settings/store/settingsStore';

const DOCK_LABEL = 'right-dock';
const PUSH_EVENT = 'dock:sync';
const REQUEST_EVENT = 'dock:request-sync';

export interface ChangedFile {
  path: string;
  name: string;

  tool: string;
}

export interface DockSync {

  task: { title: string; startedAt: string } | null;

  files: ChangedFile[];

  lang: Lang;

  /** The main window's colour theme, mirrored into the dock's own store: separate windows have
   *  separate documents and stores, so a theme flipped over there does not arrive on its own. */
  theme: RealTheme;
}

interface NodeShape {
  type?: string;
  name?: string;
  path?: string;
  status?: string;
}

export function collectChangedFiles(nodes: NodeShape[]): ChangedFile[] {
  const seen = new Set<string>();
  const out: ChangedFile[] = [];
  for (const n of nodes) {
    if (n?.type !== 'tool' || !n.name || !n.path) continue;
    if (!WRITE_TOOLS.has(n.name) || !LANDED.has(n.status ?? '')) continue;
    if (seen.has(n.path)) continue;
    seen.add(n.path);
    out.push({ path: n.path, name: baseName(n.path), tool: n.name });
  }
  return out;
}

function baseName(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

const EMPTY: DockSync = { task: null, files: [], lang: 'zh', theme: 'light' };

export function useDockSyncPublisher(): void {
  const currentId = useSessionStore((s) => s.currentId);
  const sessions = useSessionStore((s) => s.sessions);
  const chat = useChatStore((s) => (currentId ? s.sessions[currentId] : undefined));
  const lang = useLang((s) => s.lang);
  const theme = useSettings((s) => s.theme);

  const payload = useMemo<DockSync>(() => {
    // No session open: no task and no changed files, but the display preferences (language, theme)
    // still have to travel -- the dock has no other way to hear about a switch made on the start
    // page, and a frozen EMPTY would pin it to the defaults instead.
    if (!currentId) return { ...EMPTY, lang, theme };
    const cur = sessions.find((s) => s.id === currentId);
    const rounds = chat?.rounds ?? [];
    const nodes: NodeShape[] = [
      ...rounds.flatMap((r) => r.nodes as unknown as NodeShape[]),
      ...((chat?.live?.nodes ?? []) as unknown as NodeShape[]),
    ];
    return {
      task: cur ? { title: cur.title || '', startedAt: cur.created_at } : null,
      files: collectChangedFiles(nodes),
      lang,
      theme,
    };
  }, [chat, currentId, sessions, lang, theme]);

  const latest = useRef(payload);
  latest.current = payload;

  const send = () => {
    if (!inTauri()) return;
    void emitTo(DOCK_LABEL, PUSH_EVENT, latest.current).catch(() => undefined);
  };

  useEffect(send, [payload]);

  useEffect(() => {
    // 浏览器里没有壳注入的 `__TAURI_INTERNALS__`，`listen` 会同步抛错 —— 跳过。
    if (!inTauri()) return;
    const pending = listen(REQUEST_EVENT, send);
    return () => {
      void pending.then((un) => un());
    };
  }, []);
}

export function useDockSync(): DockSync | null {
  const [data, setData] = useState<DockSync | null>(null);

  useEffect(() => {

    if (!inTauri()) return;
    const pending = listen<DockSync>(PUSH_EVENT, (e) => {

      if (e.payload.lang && e.payload.lang !== useLang.getState().lang) {
        useLang.getState().setLang(e.payload.lang);
      }
      if (e.payload.theme && e.payload.theme !== useSettings.getState().theme) {
        useSettings.getState().setTheme(e.payload.theme);
      }
      setData(e.payload);
    });
    void pending.then(() => emit(REQUEST_EVENT)).catch(() => undefined);
    return () => {
      void pending.then((un) => un());
    };
  }, []);

  return data;
}

/**
 * Mirror the main window's colour theme into this window's store.
 *
 * Separate windows are separate documents with their own store instance, so a theme flipped in the
 * main window never reaches the dock on its own -- it would keep whatever the persisted value was
 * at load and never follow a switch made afterwards. The dock subscribes here (the same channel it
 * already uses for the language) and applies whatever the main window publishes.
 *
 * This lives on the window itself, not in a panel: the panels unmount when a file is open, and a
 * theme switch has to land whatever the dock happens to be showing.
 */
export function useDockThemeSync(): void {
  useEffect(() => {
    if (!inTauri()) return;
    const apply = (theme?: RealTheme) => {
      if (theme && theme !== useSettings.getState().theme) useSettings.getState().setTheme(theme);
    };
    const pending = listen<DockSync>(PUSH_EVENT, (e) => apply(e.payload?.theme));
    // Ask for the current value: the dock can mount after the main window already published, and
    // the publisher only re-sends on change or on this request.
    void pending.then(() => emit(REQUEST_EVENT)).catch(() => undefined);
    return () => {
      void pending.then((un) => un());
    };
  }, []);
}
