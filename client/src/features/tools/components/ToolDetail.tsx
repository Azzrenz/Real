
import { useMemo } from 'react';
import { Icon } from '../../../shared/ui/icons';
import type { ToolCard } from '../types';
import { parseArgs } from '../lib/labels';
import { salvageTruncatedJson, unwrapRawJson } from '../lib/jsonSalvage';
import { parseWebSearchSummary } from '../lib/webSearch';
import { openExternal } from '../../../shared/lib/openExternal';
import { useT } from '../../../shared/i18n';

function DiffBody({ text }: { text: string }) {
  const t = useT();
  return (
    <div className="diff-body">
      {text.split('\n').map((line, i) => {
        const tr = line.trimStart();
        // The salvage layer writes this count line in Chinese; it stays Chinese there so that parser
        // need not care about the language, and the wording is produced again here for display.
        const changed = tr.match(/^修改\s+(\d+)\s+处$/);
        if (changed) {
          return (
            <span key={i} className="diff-line diff-ctx">
              {t('修改 {n} 处', { n: changed[1] })}
            </span>
          );
        }
        if (tr.startsWith('- ')) return <span key={i} className="diff-line diff-del">{line}</span>;
        if (tr.startsWith('+ ')) return <span key={i} className="diff-line diff-add">{line}</span>;
        return <span key={i} className="diff-line diff-ctx">{line}</span>;
      })}
    </div>
  );
}

function SearchBody({ text }: { text: string }) {
  return (
    <div className="search-body">
      {text.split('\n').map((line, i) => {
        if (/^\d+\s*处匹配/.test(line)) return null;
        const head = line.match(/^([\w./\\-]+):(\d+)$/);
        if (head) {
          return (
            <div key={i} className="search-hit-head">
              <span className="search-hit-file">{head[1]}</span>
              <span className="search-hit-line">:{head[2]}</span>
            </div>
          );
        }
        if (line.startsWith('  ')) return <div key={i} className="search-hit-body">{line.slice(2)}</div>;
        if (line.trim().length === 0) return <div key={i} className="search-hit-gap" />;
        return <div key={i} className="search-hit-summary">{line}</div>;
      })}
    </div>
  );
}

function isDiffLike(text: string): boolean {
  const lines = text.split('\n');
  if (lines.length < 3) return false;
  let mark = 0;
  for (const l of lines) {
    const t = l.trimStart();
    if (t.startsWith('- ') || t.startsWith('+ ') || t.startsWith('@@ ')
      || t.startsWith('diff ') || t.startsWith('index ')) mark++;
  }
  return mark / lines.length > 0.25;
}

function ReadBody({ text }: { text: string }) {
  const t = useT();
  const diff = isDiffLike(text);
  return (
    <div className="read-window">
      {text.split('\n').map((line, i) => {
        // A segment head is "path · N lines" with an optional " · read S–E" suffix. Recognised in
        // either language, then rebuilt from its numbers so the wording follows the language.
        const seg = line
          .trim()
          .match(
            /^(.+?)\s+·\s+(\d+)\s*(?:行|lines?)(?:\s+·\s+本次读\s+(\d+)[–-](\d+)\s*(?:行|lines?))?$/,
          );
        if (!diff && seg && line.length < 120) {
          return (
            <span key={i} className="read-seg-head">
              {seg[1]} · {t('{n} 行', { n: seg[2] })}
              {seg[3] ? t(' · 本次读 {s}–{e} 行', { s: seg[3], e: seg[4] }) : ''}
            </span>
          );
        }
        if (diff) {
          const t = line.trimStart();
          if (t.startsWith('- ')) return <span key={i} className="diff-line diff-del">{line}</span>;
          if (t.startsWith('+ ')) return <span key={i} className="diff-line diff-add">{line}</span>;
        }
        return <span key={i} className="diff-line read-ctx">{line}</span>;
      })}
    </div>
  );
}

function ListPathRow({ path, type }: { path: string; type?: string }) {
  const m = path.match(/^(.*)\s+·\s+(\d+B)$/);
  const cleanPath = m ? m[1] : path;
  const size = m ? m[2] : '';
  const isDir = type ? type === 'dir' : !/\.[^\\/]+$/.test(cleanPath);
  const segs = cleanPath.split(/[\\/]/).filter(Boolean);
  const name = segs[segs.length - 1] || cleanPath;
  const parent = segs.slice(0, -1).join('\\');
  return (
    <div className="list-row" title={cleanPath}>
      <span className="list-icon" aria-hidden>{isDir ? '📁' : '📄'}</span>
      <span className="list-name">{name}</span>
      {size && <span className="list-size">{size}</span>}
      {parent && <span className="list-parent">{parent}</span>}
    </div>
  );
}

