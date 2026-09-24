
import { http } from '../http';
import { EP } from '../endpoints';
import type { Session, SessionDetail, SseEvent } from '../contracts';

export const sessionsApi = {
  list: () => http.get<{ sessions: Session[] }>(EP.sessions.list()),

  create: (title: string, systemPrompt: string) =>
    http.post<{ session: Session }>(EP.sessions.create(), { title, system_prompt: systemPrompt }),

  detail: (id: string, withEvents = false) =>
    http.get<SessionDetail>(EP.sessions.detail(id, withEvents)),

  rename: (id: string, title: string) =>
    http.patch<{ session: Session }>(EP.sessions.patch(id), { title }),

  remove: (id: string) => http.delete<{ deleted: boolean; id: string }>(EP.sessions.remove(id)),

  thinking: (id: string, from: number, to: number) =>
    http.get<{ text: string; len: number }>(EP.sessions.thinking(id, from, to)),

  /** Page back through events ("load earlier"): pass the seq of the earliest loaded one. */
  events: (id: string, before: number) =>
    http.get<{ events: SseEvent[]; has_earlier: boolean }>(EP.sessions.events(id, before)),
};
