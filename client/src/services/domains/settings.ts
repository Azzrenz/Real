
import { http } from '../http';
import { EP } from '../endpoints';
import type { Settings, SettingsUpdate, TestResult } from '../contracts';

export const settingsApi = {
  get: () => http.get<Settings>(EP.settings.get()),
  put: (patch: SettingsUpdate) => http.put<Settings>(EP.settings.put(), patch),
  test: () => http.post<TestResult>(EP.settings.test()),
  clearApiKey: () => http.put<Settings>(EP.settings.put(), { api_key: '' }),
};