function ListBody({ text }: { text: string }) {
  const t = useT();
  if (text.trimStart().startsWith('{')) {
    try {
      const o = JSON.parse(text) as {
        data?: { entries?: Array<{ path?: unknown; type?: unknown }> };
      };
      const entries = o?.data?.entries;
      if (Array.isArray(entries) && entries.length > 0) {
        return (
          <div className="list-body">
            {entries.map((e, i) => (
              <ListPathRow
                key={i}
                path={typeof e.path === 'string' ? e.path : '?'}
                type={typeof e.type === 'string' ? e.type : undefined}
              />
            ))}
          </div>
        );
      }
    } catch { /* fall through: show as-is */ }
  }
  return (
    <div className="list-body">
      {text.split('\n').map((line, i) => {
        const tr = line.trimStart();
        if (i === 0) {
          // Same idea as the read segment head: parse the summary, then re-render it.
          const two = tr.match(/^(\d+)\s*(?:目录|dirs?)\s*·\s*(\d+)\s*(?:文件|files?)$/);
          if (two) {
            return (
              <div key={i} className="list-stat">
                {t('{dirs} 目录 · {files} 文件', { dirs: two[1], files: two[2] })}
              </div>
            );
          }
          const one = tr.match(/^(\d+)\s*(?:个文件|files?)$/);
          if (one) {
            return <div key={i} className="list-stat">{t('{n} 个文件', { n: one[1] })}</div>;
          }
        }
        if (tr.startsWith('- ')) {
          return <ListPathRow key={i} path={tr.slice(2)} />;
        }
        if (!line.trim()) return null;
        return <div key={i} className="list-note">{line}</div>;
      })}
    </div>
  );
}

function WriteBody({ text }: { text: string }) {
  const t = useT();
  const lines = text.split('\n').map((l) => l.trim()).filter(Boolean);
  let path = '';
  let bytes = '';
  let backup = '';
  for (const l of lines) {
    const mBytes = l.match(/^写入\s*(\d+)\s*字节$/);
    if (mBytes) { bytes = mBytes[1]; continue; }
    const mBak = l.match(/^备份[:：]\s*(.*)$/);
    if (mBak) { backup = mBak[1]; continue; }
    if (!path) path = l;
  }
  return (
    <div className="write-body">
      {path && <div className="tool-kv"><span className="tool-k">path</span><span className="tool-v">{path}</span></div>}
      {bytes && <div className="tool-kv"><span className="tool-k">{t('写入')}</span><span className="tool-v">{t('{n} 字节', { n: bytes })}</span></div>}
      {backup && <div className="tool-kv"><span className="tool-k">{t('备份')}</span><span className="tool-v">{backup}</span></div>}
    </div>
  );
}

function RunBody({ text, hideCmd = false }: { text: string; hideCmd?: boolean }) {
  const lines = text.split('\n');
  let statusIdx = -1;
  for (let i = 0; i < Math.min(5, lines.length); i++) {
    const t = lines[i].trim();
    if (/^exit\s+-?\d+/.test(t) || (/passed|failed/i.test(t) && t.length < 120)) {
      statusIdx = i;
      break;
    }
  }
  const statusLine = statusIdx >= 0 ? lines[statusIdx].trim() : '';
  const exitNum = statusLine.match(/exit\s+(-?\d+)/)?.[1];
  const ok = exitNum === '0' || (!exitNum && /passed/i.test(statusLine));
  const before = statusIdx > 0 ? lines.slice(0, statusIdx) : [];
  const after = statusIdx >= 0 ? lines.slice(statusIdx + 1) : lines;
  return (
    <div className="run-body">
      {!hideCmd && before.length > 0 && <pre className="run-cmd">{before.join('\n')}</pre>}
      {statusLine && (
        <div className={`run-status${ok ? ' ok' : ' bad'}`}>
          <span className="run-status-dot" aria-hidden />
          {statusLine}
        </div>
      )}
      <pre className="run-out">
        {after.map((l, i) => {
          const t = l.trim();
          if (t === '[stderr]') {
            return <span key={i} className="run-stderr-head">{l}</span>;
          }
          if (t.startsWith('[错误] ')) {
            return <span key={i} className="run-err-line">{l.slice(5)}</span>;
          }
          if (t.startsWith('[建议] ')) {
            return <span key={i} className="run-sug-line">{l.slice(5)}</span>;
          }
          return <span key={i} className="run-line">{l || ' '}</span>;
        })}
      </pre>
    </div>
  );
}

