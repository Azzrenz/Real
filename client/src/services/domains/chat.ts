
import { http } from '../http';
import { EP } from '../endpoints';
import type { Message } from '../contracts';

export interface AttachmentPayload {
  name: string;
  data_url: string;
}

export const chatApi = {
  send: (sessionId: string, content: string, attachments?: AttachmentPayload[]) =>
    http.post<{ accepted: boolean; session_id: string; run_id?: string; message: Message }>(
      EP.chat.send(sessionId),
      attachments && attachments.length > 0 ? { content, attachments } : { content },
    ),

  cancel: (sessionId: string) =>
    http.post<{ cancelled: boolean; run_id?: string | null; session_id: string }>(
      EP.chat.cancel(sessionId),
    ),

  interject: (sessionId: string, text: string, mode: 'append' | 'interrupt' = 'append') =>
    http.post<{ ok: boolean; queued: number }>(EP.chat.interject(sessionId), { text, mode }),

  confirm: (
    sessionId: string,
    requestId: string,
    approved: boolean,
    trustSession: boolean,
    selected?: string,
    note?: string,
  ) =>
    http.post<{ ok: boolean }>(EP.chat.confirm(sessionId), {
      request_id: requestId,
      approved,
      trust_session: trustSession,
      ...(selected ? { selected } : {}),
      ...(note ? { note } : {}),
    }),
};
