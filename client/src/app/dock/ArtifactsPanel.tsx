import { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useT } from '../../shared/i18n';
import { useDockSync, type ChangedFile } from './changedFiles';

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

/** How many rows a kind shows before the rest fold away. This tab answers "what did this task
 *  produce", not "list everything ever written" -- an unbounded column turns into an archive. */
const SHOWN_PER_KIND = 6;
/** Past this age a file counts as settled: it folds into the "earlier" line and renders dimmed. */
const COLD_MS = 6 * 60 * 60 * 1000;

type TFn = ReturnType<typeof useT>;

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

/** Age in ms, or null when the entry carries no usable timestamp. */
function ageOf(at: string | undefined, now: number): number | null {
  if (!at) return null;
  const t = Date.parse(at);
  if (Number.isNaN(t)) return null;
  return Math.max(0, now - t);
}

function agoText(ms: number | null, t: TFn): string {
  if (ms === null) return '';
  const min = Math.floor(ms / 60000);
  if (min < 1) return t('刚刚');
  if (min < 60) return t('{n} 分钟前', { n: min });
  const h = Math.floor(min / 60);
  if (h < 24) return t('{n} 小时前', { n: h });
  return t('{n} 天前', { n: Math.floor(h / 24) });
}

function isCold(f: ChangedFile, now: number): boolean {
  const ms = ageOf(f.at, now);
  return ms !== null && ms > COLD_MS;
}

export function ArtifactsPanel({ onOpen }: { onOpen: (path: string) => void }) {
  const t = useT();
  const sync = useDockSync();
  const [root, setRoot] = useState('');
  const [now, setNow] = useState(() => Date.now());
  /** per kind: show the folded tail */
  const [openKind, setOpenKind] = useState<Record<string, boolean>>({});
  /** settled files: hidden behind one line until asked for */
  const [openCold, setOpenCold] = useState(false);
  /** opened at least once -- it has been seen, so it stops competing with what is new */
  const [seen, setSeen] = useState<Record<string, boolean>>({});

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

  // Ages have to advance on their own: a dock left open would otherwise keep saying "刚刚" forever,
  // and nothing would ever settle into the cold fold.
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 60_000);
    return () => clearInterval(id);
  }, []);

  const files = sync?.files ?? [];

  const groups = useMemo(() => {
    const m = new Map<Kind, ChangedFile[]>();
    for (const f of files) {
      const k = kindOf(f.path);
      const list = m.get(k);
      if (list) list.push(f);
      else m.set(k, [f]);
    }
    // `files` is already newest-first, so every group keeps that order.
    return KIND_ORDER.filter((k) => m.has(k)).map((k) => [k, m.get(k)!] as const);
  }, [files]);

  const total = files.length;
  const coldCount = useMemo(() => files.filter((f) => isCold(f, now)).length, [files, now]);
  const newest = useMemo(() => {
    for (const f of files) {
      const ms = ageOf(f.at, now);
      if (ms !== null) return ms;
    }
    return null;
  }, [files, now]);

  const open = (f: ChangedFile) => {
    setSeen((p) => (p[f.path] ? p : { ...p, [f.path]: true }));
    onOpen(f.path);
  };

  if (!sync) return <p className="dock-msg">{t('读取中…')}</p>;

  if (total === 0) return <p className="dock-msg">{t('本次任务还没有产出文件')}</p>;

  return (
    <div className="art-root">
      <div className="art-summary">
        <span className="art-total">
          {t('共 {n} 个文件 · {m} 类', { n: total, m: groups.length })}
        </span>
        {newest !== null && (
          <span className="art-kind">{t('最近产出 {ago}', { ago: agoText(newest, t) })}</span>
        )}
      </div>

      {groups.map(([kind, all]) => {
        const list = openCold ? all : all.filter((f) => !isCold(f, now));
        if (list.length === 0) return null;
        const expanded = !!openKind[kind];
        const head = expanded ? list : list.slice(0, SHOWN_PER_KIND);
        const rest = list.length - head.length;
        return (
          <section className="ov-group" key={kind}>
            <div className="ov-head">
              <span className="ov-day">{t(kind)}</span>
              <span className="ov-count">{list.length}</span>
            </div>
            <ul className="art-list">
              {head.map((f) => (
                <li key={f.path}>
                  <button
                    type="button"
                    className={
                      'art-row' +
                      (isCold(f, now) ? ' art-row--cold' : '') +
                      (seen[f.path] ? ' art-row--seen' : '')
                    }
                    onClick={() => open(f)}
                    title={f.path}
                  >
                    <span className="file-dot" data-ext={extOf(f.path)} aria-hidden />
                    <span className="art-main">
                      <span className="art-name">{f.name}</span>
                      <span className="art-sub">{relTo(f.path, root)}</span>
                    </span>
                    <span className="art-ago">{agoText(ageOf(f.at, now), t)}</span>
                  </button>
                </li>
              ))}
            </ul>
            {rest > 0 && (
              <button
                type="button"
                className="art-more"
                onClick={() => setOpenKind((p) => ({ ...p, [kind]: true }))}
              >
                {t('还有 {n} 个', { n: rest })}
              </button>
            )}
            {expanded && list.length > SHOWN_PER_KIND && (
              <button
                type="button"
                className="art-more"
                onClick={() => setOpenKind((p) => ({ ...p, [kind]: false }))}
              >
                {t('收起')}
              </button>
            )}
          </section>
        );
      })}

      {coldCount > 0 && (
        <button type="button" className="art-more" onClick={() => setOpenCold((v) => !v)}>
          {openCold ? t('收起更早的 {n} 个', { n: coldCount }) : t('更早的 {n} 个', { n: coldCount })}
        </button>
      )}
    </div>
  );
}
