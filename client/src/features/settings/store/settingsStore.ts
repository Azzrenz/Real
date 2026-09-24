
import { create } from 'zustand';
import { persist } from 'zustand/middleware';
import { settingsApi } from '../../../services/domains/settings';
import type { Settings as BackendSettings, SettingsUpdate, TestResult, ModelLabel, ProviderInfo, TuningSnapshot } from '../../../services/contracts';
import { prettyModel } from '../../../shared/lib/format';
import { t } from '../../../shared/i18n';
import { useSessionStore } from '../../sessions/store/sessionStore';

export type RealTheme = 'dark' | 'light';

export const FALLBACK_MODELS = [
  'deepseek-flash',
  'glm-5.3-flash',
];

/** model id → human-readable version name (backend archive is the source of truth). */
function labelMap(list?: ModelLabel[]): Record<string, string> {
  return Object.fromEntries((list ?? []).map((m) => [m.id, m.label]));
}

interface SettingsState {
  theme: RealTheme;
  apiKey: string;
  baseUrl: string;
  model: string;
  mode: 'mock' | 'real';
  defaultSystemPrompt: string;
  thinkingMode: string;
  thinkingEffort: string;
  /** Memory scope: shared = across sessions in one workspace; session = this task only. */
  memoryScope: string;
  sidebarCollapsed: boolean;
  autoApproveDanger: boolean;
  models: string[];
  /** model id → version name (backend model_labels; used wherever a human reads it). */
  modelLabels: Record<string, string>;
  /** Provider groups (backend providers); adding a provider means adding its JSON. */
  providers: ProviderInfo[];
  /** Provider id → whether a credential exists, which decides if we prompt for a key. */
  providerHasKey: Record<string, boolean>;
  glmReady: boolean;

  maxRounds: string;
  compactTrigger: string;
  compactKeepRaw: string;
  compactPressureChars: string;
  taskKeepOutputs: string;
  archiveKeepRecent: string;
  maxOutputTokens: string;
  taskStubMinBytes: string;
  readSpillThreshold: string;
  readLinesSpillThreshold: string;
  spillPreviewHead: string;
  spillPreviewTail: string;
  spillMaxFileBytes: string;
  /** Factory defaults from the backend; the panel's "default X" reads from here. */
  tuningDefaults: TuningSnapshot | null;
  /** Advanced parameters unlocked (local UI preference, persisted to localStorage). */
  advancedUnlocked: boolean;

  backend: BackendSettings | null;
  saving: boolean;
  savedMsg: string | null;
  testing: boolean;
  testResult: TestResult | null;

  setTheme: (t: RealTheme) => void;
  toggleTheme: () => void;
  toggleSidebar: () => void;
  toggleAutoApproveDanger: () => void;
  toggleAdvanced: () => void;
  patch: (p: Partial<Pick<SettingsState, 'apiKey' | 'baseUrl' | 'model' | 'defaultSystemPrompt' | 'thinkingMode' | 'thinkingEffort' | 'memoryScope' | 'maxRounds' | 'compactTrigger' | 'compactKeepRaw' | 'compactPressureChars' | 'taskKeepOutputs' | 'archiveKeepRecent' | 'maxOutputTokens' | 'taskStubMinBytes' | 'readSpillThreshold' | 'readLinesSpillThreshold' | 'spillPreviewHead' | 'spillPreviewTail' | 'spillMaxFileBytes'>>) => void;
  loadFromBackend: () => Promise<void>;
  saveToBackend: () => Promise<void>;
  clearApiKey: () => Promise<void>;
  testConnection: () => Promise<void>;
  /** Switch model: with a sessionId only that session changes, otherwise the global default. */
  switchModel: (model: string, apiKey?: string, sessionId?: string) => Promise<void>;
}

function tuningPatch(s: SettingsState, u: SettingsUpdate) {
  // No backend snapshot yet: buffers may still hold local defaults, so sending them
  // would overwrite the server-side tuning values on an unrelated save.
  if (!s.backend) return;
  const map: Array<[keyof SettingsState, keyof SettingsUpdate]> = [
    ['maxRounds', 'max_rounds'],
    ['compactTrigger', 'compact_trigger'],
    ['compactKeepRaw', 'compact_keep_raw'],
    ['compactPressureChars', 'compact_pressure_chars'],
    ['taskKeepOutputs', 'task_keep_outputs'],
    ['archiveKeepRecent', 'archive_keep_recent'],
    ['maxOutputTokens', 'max_output_tokens'],
    ['taskStubMinBytes', 'task_stub_min_bytes'],
    ['readSpillThreshold', 'read_spill_threshold_chars'],
    ['readLinesSpillThreshold', 'read_lines_spill_threshold_chars'],
    ['spillPreviewHead', 'spill_preview_head_chars'],
    ['spillPreviewTail', 'spill_preview_tail_chars'],
    ['spillMaxFileBytes', 'spill_max_file_bytes'],
  ];
  for (const [buf, wire] of map) {
    const raw = String(s[buf] ?? '').trim();
    if (raw === '') continue;
    const n = Number(raw);
    if (Number.isFinite(n)) (u as Record<string, unknown>)[wire] = n;
  }
}

