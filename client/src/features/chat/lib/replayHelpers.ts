
import type { ToolCard } from '../../tools/types';


export interface AsstTool {
  callId: string;
  name: string;
  args: string;
}

export function mapToolStatus(status?: string): ToolCard['status'] {
  if (status === 'error' || status === 'timeout') return 'error';
  return 'success';
}

export function parseArgs(tcArgs?: string, fallback?: string): unknown {
  const raw = typeof tcArgs === 'string' && tcArgs ? tcArgs : typeof fallback === 'string' ? fallback : '{}';
  try {
    const o = JSON.parse(raw);
    if (o && typeof o === 'object') return o;
  } catch {
 /* keep raw string */
  }
  return raw;
}

export function toolSummary(resultJson?: string | null): string | undefined {
  if (!resultJson) return undefined;
  try {
    const o = JSON.parse(resultJson) as {
      render_full?: unknown;
      content?: Array<{ text?: unknown }>;
    };
    if (typeof o.render_full === 'string' && o.render_full.trim()) {
      return o.render_full;
    }
    const first = Array.isArray(o.content) ? o.content[0] : undefined;
    if (first && typeof first.text === 'string' && first.text.trim()) {
      const t = first.text.trim();
      return t.length > 4000 ? t.slice(0, 4000) + '…' : t;
    }
  } catch {
 /* ignore */
  }
  return undefined;
}