function WebSearchBody({ text }: { text: string }) {
  const t = useT();
  const parsed = useMemo(() => parseWebSearchSummary(text), [text]);
  if (!parsed || parsed.results.length === 0) {
    return <pre className="tool-out">{text}</pre>;
  }
  return (
    <div className="ws-body">
      <div className="ws-query">
        <span className="ws-q-label">{t('检索')}</span>
        <span className="ws-q-text" title={parsed.query}>{parsed.query}</span>
        <span className="ws-count">{t('{n} 条', { n: parsed.results.length })}</span>
      </div>
      {parsed.results.map((r, i) => (
        <div key={i} className="ws-item">
          <div className="ws-item-head">
            <span className="ws-idx" aria-hidden>{i + 1}</span>
            {r.title && <span className="ws-title" title={r.title}>{r.title}</span>}
            {r.time && <span className="ws-time">{r.time}</span>}
          </div>
          {r.snippet && <div className="ws-snippet">{r.snippet}</div>}
          {r.url && (
            <div className="ws-url-row">
              <span className="ws-url" title={r.url}>{r.url}</span>
              <span
                className="tool-open-web"
                role="button"
                tabIndex={0}
                title={t('在浏览器打开：{url}', { url: r.url })}
                aria-label={t('在浏览器打开该网页')}
                onClick={() => void openExternal(r.url)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' || e.key === ' ') void openExternal(r.url);
                }}
              >
                <Icon name="external" size={12} />
              </span>
            </div>
          )}
        </div>
      ))}
    </div>
  );
}

// 工具的入参键名以 registry 实际下发的为准（read 发 paths、modify 发 file/change_spec、
const TOOL_ARG_KEYS: Record<string, string[]> = {
  read: ['paths', 'path', 'mode', 'start_line', 'end_line', 'offset', 'limit'],
  write: ['path', 'content'],
  edit: ['file', 'path', 'find', 'replace'],
  modify: ['file', 'path', 'change_spec', 'find', 'replace'],
  search: ['pattern', 'path', 'glob'],
  find_files: ['pattern', 'path'],
  list: ['path', 'depth'],
  audit: ['path'],
  web_fetch: ['url'],
  web_search: ['query'],
  db_query: ['sql'],
  verify: ['command'],
};

// reason 是给标题行用的意图，不放进 IN 参数
const NON_ARG_KEYS = new Set(['reason']);

function ArgsSection({ name, args }: { name: string; args: Record<string, unknown> | null }) {
  const src: Record<string, unknown> = args ?? {};
  const usable = (k: string) =>
    src[k] != null && String(src[k]).trim() !== '' && !NON_ARG_KEYS.has(k);
  // 固定键命不中就回落到对象自有键 —— 否则 read(paths)/modify(file) 这类
  // 与表里键名不一致的调用，过滤完 rows 为空、整块 IN 直接不渲染，就是"参数全丢"。
  let keys = (TOOL_ARG_KEYS[name] ?? []).filter(usable);
  if (keys.length === 0) keys = Object.keys(src).filter(usable).slice(0, 8);
  const rows = keys.map((k) => ({ k, v: String(src[k]) }));
  if (rows.length === 0) {
    return (
      <div className="tool-section">
        <div className="tool-section-head">IN</div>
        <div className="tool-kv"><span className="tool-k">{'（本次调用未带参数）'}</span></div>
      </div>
    );
  }
  return (
    <div className="tool-section">
      <div className="tool-section-head">IN</div>
      {rows.map((r) => (
        <div key={r.k} className="tool-kv">
          <span className="tool-k">{r.k}</span>
          <span className="tool-v">{r.v}</span>
        </div>
      ))}
    </div>
  );
}

