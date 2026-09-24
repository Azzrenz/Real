import { lazy, Suspense, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { handleProseClick, renderMarkdown } from '../shared/lib/markdown';
import { Icon } from '../shared/ui/icons';
import { t } from '../shared/i18n';
import { useSettings } from '../features/settings/store/settingsStore';
import './file-preview.css';

const MonacoEditor = lazy(() => import('@monaco-editor/react'));

const MONACO_LANG_BY_EXT: Record<string, string> = {
  py: 'python', ts: 'typescript', tsx: 'typescript', js: 'javascript', jsx: 'javascript',
  mjs: 'javascript', cjs: 'javascript', sh: 'shell', bash: 'shell', zsh: 'shell',
  json: 'json', css: 'css', scss: 'scss', less: 'less',
  html: 'html', htm: 'html', xml: 'xml', svg: 'xml',
  rs: 'rust', toml: 'ini', yml: 'yaml', yaml: 'yaml', sql: 'sql',
  go: 'go', java: 'java', c: 'c', cpp: 'cpp',
  md: 'markdown', markdown: 'markdown',
};

type Kind = 'markdown' | 'html' | 'pdf' | 'image' | 'code' | 'text';

const MIME_BY_EXT: Record<string, string> = {
  html: 'text/html', htm: 'text/html',
  pdf: 'application/pdf',
  png: 'image/png', jpg: 'image/jpeg', jpeg: 'image/jpeg',
  gif: 'image/gif', webp: 'image/webp', svg: 'image/svg+xml', bmp: 'image/bmp',
};

function extOf(path: string): string {
  return path.split('.').pop()?.toLowerCase() ?? '';
}

function kindOf(path: string): Kind {
  const ext = path.split('.').pop()?.toLowerCase() ?? '';
  if (ext === 'md' || ext === 'markdown') return 'markdown';
  if (ext === 'html' || ext === 'htm') return 'html';
  if (ext === 'pdf') return 'pdf';
  if (['png', 'jpg', 'jpeg', 'gif', 'webp', 'svg', 'bmp'].includes(ext)) return 'image';
  if (MONACO_LANG_BY_EXT[ext]) return 'code';
  return 'text';
}

function MonacoView({ path, source }: { path: string; source: string }) {
  const theme = useSettings((s) => s.theme);
  const [draft, setDraft] = useState(source);
  const [saved, setSaved] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const dirty = draft !== source;
  const ext = path.split('.').pop()?.toLowerCase() ?? '';

  useEffect(() => {
    setDraft(source);
    setSaved(false);
    setErr(null);
  }, [source, path]);

  const save = () => {
    void invoke('write_preview_text', { path, content: draft })
      .then(() => {
        setSaved(true);
        window.setTimeout(() => setSaved(false), 2000);
      })
      .catch((e) => setErr(String(e)));
  };

  const saveRef = useRef(save);
  useEffect(() => {
    saveRef.current = save;
  });

  return (
    <div className="fp-editor">
      <div className="fp-editor-bar">
        {err ? (
          <span className="fp-editor-err">{err}</span>
        ) : (
          <span className="fp-editor-state">
            {dirty ? t('未保存') : saved ? t('已保存') : ''}
          </span>
        )}
        <button
          type="button"
          className={`fp-editor-save${saved ? ' on' : ''}`}
          onClick={save}
          disabled={!dirty}
          title={t('保存')}
          aria-label={t('保存')}
        >
          <Icon name="check" size={13} />
        </button>
      </div>
      <Suspense fallback={<p className="fp-msg">{t('加载编辑器…')}</p>}>
        <MonacoEditor
          height="100%"
          language={MONACO_LANG_BY_EXT[ext] ?? 'plaintext'}
          value={draft}
          onChange={(v) => setDraft(v ?? '')}
          theme={theme === 'dark' ? 'vs-dark' : 'vs'}
          options={{
            fontSize: 13,
            fontFamily: "'JetBrains Mono','Cascadia Mono','Consolas',monospace",
            lineNumbers: 'on',
            minimap: { enabled: false },
            scrollBeyondLastLine: false,
            wordWrap: 'off',
            tabSize: 2,
            renderLineHighlight: 'all',
            smoothScrolling: true,
            padding: { top: 10, bottom: 10 },
          }}

          onMount={(ed, monaco) => {
            ed.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, () => saveRef.current());
          }}
        />
      </Suspense>
    </div>
  );
}

export function FilePreview({
  path,
  onOpenPath,
}: {
  path: string;

  onOpenPath?: (path: string) => void;
}) {
  const kind = kindOf(path);
  const [text, setText] = useState<string | null>(null);
  const [url, setUrl] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [zoom, setZoom] = useState(false);
  const blobRef = useRef<string | null>(null);

  useEffect(() => {
    let alive = true;
    setText(null);
    setUrl(null);
    setErr(null);
    if (blobRef.current) {
      URL.revokeObjectURL(blobRef.current);
      blobRef.current = null;
    }

    if (kind === 'html' || kind === 'pdf' || kind === 'image') {
      invoke<number[]>('read_preview_bytes', { path })
        .then((bytes) => {
          if (!alive) return;
          const mime = MIME_BY_EXT[extOf(path)] ?? 'application/octet-stream';
          const blob = new Blob([new Uint8Array(bytes)], { type: mime });
          blobRef.current = URL.createObjectURL(blob);
          setUrl(blobRef.current);
        })
        .catch((e) => alive && setErr(String(e)));
    } else {
      invoke<string | null>('read_preview_text', { path })
        .then((r) => {
          if (!alive) return;

          if (r === null) {
            setErr(t('该文件不是文本，暂不支持预览'));
            return;
          }
          setText(r);
        })
        .catch((e) => alive && setErr(String(e)));
    }
    return () => {
      alive = false;
    };
  }, [path, kind]);

  useEffect(
    () => () => {
      if (blobRef.current) URL.revokeObjectURL(blobRef.current);
    },
    [],
  );

  const name = path.split(/[\\/]/).pop() ?? path;

  return (
    <div className="fp-root">

      <div className="fp-body">
        {err ? (
          <p className="fp-msg">{err}</p>
        ) : kind === 'html' || kind === 'pdf' || kind === 'image' ? (
          url ? (
            kind === 'image' ? (

              <>
                <button type="button" className="fp-img-btn" onClick={() => setZoom(true)}>
                  <img className="fp-img" src={url} alt={name} />
                </button>
                {zoom && (
                  <div
                    className="fp-zoom"
                    role="dialog"
                    aria-modal="true"
                    aria-label={name}
                    onClick={() => setZoom(false)}
                  >
                    <img className="fp-zoom-img" src={url} alt={name} />
                  </div>
                )}
              </>
            ) : (
              <iframe
                className="fp-frame"
                src={url}
                title={name}
                sandbox={kind === 'html' ? '' : undefined}
              />
            )
          ) : (
            <p className="fp-msg">{t('加载中…')}</p>
          )
        ) : text === null ? (
          <p className="fp-msg">{t('加载中…')}</p>
        ) : kind === 'markdown' ? (

          <div
            className="prose fp-md"
            onClick={(e) => handleProseClick(e, { onOpenPath })}
            dangerouslySetInnerHTML={{ __html: renderMarkdown(text) }}
          />
        ) : (

          <MonacoView path={path} source={text} />
        )}
      </div>
    </div>
  );
}
