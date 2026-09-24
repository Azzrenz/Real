import { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useT } from '../../shared/i18n';
import { useDockSync } from './changedFiles';

const KIND_ORDER = ['网页', '文档', '表格', '数据', '图片', 'PDF', '演示', '源码', '文本'] as const;

type Kind = (typeof KIND_ORDER)[number];

const EXT_KIND: Record<string, Kind> = {
  html: '网页',
  htm: '网页',
  md: '文档',
  doc: '文档',
  docx: '文档',
  odt: '文档',
  rtf: '文档',
  csv: '表格',
  tsv: '表格',
  xls: '表格',
  xlsx: '表格',
  ods: '表格',
  json: '数据',
  jsonl: '数据',
  yaml: '数据',
  yml: '数据',
  toml: '数据',
  xml: '数据',
  png: '图片',
  jpg: '图片',
  jpeg: '图片',
  gif: '图片',
  webp: '图片',
  svg: '图片',
  bmp: '图片',
  ico: '图片',
  pdf: 'PDF',
  ppt: '演示',
  pptx: '演示',
  odp: '演示',
  rs: '源码',
  ts: '源码',
  tsx: '源码',
  js: '源码',
  jsx: '源码',
  mjs: '源码',
  cjs: '源码',
  vue: '源码',
  svelte: '源码',
  py: '源码',
  go: '源码',
  java: '源码',
  c: '源码',
  h: '源码',
  cpp: '源码',
  hpp: '源码',
  cs: '源码',
  rb: '源码',
  php: '源码',
  swift: '源码',
  kt: '源码',
  sql: '源码',
  sh: '源码',
  bat: '源码',
  ps1: '源码',
  css: '源码',
  scss: '源码',
  less: '源码',
};

function extOf(p: string): string {
  const i = p.lastIndexOf('.');
  return i < 0 ? '' : p.slice(i + 1).toLowerCase();
}

function kindOf(path: string): Kind {
  return EXT_KIND[extOf(path)] ?? '文本';
}

function relTo(p: string, root: string): string {
  if (!root) return p;
  const a = p.replace(/[\\/]+/g, '/');
  const b = root.replace(/[\\/]+/g, '/').replace(/\/+$/, '');
  if (b && a.toLowerCase().startsWith(`${b.toLowerCase()}/`)) return a.slice(b.length + 1);
  return p;
}

export function ArtifactsPanel({ onOpen }: { onOpen: (path: string) => void }) {
  const t = useT();
  const sync = useDockSync();
  const [root, setRoot] = useState('');

  useEffect(() => {
    let alive = true;
    invoke<string>('workspace_root')
      .then((r) => {
        if (alive) setRoot(r);
      })
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, []);

  const groups = useMemo(() => {
    const m = new Map<Kind, { path: string; name: string }[]>();
    for (const f of sync?.files ?? []) {
      const k = kindOf(f.path);
      const list = m.get(k);
      if (list) list.push(f);
      else m.set(k, [f]);
    }
    return KIND_ORDER.filter((k) => m.has(k)).map((k) => [k, m.get(k)!] as const);
  }, [sync]);

  const total = sync?.files.length ?? 0;

  if (!sync) return <p className="dock-msg">{t('读取中…')}</p>;

  if (total === 0) return <p className="dock-msg">{t('本次任务还没有产出文件')}</p>;

  return (
    <div className="art-root">
      <div className="art-summary">
        <span className="art-total">
          {t('共 {n} 个文件 · {m} 类', { n: total, m: groups.length })}
        </span>
      </div>

      {groups.map(([kind, files]) => (
        <section className="ov-group" key={kind}>
          <div className="ov-head">
            <span className="ov-day">{t(kind)}</span>
            <span className="ov-count">{files.length}</span>
          </div>
          <ul className="art-list">
            {files.map((f) => (
              <li key={f.path}>
                <button
                  type="button"
                  className="art-row"
                  onClick={() => onOpen(f.path)}
                  title={f.path}
                >
                  <span className="file-dot" data-ext={extOf(f.path)} aria-hidden />
                  <span className="art-main">
                    <span className="art-name">{f.name}</span>
                    <span className="art-sub">{relTo(f.path, root)}</span>
                  </span>
                </button>
              </li>
            ))}
          </ul>
        </section>
      ))}
    </div>
  );
}
