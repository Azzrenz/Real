import { useEffect, useMemo, useState } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { skillsApi } from '../services/domains/skills';
import { renderMarkdown } from '../shared/lib/markdown';
import { Icon } from '../shared/ui/icons';
import { useSettings } from '../features/settings/store/settingsStore';
import './skill-detail.css';
import { useT } from '../shared/i18n';

/** Split SKILL.md into its frontmatter fields and the markdown body. */
function parseSkillMd(raw: string): { description: string; category: string; body: string } {
  const t = raw.replace(/^\uFEFF/, '').trimStart();
  if (!t.startsWith('---')) return { description: '', category: '', body: t };
  const end = t.indexOf('\n---', 3);
  if (end < 0) return { description: '', category: '', body: t };
  let description = '';
  let category = '';
  for (const line of t.slice(3, end).split('\n')) {
    if (line.startsWith('description:')) description = line.slice(12).trim();
    else if (line.startsWith('category:')) category = line.slice(9).trim();
  }
  return { description, category, body: t.slice(end + 4).replace(/^\s+/, '') };
}

/**
 * The skill detail window: a second webview of the same bundle, built by Rust, which
 * injects the skill name before the page loads. It renders the whole SKILL.md -- the
 * point of a separate window is that the document finally has room to be read.
 */
export function SkillDetailWindow({ name }: { name: string }) {
  // Its own webview: the language has to be subscribed here, not inherited.
  const t = useT();
  const win = () => getCurrentWindow();
  const theme = useSettings((s) => s.theme);
  const [doc, setDoc] = useState<{ description: string; category: string; body: string } | null>(
    null,
  );
  const [err, setErr] = useState<string | null>(null);

  // Own document, own theme attribute: the one App sets on the main window does not apply here.
  useEffect(() => {
    document.documentElement.setAttribute('data-theme', theme);
  }, [theme]);

  useEffect(() => {
    let alive = true;
    skillsApi
      .raw(name)
      .then((r) => {
        if (alive) setDoc(parseSkillMd(r.text));
      })
      .catch((e) => {
        if (alive) setErr((e as Error).message || t('技能内容加载失败'));
      });
    return () => {
      alive = false;
    };
  }, [name]);

  const html = useMemo(() => (doc?.body ? renderMarkdown(doc.body) : ''), [doc]);

  return (
    <div className="skill-win">
      <div className="titlebar">
        <div className="skill-win-id">
          {doc?.category ? <span className="skill-pop-cat">{doc.category}</span> : null}
          <span className="skill-win-name">{name}</span>
        </div>
        <div className="titlebar-drag" data-tauri-drag-region />
        <div className="titlebar-actions">
          <button
            type="button"
            className="titlebar-btn"
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => win().minimize().catch(() => undefined)}
            aria-label={t('最小化')}
          >
            <Icon name="minimize" size={15} strokeWidth={1.7} />
          </button>
          <button
            type="button"
            className="titlebar-btn"
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => win().toggleMaximize().catch(() => undefined)}
            aria-label={t('最大化')}
          >
            <Icon name="maximize" size={14} strokeWidth={1.7} />
          </button>
          <button
            type="button"
            className="titlebar-btn titlebar-btn--close"
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => win().close().catch(() => undefined)}
            aria-label={t('关闭')}
          >
            <Icon name="x" size={15} strokeWidth={1.7} />
          </button>
        </div>
      </div>

      <div className="skill-win-body">
        {err ? (
          <p className="skill-win-msg">{err}</p>
        ) : !doc ? (
          <p className="skill-win-msg">{t('加载中…')}</p>
        ) : (
          <>
            {doc.description ? <p className="skill-win-desc">{doc.description}</p> : null}
            <div className="prose skill-win-doc" dangerouslySetInnerHTML={{ __html: html }} />
          </>
        )}
      </div>
    </div>
  );
}
