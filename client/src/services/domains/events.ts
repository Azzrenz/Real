
import { http } from '../http';
import { EP } from '../endpoints';
import type { SseEvent } from '../contracts';

export const eventsApi = {
  streamUrl: (sessionId: string, after: number) => EP.events.stream(sessionId, after),

  poll: (sessionId: string, after: number) =>
    http.get<SseEvent[]>(EP.events.poll(sessionId, after)),
};

export const SSE_BASE_URL = http.baseUrl;
