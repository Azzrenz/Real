import { useEffect } from 'react';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';

import { inTauri } from '../../shared/lib/tauri';

export const DOCK_RESIZE_MAIN = 'dock:resize-main';

export type DockResizeEdge = 'north' | 'south' | 'west';

export function useMainResizeFromDock(): void {
  useEffect(() => {
    // 浏览器里没有壳注入的 `__TAURI_INTERNALS__`，`listen` 会同步抛错 —— 跳过。
    if (!inTauri()) return;
    const pending = listen<DockResizeEdge>(DOCK_RESIZE_MAIN, (e) => {

      const dir = e.payload === 'north' ? 'North' : e.payload === 'west' ? 'East' : 'South';
      void getCurrentWindow()
        .startResizeDragging(dir)
        .catch(() => undefined);
    });
    return () => {
      void pending.then((un) => un());
    };
  }, []);
}
