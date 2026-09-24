// UI language preference. Kept separate from the settings store so the i18n

import { create } from 'zustand';
import { persist } from 'zustand/middleware';

export type Lang = 'zh' | 'en';

export const LANGS: Array<{ id: Lang; label: string }> = [
  { id: 'zh', label: '简体中文' },
  { id: 'en', label: 'English' },
];

interface LangState {
  lang: Lang;
  setLang: (lang: Lang) => void;
}

export const useLang = create<LangState>()(
  persist(
    (set) => ({
      lang: 'en',
      setLang: (lang) => {
        set({ lang });
        void pushLangToBackend(lang);
      },
    }),
    {
      // v2: the default flipped to English, so the old key must not keep an 'zh' alive.
      name: 'real-lang-v2',
      // On boot, push the persisted UI language so the backend can't stay on its default
      // while the UI is already English (that mismatch is exactly what this feature fixes).
      onRehydrateStorage: () => (state) => {
        if (state) void pushLangToBackend(state.lang);
      },
    },
  ),
);

/**
 * Mirror the UI language into the backend global setting (`settings` table `output_lang`).
 * The backend reads it every turn, so model-produced thinking / narration / tool-card text
 * follows the UI language — no need to carry a per-message `lang` field (see chat service).
 * Fire-and-forget; a dynamic import keeps `shared/i18n` out of a cycle with `services/http`.
 */
function pushLangToBackend(lang: Lang): Promise<void> {
  return import('../../services/domains/settings')
    .then(({ settingsApi }) => settingsApi.put({ output_lang: lang }))
    .then(() => undefined)
    .catch(() => undefined);
}
