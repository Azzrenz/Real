// Lightweight toast store: replaces window.alert (blocking, jarring in Tauri WebView).

import { create } from 'zustand';

export interface ToastItem {
  id: number;
  text: string;
  kind: 'error' | 'info' | 'ok';
}

interface ToastState {
  toasts: ToastItem[];
  show: (text: string, kind?: ToastItem['kind']) => void;
  dismiss: (id: number) => void;
}

let NEXT_ID = 0;

export const useToastStore = create<ToastState>((set, get) => ({
  toasts: [],
  show: (text, kind = 'info') => {
    const id = ++NEXT_ID;
    set({ toasts: [...get().toasts, { id, text, kind }] });
    window.setTimeout(() => {
      set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) }));
    }, 4000);
  },
  dismiss: (id) => set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })),
}));
