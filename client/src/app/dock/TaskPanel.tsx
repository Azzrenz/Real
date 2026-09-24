import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Icon } from '../../shared/ui/icons';
import { useT } from '../../shared/i18n';
import { toolActionZh } from '../../features/tools/lib/labels';
import { http } from '../../services/http';
import { useDockSync } from './changedFiles';

interface RecentFile {
  name: string;
  path: string;
  size: number;
  modified: number;
}

function extOf(p: string): string {
  const i = p.lastIndexOf('.');
  return i < 0 ? '' : p.slice(i + 1).toLowerCase();
}

export function TaskPanel({ onOpen }: { onOpen: (path: string) => void }) {
  const t = useT();
  const sync = useDockSync();
  const [, setTick] = useState(0);
  const [logs, setLogs] = useState<RecentFile[] | null>(null);

  useEffect(() => {
    const id = window.setInterval(() => setTick((n) => n + 1), 10_000);
    return () => window.clearInterval(id);
  }, []);

  useEffect(() => {
    let alive = true;
    invoke<string>('workspace_root')
      .then((root) =>
        http.get<RecentFile[]>(`/api/memory/files?workspace=${encodeURIComponent(root)}`),
      )
      .then((r) => {
        if (alive) setLogs(r);
      })
      .catch(() => {
        if (alive) setLogs([]);
      });
    return () => {
      alive = false;
    };
  }, []);

  const task = sync?.task ?? null;
  const files = sync?.files ?? [];
  const started = task?.startedAt ? new Date(task.startedAt).getTime() : 0;
  const mins = started ? Math.floor((Date.now() - started) / 60000) : 0;

  return (
    <div className="panel-pad">
      {task ? (
        <>
          <div className="panel-head">
            <Icon name="target" size={13} />
            <span className="panel-head-title">{t('当前任务')}</span>
          </div>
          <dl className="kv">
            <div className="kv-row">
              <dt>{t('任务')}</dt>
              <dd title={task.title}>{task.title || t('未命名')}</dd>
            </div>
            <div className="kv-row">
              <dt>{t('已进行')}</dt>
              <dd>
                {started
                  ? mins >= 60
                    ? t('{h} 时 {m} 分', { h: Math.floor(mins / 60), m: mins % 60 })
                    : t('{n} 分钟', { n: mins })
                  : '—'}
              </dd>
            </div>
          </dl>

          <div className="panel-sub">{t('改动过的文件')}</div>
          {files.length === 0 ? (
            <p className="dock-msg">{t('还没有改动')}</p>
          ) : (
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
                      <span className="art-sub">{t(toolActionZh(f.tool))}</span>
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          )}
        </>
      ) : (
        <p className="dock-msg">{t('当前没有打开的任务')}</p>
      )}

      <div className="panel-sub">{t('记忆与工作日志')}</div>
      {logs === null ? (
        <p className="dock-msg">{t('读取中…')}</p>
      ) : logs.length === 0 ? (
        <p className="dock-msg">{t('还没有日志')}</p>
      ) : (
        <ul className="art-list">
          {logs.map((f) => (
            <li key={f.path}>
              <button
                type="button"
                className="art-row"
                onClick={() => onOpen(f.path)}
                title={f.path}
              >
                <span className="file-dot" data-ext="md" aria-hidden />
                <span className="art-main">
                  <span className="art-name">
                    {f.name === 'MEMORY.md' ? t('长期记忆') : f.name.replace(/\.md$/, '')}
                  </span>
                  <span className="art-sub">{ago(f.modified, t)}</span>
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function ago(sec: number, t: ReturnType<typeof useT>): string {
  if (!sec) return '—';
  const d = Math.max(0, Date.now() / 1000 - sec);
  if (d < 60) return t('刚刚');
  if (d < 3600) return t('{n} 分钟前', { n: Math.floor(d / 60) });
  if (d < 86400) return t('{n} 小时前', { n: Math.floor(d / 3600) });
  return t('{n} 天前', { n: Math.floor(d / 86400) });
}
