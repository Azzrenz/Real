
import { t } from '../../../shared/i18n';

export const MAX_ATTACHMENTS = 10;

export const MAX_FILE_BYTES = 8 * 1024 * 1024;

const IMAGE_EXT = new Set(['png', 'jpg', 'jpeg', 'webp', 'gif', 'bmp', 'svg']);

export interface Attachment {
  id: string;
  name: string;
  size: number;
  kind: 'image' | 'file';
  dataUrl: string;
}

function extOf(name: string): string {
  const i = name.lastIndexOf('.');
  return i >= 0 ? name.slice(i + 1).toLowerCase() : '';
}

let seq = 0;
function nextId(): string {
  seq += 1;
  return `att-${Date.now()}-${seq}`;
}

export function fileToAttachment(file: File): Promise<{ ok: true; att: Attachment } | { ok: false; reason: string }> {
  if (file.size > MAX_FILE_BYTES) {
    return Promise.resolve({
      ok: false,
      reason: t('{name} 超过 {mb}MB 上限', {
        name: file.name,
        mb: Math.round(MAX_FILE_BYTES / 1024 / 1024),
      }),
    });
  }
  return new Promise((resolve) => {
    const reader = new FileReader();
    reader.onerror = () => resolve({ ok: false, reason: t('{name} 读取失败', { name: file.name }) });
    reader.onload = () => {
      const dataUrl = typeof reader.result === 'string' ? reader.result : '';
      if (!dataUrl) {
        resolve({ ok: false, reason: t('{name} 读取失败', { name: file.name }) });
        return;
      }
      resolve({
        ok: true,
        att: {
          id: nextId(),
          name: file.name,
          size: file.size,
          kind: IMAGE_EXT.has(extOf(file.name)) ? 'image' : 'file',
          dataUrl,
        },
      });
    };
    reader.readAsDataURL(file);
  });
}

export async function mergeAttachments(
  current: Attachment[],
  files: File[],
): Promise<{ list: Attachment[]; rejected: string[] }> {
  const rejected: string[] = [];
  const list = [...current];
  for (const f of files) {
    if (list.length >= MAX_ATTACHMENTS) {
      rejected.push(t('最多只能带 {n} 个附件', { n: MAX_ATTACHMENTS }));
      break;
    }
    const r = await fileToAttachment(f);
    if (!r.ok) {
      rejected.push(r.reason);
      continue;
    }
    if (list.some((a) => a.name === r.att.name && a.size === r.att.size)) continue;
    list.push(r.att);
  }
  return { list, rejected };
}

export function toPayload(list: Attachment[]): Array<{ name: string; data_url: string }> {
  return list.map((a) => ({ name: a.name, data_url: a.dataUrl }));
}

export function parseMessageAttachments(itemJson: string | null): Attachment[] {
  if (!itemJson) return [];
  try {
    const o = JSON.parse(itemJson) as { attachments?: unknown };
    if (!Array.isArray(o.attachments)) return [];
    const out: Attachment[] = [];
    o.attachments.forEach((raw, i) => {
      if (!raw || typeof raw !== 'object') return;
      const r = raw as Record<string, unknown>;
      const name = typeof r.name === 'string' ? r.name : '';
      const dataUrl = typeof r.data_url === 'string' ? r.data_url : '';
      if (!name) return;
      out.push({
        id: `hist-${i}-${name}`,
        name,
        size: 0,
        kind: r.kind === 'image' ? 'image' : 'file',
        dataUrl,
      });
    });
    return out;
  } catch {
    return [];
  }
}