export function ToolDetail({ tool }: { tool: ToolCard }) {
  const t = useT();
  const args = parseArgs(tool.args);
  const summary = useMemo(() => unwrapRawJson(tool.name, tool.summary ?? ''), [tool.name, tool.summary]);
  const isEdit = tool.name === 'edit' || tool.name === 'modify';
  const isRead = tool.name === 'read';
  const isWrite = tool.name === 'write';
  const isRun = tool.name === 'run';
  const isSearch = tool.name === 'search';
  const isWebSearch = tool.name === 'web_search';
  const isWebFetch = tool.name === 'web_fetch';
  const fetchUrl = isWebFetch && typeof args?.url === 'string' ? args.url : '';
  const isList = tool.name === 'list' || tool.name === 'audit' || tool.name === 'find_files';
  const isCmd = tool.name === 'run' || tool.name === 'verify';
  const running = tool.status === 'running';
  const pending = running && !summary;
  const failed = tool.status === 'error';
  const failText = useMemo(() => {
    if (!failed) return summary;
    const s = summary.trimStart();
    if (!s.startsWith('{')) return summary;
    try {
      return JSON.stringify(JSON.parse(s), null, 2);
    } catch {
      const v = salvageTruncatedJson(s);
      return v ? JSON.stringify(v, null, 2) : summary;
    }
  }, [failed, summary]);
  return (
    <div className="tool-detail">
      {failed ? (
        <div className="tool-section">
          <div className="tool-error-box" role="alert">
            <div className="tool-error-head">{t('执行失败')}</div>
            <pre className="tool-error-text">{failText}</pre>
          </div>
        </div>
      ) : (
      <>
      {/* 入参常驻：跑完不再撤掉。原来只在 `pending`（running 且还没输出）时渲染，
          跑完的调用 IN 整块消失 —— 用户看到的就是"凡带参数的参数全丢"。 */}
      {!isRun && <ArgsSection name={tool.name} args={args} />}

      {/* One card for the whole call: IN on top, the echo below, split by a hairline.
          The echo used to carry its own box, which left the arguments floating outside
          it -- two halves of the same call reading as two unrelated blocks. */}
      {isCmd && (
        <div className="tool-section tool-cmd">
          {isRun && (
            <>
              <div className="tool-section-head">IN</div>
              {typeof args?.command === 'string' && args.command && (
                <div className="tool-kv"><span className="tool-k">command</span><span className="tool-v">{args.command}</span></div>
              )}
              {typeof args?.description === 'string' && args.description && (
                <div className="tool-kv"><span className="tool-k">description</span><span className="tool-v">{args.description}</span></div>
              )}
              {typeof args?.workdir === 'string' && args.workdir && (
                <div className="tool-kv"><span className="tool-k">workdir</span><span className="tool-v">{args.workdir}</span></div>
              )}
            </>
          )}
          {summary && <RunBody text={summary} hideCmd={isRun} />}
        </div>
      )}

      {isEdit && summary && (
        <div className="tool-section">
          <DiffBody text={summary} />
        </div>
      )}

      {isSearch && summary && (
        <div className="tool-section">
          <div className="tool-section-head">{t('命中')}</div>
          <SearchBody text={summary} />
        </div>
      )}

      {isWebSearch && summary && (
        <div className="tool-section">
          <div className="tool-section-head">{t('检索结果')}</div>
          <WebSearchBody text={summary} />
        </div>
      )}

      {isWebFetch && fetchUrl && (
        <div className="tool-section">
          <div className="tool-section-head">{t('页面')}</div>
          <div className="ws-url-row">
            <span className="ws-url" title={fetchUrl}>{fetchUrl}</span>
            <span
              className="tool-open-web"
              role="button"
              tabIndex={0}
              title={t('在浏览器打开：{url}', { url: fetchUrl })}
              aria-label={t('在浏览器打开该网页')}
              onClick={() => void openExternal(fetchUrl)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' || e.key === ' ') void openExternal(fetchUrl);
              }}
            >
              <Icon name="external" size={12} />
            </span>
          </div>
        </div>
      )}

      {isRead && summary && (
        <div className="tool-section">
          <ReadBody text={summary} />
        </div>
      )}

      {isList && summary && (
        <div className="tool-section">
          <ListBody text={summary} />
        </div>
      )}

      {isWrite && summary && (
        <div className="tool-section">
          <WriteBody text={summary} />
        </div>
      )}

      {!isRead && !isEdit && !isSearch && !isWebSearch && !isList && !isCmd && !isWrite && summary && (
        <div className="tool-section">
          <pre className="tool-out">{summary}</pre>
        </div>
      )}

      {typeof tool.exitCode === 'number' && tool.exitCode > 0 && (
        <div className="tool-section">
          <div className="tool-section-head">OUT</div>
          <span className="tool-exit-code">{t('退出码 {n}', { n: tool.exitCode })}</span>
        </div>
      )}

      {pending && (
        <div className="tool-section">
          <div className="tool-pending">
            <span className="tool-pending-dot" aria-hidden />
            {t('执行中 · 输出在跑完后回填')}
          </div>
        </div>
      )}
      </>
      )}
    </div>
  );
}
