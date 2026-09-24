
import { create } from 'zustand';
import { scheduleApi } from '../../../services/domains/schedule';
import type { ScheduledJob } from '../../../services/contracts';

interface ScheduleState {
  jobs: ScheduledJob[];
  loading: boolean;
  error: string | null;

  load: () => Promise<void>;
  create: (input: { name: string; prompt: string; schedule: string; workspace?: string }) => Promise<boolean>;
  remove: (id: string) => Promise<boolean>;
  toggle: (id: string, enabled: boolean) => Promise<void>;
  run: (id: string) => Promise<boolean>;
}

export const useScheduleStore = create<ScheduleState>((set, get) => ({
  jobs: [],
  loading: false,
  error: null,

  load: async () => {
    set({ loading: true, error: null });
    try {
      const res = await scheduleApi.list();
      set({ jobs: res.jobs ?? [], loading: false });
    } catch (e) {
      set({ loading: false, error: (e as Error).message });
    }
  },

  create: async (input) => {
    try {
      const res = await scheduleApi.create(input);
      set({ jobs: [res.job, ...get().jobs] });
      return true;
    } catch (e) {
      set({ error: (e as Error).message });
      return false;
    }
  },

  remove: async (id) => {
    try {
      await scheduleApi.remove(id);
      set({ jobs: get().jobs.filter((j) => j.id !== id) });
      return true;
    } catch (e) {
      set({ error: (e as Error).message });
      return false;
    }
  },

  toggle: async (id, enabled) => {
    try {
      const res = await scheduleApi.toggle(id, enabled);
      set({ jobs: get().jobs.map((j) => (j.id === id ? { ...j, enabled: res.enabled } : j)) });
    } catch (e) {
      set({ error: (e as Error).message });
    }
  },

  run: async (id) => {
    try {
      await scheduleApi.run(id);
      return true;
    } catch (e) {
      set({ error: (e as Error).message });
      return false;
    }
  },
}));
