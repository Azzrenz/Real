
import { http } from '../http';
import { EP } from '../endpoints';
import type { ScheduledJob } from '../contracts';

export const scheduleApi = {
  list: () => http.get<{ jobs: ScheduledJob[] }>(EP.schedule.list()),

  create: (input: { name: string; prompt: string; schedule: string; workspace?: string }) =>
    http.post<{ job: ScheduledJob }>(EP.schedule.create(), input),

  remove: (id: string) => http.delete<{ deleted: boolean; id: string }>(EP.schedule.remove(id)),

  toggle: (id: string, enabled = true) =>
    http.post<{ id: string; enabled: boolean }>(EP.schedule.toggle(id), { enabled }),

  run: (id: string) => http.post<{ triggered: boolean; id: string }>(EP.schedule.run(id)),
};
