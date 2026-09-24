
import type { ThinkRow } from '../chat/store/chatStore';

export type { ThinkRow };

export const BODY_CHAR_BUDGET = 4000;

export const SUMMARY_MAX_CHARS = 48;

export function isThinkNoteNoise(note: string): boolean {
  const s = note.trim();
  if (!s.startsWith('正在')) return false;
  const parts = s.split('；');
  if (parts.length < 2) return false;
  return parts.every((p) => /^正在\S+$/.test(p.trim()));
}
