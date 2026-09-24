import { useEffect, useRef } from 'react';
import type { Attachment } from '../lib/attachments';

// Draft persistence: newest 20 sessions only, written with a 400ms debounce so
const DRAFT_KEY = 'real-drafts';
const DRAFT_KEEP = 20;
const DRAFT_DEBOUNCE_MS = 400;

export interface Draft {
  text: string;
  atts: Attachment[];
  ts: number;
}

function readAll(): Record<string, Draft> {
  try {
    return JSON.parse(localStorage.getItem(DRAFT_KEY) ?? '{}') as Record<string, Draft>;
  } catch {
    return {};
  }
}

export function readDraft(sid: string): Draft | null {
  return readAll()[sid] ?? null;
}

export function clearDraft(sid: string): void {
  const all = readAll();
  if (!all[sid]) return;
  delete all[sid];
  localStorage.setItem(DRAFT_KEY, JSON.stringify(all));
}

function write(sid: string, text: string, atts: Attachment[]): void {
  try {
    const all = readAll();
    if (text.trim() || atts.length > 0) {
      all[sid] = { text, atts, ts: Date.now() };
    } else {
      delete all[sid];
    }
    const kept = Object.entries(all)
      .sort((a, b) => b[1].ts - a[1].ts)
      .slice(0, DRAFT_KEEP);
    localStorage.setItem(DRAFT_KEY, JSON.stringify(Object.fromEntries(kept)));
  } catch {
    // Quota / serialization failure: keep the text at least, drop attachments.
    try {
      const all = readAll();
      all[sid] = { text, atts: [], ts: Date.now() };
      localStorage.setItem(DRAFT_KEY, JSON.stringify(all));
    } catch { /* ignore */ }
  }
}

/** Keeps the composer's text per session. Debounced while typing, but written at once on */
export function useDraftSaver(sessionId: string, input: string, attachments: Attachment[]): void {
  const latest = useRef({ text: input, atts: attachments });
  latest.current = { text: input, atts: attachments };

  useEffect(() => {
    const timer = setTimeout(() => write(sessionId, input, attachments), DRAFT_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [input, attachments, sessionId]);

  useEffect(
    () => () => write(sessionId, latest.current.text, latest.current.atts),
    [sessionId],
  );
}

// A draft older than the newest user message means it was already sent (or the
export function takeStaleDraft(sid: string, lastUserAt: number): Draft | null {
  const all = readAll();
  const d = all[sid];
  if (!d || typeof d.ts !== 'number' || d.ts >= lastUserAt) return null;
  delete all[sid];
  try {
    localStorage.setItem(DRAFT_KEY, JSON.stringify(all));
  } catch {
    /* ignore */
  }
  return d;
}
