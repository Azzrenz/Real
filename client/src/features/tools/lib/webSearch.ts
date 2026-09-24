
import { salvageTruncatedJson } from './jsonSalvage';

export interface WebSearchResult {
  title: string;
  time: string;
  snippet: string;
  url: string;
}

export interface WebSearchParsed {
  query: string;
  results: WebSearchResult[];
}

export function parseWebSearchSummary(text: string): WebSearchParsed | null {
  const s = text.trimStart();
  if (!s.startsWith('{')) return null;
  let v: Record<string, unknown> | undefined;
  try {
    v = JSON.parse(s) as Record<string, unknown>;
  } catch {
    v = salvageTruncatedJson(s);
  }
  const data = v?.data;
  if (!data || typeof data !== 'object') return null;
  const d = data as { query?: unknown; results?: unknown };
  const rawResults = Array.isArray(d.results) ? d.results : [];
  const results: WebSearchResult[] = [];
  for (const r of rawResults) {
    if (!r || typeof r !== 'object') continue;
    const o = r as Record<string, unknown>;
    let title = typeof o.title === 'string' ? o.title.trim() : '';
    let time = '';
    const m = title.match(/（发布时间：([^）]+)）$/);
    if (m) {
      time = m[1].trim();
      title = title.slice(0, m.index).trim();
    }
    const snippet = typeof o.snippet === 'string' ? o.snippet.trim() : '';
    const url = typeof o.url === 'string' ? o.url.trim() : '';
    if (!title && !snippet && !url) continue;
    results.push({ title, time, snippet, url });
  }
  return { query: typeof d.query === 'string' ? d.query : '', results };
}
