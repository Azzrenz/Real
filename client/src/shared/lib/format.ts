
import { t } from '../i18n';

export function fmtTime(ts?: string): string {
  if (!ts) return '';
  const d = new Date(ts);
  if (Number.isNaN(d.getTime())) return '';
  return d.toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit', hour12: false });
}

export function fmtDateTime(ts?: string | null): string {
  if (!ts) return '';
  const d = new Date(ts);
  if (Number.isNaN(d.getTime())) return '';
  return d.toLocaleString('zh-CN', {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
  });
}

const MODEL_BRANDS: Record<string, string> = {
  deepseek: 'DeepSeek', glm: 'GLM', kimi: 'Kimi', moonshot: 'Moonshot',
  claude: 'Claude', gpt: 'GPT', qwen: 'Qwen', gemini: 'Gemini',
  minimax: 'MiniMax', doubao: 'Doubao', hunyuan: 'Hunyuan', ernie: 'ERNIE',
  llama: 'Llama', mistral: 'Mistral', grok: 'Grok', phi: 'Phi',
};

/**
 * The human-readable name of a model.
 *
 * Precedence: the label from the backend archive, then a heuristic built from the id.
 * Why the backend label must win: `deepseek-flash` is an API-level name (the model parameter
 * sent upstream) while the model is actually V4.1 Flash — no amount of string assembly produces
 * "V4.1", so the user could not tell which version they paid for. The label comes from
 * providers/*.json (single source of truth).
 *
 * The heuristic is only a fallback: older rounds may reference a model that has left the list.
 * A missing labels map is not an error, so older call sites keep working unchanged.
 */
export function prettyModel(model: string, labels?: Record<string, string>): string {
  const mapped = labels?.[model];
  if (mapped) return mapped;
  return model
    .split(/[-_]/)
    .filter(Boolean)
    .map((seg) => MODEL_BRANDS[seg.toLowerCase()] ?? seg.charAt(0).toUpperCase() + seg.slice(1))
    .join(' ');
}

export function countLines(s: string): number {
  if (!s) return 0;
  let n = 1;
  for (let i = 0; i < s.length; i++) if (s.charCodeAt(i) === 10) n++;
  return n;
}

/** Round duration in human form: seconds under a minute, minutes under an hour. */
export function fmtDuration(ms: number): string {
  const sec = Math.max(0, Math.round(ms / 1000));
  if (sec < 60) return t('{n} 秒', { n: sec });
  const min = Math.floor(sec / 60);
  const rest = sec % 60;
  if (min < 60) return rest > 0 ? t('{m} 分 {s} 秒', { m: min, s: rest }) : t('{m} 分', { m: min });
  const hr = Math.floor(min / 60);
  const restMin = min % 60;
  return restMin > 0 ? t('{h} 小时 {m} 分', { h: hr, m: restMin }) : t('{h} 小时', { h: hr });
}