/**
 * Backend snapshot → input buffers.
 * A key the backend omits leaves the buffer empty (= leave that setting alone). Never invent
 * numbers here: hard-coded fallbacks drift from the backend constants and would write stale
 * values back to the server whenever the backend does not answer.
 */
function tuningBuffersFrom(b: BackendSettings): Pick<SettingsState, 'maxRounds' | 'compactTrigger' | 'compactKeepRaw' | 'compactPressureChars' | 'taskKeepOutputs' | 'archiveKeepRecent' | 'maxOutputTokens' | 'taskStubMinBytes' | 'readSpillThreshold' | 'readLinesSpillThreshold' | 'spillPreviewHead' | 'spillPreviewTail' | 'spillMaxFileBytes'> {
  const t = (v?: number) => (v == null ? '' : String(v));
  return {
    maxRounds: t(b.max_rounds),
    compactTrigger: t(b.compact_trigger),
    compactKeepRaw: t(b.compact_keep_raw),
    compactPressureChars: t(b.compact_pressure_chars),
    taskKeepOutputs: t(b.task_keep_outputs),
    archiveKeepRecent: t(b.archive_keep_recent),
    maxOutputTokens: t(b.max_output_tokens),
    taskStubMinBytes: t(b.task_stub_min_bytes),
    readSpillThreshold: t(b.read_spill_threshold_chars),
    readLinesSpillThreshold: t(b.read_lines_spill_threshold_chars),
    spillPreviewHead: t(b.spill_preview_head_chars),
    spillPreviewTail: t(b.spill_preview_tail_chars),
    spillMaxFileBytes: t(b.spill_max_file_bytes),
  };
}

