import { create } from 'zustand';

const WIDTH_KEY = 'real.dock-panel-width';
export const DOCK_PANEL_MIN = 260;
export const DOCK_PANEL_MAX_RATIO = 0.7;
export const DOCK_PANEL_DEFAULT = 420;
/**
 * Floor for everything left of the panel: the session sidebar (`--sidebar-w: 261`)
 * plus a usable chat column (~420). The panel may never eat into this.
 *
 * It covers the sidebar too because the panel, the sidebar and the chat column are
 * flex siblings -- a floor on "window minus panel" alone would still let the chat
 * column collapse once the sidebar is counted. Slightly conservative while the
 * sidebar is collapsed; that is the harmless direction.
 */
export const MIDDLE_MIN = 680;

/**
 * The width the panel is allowed to take. **Single source of the width policy** --
 * recall, drag and render all go through it.
 *
 * Why it must be clamped here and not only on drag: the width is *remembered*. A
 * value stored on a wide window comes straight back on a narrow one, and the panel
 * is `flex: none` while the chat column is `min-width: 0` -- so the chat column is
 * the one that absorbs the whole loss and can be squeezed to nothing.
 *
 * Ceiling = min(70% of the window, window - MIDDLE_MIN), floored at DOCK_PANEL_MIN.
 */
export function clampPanelWidth(w: number, winW: number): number {
  const ceiling = Math.min(winW * DOCK_PANEL_MAX_RATIO, winW - MIDDLE_MIN);
  const max = Math.max(DOCK_PANEL_MIN, Math.floor(ceiling));
  const want = Number.isFinite(w) ? Math.round(w) : DOCK_PANEL_DEFAULT;
  return Math.max(DOCK_PANEL_MIN, Math.min(max, want));
}

function recallWidth(): number {
  try {
    const n = Number(localStorage.getItem(WIDTH_KEY));
    const want = Number.isFinite(n) && n >= DOCK_PANEL_MIN ? n : DOCK_PANEL_DEFAULT;
    return clampPanelWidth(want, window.innerWidth);
  } catch {
    return DOCK_PANEL_DEFAULT;
  }
}

interface DockPanelState {
  open: boolean;
  path: string | null;
  files: string[];
  width: number;
  setOpen: (v: boolean) => void;
  toggle: () => void;
  openFile: (p: string) => void;
  selectFile: (p: string) => void;
  closeFile: (p: string) => void;
  setWidth: (w: number) => void;
}

export const useDockPanel = create<DockPanelState>()((set, get) => ({
  open: false,
  path: null,
  files: [],
  width: recallWidth(),
  setOpen: (v) => set({ open: v }),
  toggle: () => set((s) => ({ open: !s.open })),
  openFile: (p) =>
    set((s) => ({
      open: true,
      files: s.files.includes(p) ? s.files : [...s.files, p],
      path: p,
    })),
  selectFile: (p) => set({ path: p, open: true }),
  closeFile: (p) => {
    const { files, path } = get();
    const idx = files.indexOf(p);
    const next = files.filter((x) => x !== p);
    set({
      files: next,
      path: path === p ? (next.length ? next[Math.min(idx, next.length - 1)] : null) : path,
    });
  },
  setWidth: (w) => {
    const clamped = clampPanelWidth(w, window.innerWidth);
    try {
      localStorage.setItem(WIDTH_KEY, String(clamped));
    } catch {
      void 0;
    }
    set({ width: clamped });
  },
}));
