// Shared list shaping for every session picker (sidebar and header dock).

import type { Session } from '../../../services/contracts';
import { t } from '../../../shared/i18n';

export function fmtTime(ts: string): string {
  const d = new Date(ts);
  if (Number.isNaN(d.getTime())) return '';
  const diff = Math.floor((Date.now() - d.getTime()) / 1000);
  if (diff < 60) return t('刚刚');
  if (diff < 3600) return t('{n} 分钟前', { n: Math.floor(diff / 60) });
  if (diff < 86400) return t('{n} 小时前', { n: Math.floor(diff / 3600) });
  if (diff < 86400 * 7) return t('{n} 天前', { n: Math.floor(diff / 86400) });
  return d.toLocaleDateString('zh-CN', { month: 'numeric', day: 'numeric' });
}

export function groupLabel(ts: string): string | null {
  const d = new Date(ts);
  if (Number.isNaN(d.getTime())) return null;
  const now = new Date();
  const today = now.toDateString();
  const yesterday = new Date(now);
  yesterday.setDate(yesterday.getDate() - 1);
  if (d.toDateString() === today) return t('今天');
  if (d.toDateString() === yesterday.toDateString()) return t('昨天');
  return t('更早');
}

export const GROUP_ORDER = ['今天', '昨天', '更早'];

export function groupSessions(sessions: Session[]): Array<{ label: string; items: Session[] }> {
  const map: Record<string, Session[]> = {};
  for (const s of sessions) {
    const label = groupLabel(s.updated_at) ?? t('更早');
    (map[label] ??= []).push(s);
  }
  return GROUP_ORDER.map((g) => t(g))
    .filter((label) => map[label]?.length)
    .map((label) => ({ label, items: map[label] }));
}
