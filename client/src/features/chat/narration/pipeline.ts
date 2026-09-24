
export const LEAD_PUNCT = /^[\s，。、；：！？…·,.!?;:]+/;

export function isNarrationNoise(s: string): boolean {
  return s.trim() === '';
}

export const VERBAL_PRELUDE_RE = /^(我[来去]?|你|您|咱们|让我|好的?|好[的吧]?|行|可以|嗯|明白|好嘞|开始吧|稍等)[\s。，.!?,，~～]*$/;
export function isVerbalPrelude(s: string): boolean {
  const t = s.replace(LEAD_PUNCT, '').trim();
  if (!t) return false;
  if (t.length > 10) return false;
  if (/[。！？\n]/.test(t)) return false;
  return VERBAL_PRELUDE_RE.test(t);
}

export const SENT_END = /[。！？.!?\n]/;
export function cutAtSentenceEnd(s: string): { head: string; tail: string } {
  for (let i = s.length - 1; i >= 0; i--) {
    const c = s[i];
    if (!SENT_END.test(c)) continue;
    if (c === '.') {
      let bt = 0;
      for (let k = 0; k < i; k++) if (s[k] === '`') bt ^= 1;
      if (bt & 1) continue;
      const prev = s[i - 1] ?? '';
      const next = s[i + 1] ?? '';
      const isAlnum = (ch: string) => /[A-Za-z0-9]/.test(ch);
      const isCjk = (ch: string) => /[一-鿿㐀-䶿]/.test(ch);
      if (isAlnum(next) && (isAlnum(prev) || isCjk(prev))) continue;
    }
    return { head: s.slice(0, i + 1), tail: s.slice(i + 1).trim() };
  }
  return { head: '', tail: '' };
}

export const narTail: Record<string, string> = {};

export function completeSentences(text: string): string {
  return cutAtSentenceEnd(text).head;
}

export function hasBlockMarkdown(text: string): boolean {
  if (!text) return false;
  if (text.includes('```')) return true;
  if (/(^|\n)\s*#{1,6}\s*\S/.test(text)) return true;
  const pipeLines = text.match(/(^|\n)\s*\|/g);
  if (pipeLines && pipeLines.length >= 2) return true;
  // Ordered / unordered lists: plans and step lists arrive as "1. ... 2. ...". Without this
  // they fall through to the plain-text path, so the numbering never becomes a real list.
  if (/(^|\n)[ \t]*(?:\d+[.、)]\s+\S|[-*+]\s+\S)/.test(text)) return true;
  return false;
}
