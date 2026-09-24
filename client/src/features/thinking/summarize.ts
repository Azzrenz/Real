
import { SUMMARY_MAX_CHARS } from './contracts';

const PRELUDE_RE = /^(好[的吧嘞]?|嗯+|行|可以|明白|收到|那么|我[来去]|让我|咱们|哦|啊)[\s，。、,.!！?？~～]*$/;

function stripMarks(line: string): string {
  return line
    .replace(/^\s*#{1,6}\s+/, '')
    .replace(/^\s*>\s?/, '')
    .replace(/^\s*[-*+•]\s+/, '')
    .replace(/^\s*\d{1,2}[.)、]\s+/, '')
    .replace(/[`*]/g, '')
    .trim();
}

function clip(s: string): string {
  const chars = Array.from(s);
  return chars.length > SUMMARY_MAX_CHARS ? chars.slice(0, SUMMARY_MAX_CHARS).join('') + '…' : s;
}

export function summarizeThinking(body: string): string {
  if (!body) return '';
  const lines = body.split('\n');
  for (const raw of lines) {
    const line = stripMarks(raw);
    if (!line) continue;
    if (line.length <= 8 && PRELUDE_RE.test(line)) continue;
    return clip(line);
  }
  return '';
}
