
import type { ToolCard } from '../types';
import { parseArgs } from './labels';
import type { T } from '../../../shared/i18n';

export function cmdShort(command: string): string {
  const s = command.trim().replace(/\s+/g, ' ');
  if (s.length <= 24) return s;
  const cut = s.slice(0, 24);
  return cut.includes(' ') ? cut.slice(0, cut.lastIndexOf(' ')) : cut;
}

function scriptEntity(primary: string): string {
  const m = primary.match(/(?:python3?(?:\.exe)?|uv\s+run(?:\s+python3?(?:\.exe)?)?)\s+["']?([A-Za-z0-9_./\\:-]+\.(?:py|ps1|sh|js))/i)
    || primary.match(/\b(?:[A-Za-z0-9_./\\:-]*[\\/])?([A-Za-z0-9_][A-Za-z0-9_-]*\.(?:py|ps1|sh|js))\b/i);
  if (!m) return '';
  return m[1].split(/[\\/]/).pop() ?? '';
}

function testEntity(primary: string): string {
  const m = primary.match(/(?:pytest|cargo test|npm (?:run )?test|vitest|jest)\s+(?:-[^\s]+\s+)*([^\s]+)/i);
  const t = m?.[1] ?? '';
  return t && !t.startsWith('-') ? t.split(/[\\/]/).pop() ?? '' : '';
}

function cmdEntity(tool: ToolCard): string {
  const argsObj = parseArgs(tool.args);
  const command = typeof argsObj?.command === 'string' ? argsObj.command : '';
  if (!command) return '';
  const primary = command.split(/&&|\|\|/)[0].split('|')[0].trim();
  if (!primary) return '';
  return scriptEntity(primary) || testEntity(primary) || cmdShort(primary);
}

function fileEntity(tool: ToolCard): string {
  if (!tool.path) return '';
  return tool.path.split(/[\\/]/).pop() || tool.path;
}

function lineSuffix(tool: ToolCard): string {
  if (tool.name !== 'modify' && tool.name !== 'edit') return '';
  const args = parseArgs(tool.args) as Record<string, unknown> | null;
  const cs = args?.change_spec as Record<string, unknown> | undefined;
  const line = cs?.line;
  return (typeof line === 'number' || typeof line === 'string') && line !== '' ? `:${line}` : '';
}

export function toolEntity(tool: ToolCard): string {
  const isCmd = tool.name === 'run' || tool.name === 'verify';
  if (isCmd) return cmdEntity(tool);
  const f = fileEntity(tool);
  return f ? `${f}${lineSuffix(tool)}` : '';
}

export function tidyPurpose(text: string): string {
  let t = (text ?? '').trim();
  if (!t) return '';
  t = t.replace(/（[^）]*）/g, '').replace(/\([^)]*\)/g, '');
  t = t.replace(/[。．.!！?？：:；;，,、~～…]+$/, '');
  t = t.replace(/\s+/g, ' ').trim();
  return t;
}

export function durationText(ms: number | undefined, t: T): string {
  // Below a minute the output is digits plus a unit suffix, which reads the same in both languages.
  if (typeof ms !== 'number' || ms < 1000) return '';
  const s = ms / 1000;
  if (s < 60) return `${s < 10 ? s.toFixed(1) : Math.round(s)}s`;
  const m = Math.floor(s / 60);
  const rest = Math.round(s - m * 60);
  return rest > 0 ? t('{m}分{r}秒', { m, r: rest }) : t('{m}分', { m });
}

export function titleCompact(s: string): string {
  let t = (s ?? '').trim();
  if (!t) return '';
  t = t.replace(/\*+/g, '').replace(/`+/g, '').trim();
  t = t.replace(
    /^(?:先(?:要|去|把|给|将)?|然后|接着|再(?:去|把|给|将)?|去(?=[把给])|并(?:且)?|顺带|顺便|同时|以及|和|继续|现在|接下来|需要|要去|把|给|将|但|可是|不过|然而|只是|却)+(?:，|、)?/,
    '',
  );
  t = t.replace(/(?<=[\u4e00-\u9fa5A-Za-z0-9])(\s)?的(?=[\u4e00-\u9fa5])/g, '$1');
  t = t.replace(/(?:即可|就行|了)+[。；;]?$/, '');
  const out = t.replace(/\s+/g, ' ').trim();
  return out || (s ?? '').trim();
}

const ACTION_VERBS =
  /(修复|验证|排查|重写|替换|统一|编译|定位|检索|扫描|确认|实现|补充|删除|清理|重构|抽取|检查|审查|盘点|摸清|精修|恢复|接入|退役|收敛|迁移|提取|核对|比对|落地|改造|修正|补齐|打通|收口|接上|串起|探(?:测|查|索)?|复现|复验|拼|改|修|查|读|写|跑|看|加)/;

function isResidue(s: string): boolean {
  if (/^[0-9]/.test(s)) return true;
  if (/\\\\|[\/\\]{2,}/.test(s)) return true;
  if (s.includes('→')) return true;
  if (!/[\u4e00-\u9fa5]/.test(s)) return true;
  if (s.length < 8 && /[0-9\\/]/.test(s)) return true;
  return false;
}

const INTENT_START =
  /^(查|搜|修|改|写|读|看|跑|删|加|找|审|扫|取|填|换|恢复|验证|确认|检查|确保|排查|测试|实现|补充|删除|清理|重构|抽取|核对|比对|编译|定位|提取|盘点|摸清|精修|审查|收敛|迁移|接入|退役|落地|改造|修正|补齐|打通|收口|拼)/;

export function pickTitle(segs: string[]): string {
  let l1b = '';
  let l2 = '';
  for (let i = segs.length - 1; i >= 0; i--) {
    const raw = segs[i].replace(/\s+/g, '');
    if (raw.length < 6 || raw.length > 24) continue;
    if (isResidue(raw)) continue;
    const compact = titleCompact(raw);
    if (compact.length < 6) continue;
    if (INTENT_START.test(compact)) return compact;
    if (ACTION_VERBS.test(compact) && !l1b) l1b = compact;
    if (!l2) l2 = compact;
  }
  return l1b || l2;
}