export const useSettings = create<SettingsState>()(
  persist(
    (set, get) => ({
      theme: 'light',
      apiKey: '',
      baseUrl: 'https://api.deepseek.com',
      model: 'glm-5.3-flash',
      mode: 'mock',
      defaultSystemPrompt: '',
      thinkingMode: 'auto',
      // Must equal the backend default (REAL_THINKING_EFFORT=low in server/src/config.rs):
      // any divergence here writes the wrong value into the DB the moment settings are saved.
      thinkingEffort: 'low',
      memoryScope: 'shared',
      sidebarCollapsed: true,
      autoApproveDanger: false,
      models: FALLBACK_MODELS,
      modelLabels: {},
      providers: [],
      providerHasKey: {},
      glmReady: false,
      /**
        * Tuning input buffers: an empty string means "not fetched yet" or "leave unchanged".
         * The backend is the single source of truth for every number; never invent one here.
         */
      maxRounds: '',
      compactTrigger: '',
      compactKeepRaw: '',
      compactPressureChars: '',
      taskKeepOutputs: '',
      archiveKeepRecent: '',
      maxOutputTokens: '',
      taskStubMinBytes: '',
      readSpillThreshold: '',
      readLinesSpillThreshold: '',
      spillPreviewHead: '',
      spillPreviewTail: '',
      spillMaxFileBytes: '',
      tuningDefaults: null,
      advancedUnlocked: false,

      backend: null,
      saving: false,
      savedMsg: null,
      testing: false,
      testResult: null,

      setTheme: (theme) => set({ theme }),
      toggleTheme: () => set((s) => ({ theme: s.theme === 'dark' ? 'light' : 'dark' })),
      toggleSidebar: () => set((s) => ({ sidebarCollapsed: !s.sidebarCollapsed })),
      toggleAutoApproveDanger: () => set((s) => ({ autoApproveDanger: !s.autoApproveDanger })),
      toggleAdvanced: () => set((s) => ({ advancedUnlocked: !s.advancedUnlocked })),
      patch: (p) => set(p),

      loadFromBackend: async () => {
        try {
          const s = await settingsApi.get();
          set({
            backend: s,
            mode: s.llm_mode,
            model: s.model,
            baseUrl: s.base_url,
            models: s.models || [],
            modelLabels: labelMap(s.model_labels),
            providers: s.providers || [],
            providerHasKey: (s.provider_has_key || {}) as Record<string, boolean>,
            glmReady: !!s.has_glm_key,
            defaultSystemPrompt: s.default_system_prompt,
            thinkingMode: s.thinking_mode || 'auto',
            thinkingEffort: s.thinking_effort || 'low',
            memoryScope: s.memory_scope || 'shared',
            tuningDefaults: s.tuning_defaults ?? null,
            ...tuningBuffersFrom(s),
          });
        } catch (e) {
          console.warn('[settings] 后端设置拉取失败（后端未启动？）', e);
          set({ backend: null, models: FALLBACK_MODELS, modelLabels: {}, providers: [], providerHasKey: {} });
        }
      },

      saveToBackend: async () => {
        const { apiKey, baseUrl, model, defaultSystemPrompt, thinkingMode, thinkingEffort, memoryScope } = get();
        // Carry the current session along: the backend writes both the session row and the global
        // default. The composer's model chip reads the session level while this panel reads the
        // global one, so both layers have to be written for the two to stay in step.
        const currentId = useSessionStore.getState().currentId;
        const payload: SettingsUpdate = {
          base_url: baseUrl,
          model,
          default_system_prompt: defaultSystemPrompt,
          thinking_mode: thinkingMode,
          thinking_effort: thinkingEffort,
          memory_scope: memoryScope,
          ...(apiKey.trim() ? { api_key: apiKey.trim() } : {}),
          ...(currentId ? { session_id: currentId } : {}),
        };
        tuningPatch(get(), payload);
        set({ saving: true, savedMsg: null });
        try {
          const s = await settingsApi.put(payload);
          set({
            backend: s,
            apiKey: '',
            mode: s.llm_mode,
            model: s.model,
            baseUrl: s.base_url,
            models: s.models || [],
            modelLabels: labelMap(s.model_labels),
            providers: s.providers || [],
            providerHasKey: (s.provider_has_key || {}) as Record<string, boolean>,
            defaultSystemPrompt: s.default_system_prompt,
            thinkingMode: s.thinking_mode || 'auto',
            thinkingEffort: s.thinking_effort || 'low',
            memoryScope: s.memory_scope || 'shared',
            tuningDefaults: s.tuning_defaults ?? null,
            ...tuningBuffersFrom(s),
            savedMsg: t('已保存，即时生效（无需重启）'),
          });
        } catch (e) {
          set({ savedMsg: t('保存失败：{err}', { err: (e as Error).message }) });
        } finally {
          set({ saving: false });
        }
      },

      clearApiKey: async () => {
        set({ saving: true });
        try {
          const s = await settingsApi.clearApiKey();
          set({ backend: s, apiKey: '', mode: s.llm_mode, savedMsg: t('API Key 已清除，已切回 Mock 模式') });
        } catch (e) {
          set({ savedMsg: t('清除失败：{err}', { err: (e as Error).message }) });
        } finally {
          set({ saving: false });
        }
      },

      testConnection: async () => {
        set({ testing: true, testResult: null });
        try {
          const r = await settingsApi.test();
          set({ testResult: r });
        } catch (e) {
          set({ testResult: { ok: false, url: get().baseUrl, error: (e as Error).message } });
        } finally {
          set({ testing: false });
        }
      },

      switchModel: async (model, apiKey, sessionId) => {
        set({ saving: true, savedMsg: null });
        try {
          const payload: SettingsUpdate = {
            model,
            ...(apiKey && apiKey.trim() ? { api_key: apiKey.trim() } : {}),
            ...(sessionId ? { session_id: sessionId } : {}),
          };
          const s = await settingsApi.put(payload);
          set({
            backend: s,
            apiKey: '',
            mode: s.llm_mode,
            // `model` here stays the GLOBAL default on purpose: a session-scoped switch
            // must not move the value every other session falls back to. The chip shows
            // the session's own model instead (see ModelPicker).
            model: s.model,
            baseUrl: s.base_url,
            models: s.models || [],
            modelLabels: labelMap(s.model_labels),
            providers: s.providers || [],
            providerHasKey: (s.provider_has_key || {}) as Record<string, boolean>,
            glmReady: !!s.has_glm_key,
            savedMsg: t('已切换到 {name}（即时生效）', {
              name: prettyModel(s.model, labelMap(s.model_labels)),
            }),
          });
        } catch (e) {
          set({ savedMsg: t('切换失败：{err}', { err: (e as Error).message }) });
          throw e;
        } finally {
          set({ saving: false });
        }
      },
    }),
    {
      name: 'real-settings-v2',
      partialize: (s) =>
        ({
          theme: s.theme,
          autoApproveDanger: s.autoApproveDanger,
          advancedUnlocked: s.advancedUnlocked,
        } as SettingsState),
      // The sidebar always starts collapsed: panel width is a fixed part of the
      // default layout, so a stale "expanded" from the last run must not win.
      merge: (persisted, current) => ({
        ...current,
        ...(persisted as Partial<SettingsState>),
        sidebarCollapsed: true,
      }),
    },
  ),
);
