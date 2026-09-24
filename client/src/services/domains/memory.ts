
import { http } from '../http';
import { EP } from '../endpoints';
import type { MemoryRow, MemoryStats, Persona, PreferenceRow } from '../contracts';

export const memoryApi = {
  persona: {
    get: () => http.get<Persona>(EP.memory.persona()),
    put: (value: Persona) => http.put<Persona>(EP.memory.persona(), value),
  },

  preferences: {
    list: (workspace = '') => http.get<PreferenceRow[]>(EP.memory.preferences(workspace)),
    add: (workspace: string, value: string) =>
      http.post<{ ok: boolean; error?: string }>(EP.memory.addPreference(), { workspace, value }),
    remove: (workspace: string, key: string) =>
      http.delete<{ ok: boolean }>(EP.memory.removePreference(workspace, key)),
  },

  themes: {
    list: (workspace = '') => http.get<MemoryRow[]>(EP.memory.themes(workspace)),
    pin: (workspace: string, id: string | number) =>
      http.post<{ ok: boolean; priority: number }>(EP.memory.pinTheme(), { workspace, id }),
    remove: (id: string | number) => http.delete<{ ok: boolean }>(EP.memory.removeTheme(id)),
  },

  digests: {
    list: (workspace = '') => http.get<MemoryRow[]>(EP.memory.digests(workspace)),
  },

  stats: (workspace = '') => http.get<MemoryStats>(EP.memory.stats(workspace)),
};
