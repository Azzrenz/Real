
import { memo, useEffect, useRef, useState } from 'react';
import { Icon } from '../../../shared/ui/icons';
import { DetailWindow } from '../../../shared/ui/DetailWindow';
import { useChatStore, type ThinkRow } from '../store/chatStore';
import { useSessionStore } from '../../sessions/store/sessionStore';
import { foldPlainThinkingText } from '../../thinking/parseThinking';
import { useT } from '../../../shared/i18n';

/* ---------- thinking disclosure row ---------- */

function ThinkCodeBlock({ lang, code, active }: { lang: string; code: string; active?: boolean }) {
  const t = useT();
  const [open, setOpen] = useState(true);
  const codeRef = useRef<HTMLPreElement>(null);
  const autoScroll = useRef(true);
  const lines = code.split('\n').length;
  useEffect(() => {
    if (active && autoScroll.current && codeRef.current) {
      codeRef.current.scrollTop = codeRef.current.scrollHeight;
    }
  }, [code, active]);
  const onCodeScroll = () => {
    const el = codeRef.current;
    if (!el) return;
    autoScroll.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
  };
  return (
    <div className={`think-code-wrap${open ? ' open' : ''}`}>
      <button
        className="think-code-head"
        onClick={() => setOpen(!open)}
        aria-expanded={open}
        title={open ? t('收起代码块') : t('展开代码块')}
      >
        <span className="think-code-lang">{lang || 'code'}</span>
        <span className="think-code-meta">{t('{n} 行', { n: lines })}</span>
        <Icon name="chevron" size={14} className={`flow-row-caret${open ? ' open' : ''}`} />
      </button>
      {open && (
        <pre className="think-code" ref={codeRef} onScroll={onCodeScroll}>
          <code>{code}</code>
        </pre>
      )}
    </div>
  );
}

function ThinkContent({ text, active }: { text: string; active?: boolean }) {
  const parts = text.split('```');
  const out: React.ReactNode[] = [];
  for (let i = 0; i < parts.length; i++) {
    if (i % 2 === 0) {
      // display fold: the model's "blank line per sentence" no longer renders
      // as a whole empty line (that was the loose leading the user reported)
      const plain = foldPlainThinkingText(parts[i]);
      if (plain) out.push(<pre key={`t${i}`} className="thinking-text">{plain}</pre>);
    } else {
      let code = parts[i];
      let lang = '';
      const nl = code.indexOf('\n');
      if (nl >= 0) {
        lang = code.slice(0, nl).trim();
        code = code.slice(nl + 1);
      }
      out.push(<ThinkCodeBlock key={`c${i}`} lang={lang} code={code} active={active} />);
    }
  }
  return <>{out}</>;
}

function isActionConcatNoise(note: string): boolean {
  const s = note.trim();
  if (!s.startsWith('正在')) return false;
  const parts = s.split('；');
  if (parts.length < 2) return false;
  return parts.every((p) => /^正在\S+$/.test(p.trim()));
}

const THINK_BODY_CHARS = 4000;
export const ThinkRowView = memo(function ThinkRowView({ row }: { row: ThinkRow; running?: boolean }) {
  const t = useT();
  const [open, setOpen] = useState(false);
  const active = row.status === 'thinking';
  const [shownLen, setShownLen] = useState(() => row.body?.length ?? 0);
  const shownRef = useRef(row.body?.length ?? 0);
  const targetRef = useRef(row.body?.length ?? 0);
  const lastAdvanceRef = useRef(Date.now());
  const clockRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const stopClock = () => {
    if (clockRef.current) {
      clearInterval(clockRef.current);
      clockRef.current = null;
    }
  };
  useEffect(() => {
    targetRef.current = row.body?.length ?? 0;
    if (!active) {
      stopClock();
      shownRef.current = targetRef.current;
      setShownLen(targetRef.current);
      return;
    }
    if (targetRef.current > shownRef.current && Date.now() - lastAdvanceRef.current > 2000) {
      shownRef.current = targetRef.current;
      lastAdvanceRef.current = Date.now();
      setShownLen(targetRef.current);
    }
    if (!clockRef.current) {
      clockRef.current = setInterval(() => {
        const gap = targetRef.current - shownRef.current;
        if (gap <= 0) return;
        const step = Math.max(1, Math.ceil(gap / 10));
        shownRef.current = Math.min(targetRef.current, shownRef.current + step);
        lastAdvanceRef.current = Date.now();
        setShownLen(shownRef.current);
      }, 50);
    }
  }, [row.body, active]);
  useEffect(() => stopClock, []);
  const bodyRef = useRef<HTMLDivElement>(null);
  const autoScroll = useRef(true);
  const hasBody = (row.body ?? '').length > 0;
  const prevActiveRef = useRef(active);
  useEffect(() => {
    const wasActive = prevActiveRef.current;
    prevActiveRef.current = active;
    if (active) {
      if (hasBody) setOpen(true);
    } else if (wasActive) {
      setOpen(false);
    }
  }, [active, hasBody]);
  useEffect(() => {
    if (open && active && autoScroll.current && bodyRef.current) {
      bodyRef.current.scrollTop = bodyRef.current.scrollHeight;
    }
  }, [row.body, active, open, shownLen]);
  const handleBodyScroll = () => {
    const el = bodyRef.current;
    if (!el) return;
    autoScroll.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
  };
  const showNote = !!row.note && !isActionConcatNoise(row.note);
  const bodyText = row.body ?? '';
  const truncated = !active && bodyText.length > THINK_BODY_CHARS;
  const truncHead = (() => {
    if (!truncated) return bodyText;
    const head = bodyText.slice(0, THINK_BODY_CHARS);
    const nl = head.lastIndexOf('\n');
    return nl >= THINK_BODY_CHARS * 0.6 ? bodyText.slice(0, nl) : head;
  })();
  const [showAll, setShowAll] = useState(false);
  const toggle = () => {
    const next = !open;
    setOpen(next);
    if (next && row.lazy && !row.loading && !row.body) {
      const sid = useSessionStore.getState().currentId;
      if (sid) void useChatStore.getState().loadThinking(sid, row.id);
    }
  };
  const lineCount = bodyText ? bodyText.split('\n').length : 0;
  if (active && !hasBody && !row.lazy) return null;
  return (
    <>
      {showNote && <div className="flow-narration">{row.note}</div>}
      <div className={`flow-row think-row${active ? ' running' : ''}${open ? ' open' : ''}`}>
        <button
          className="flow-row-main"
          onClick={toggle}
          aria-expanded={open}
          title={open ? t('收起') : t('展开')}
        >
          <Icon name="target" size={14} className="flow-row-icon" />
          <span className="flow-row-title">
            <span className="flow-title-text">
              {row.label && row.label !== t('深度思考')
                ? row.label
                : active
                  ? t('正在推理中')
                  : t('推理已完成')}
            </span>
          </span>
          <Icon name="chevron" size={14} className={`flow-row-caret${open ? ' open' : ''}`} />
        </button>
      {open && (
        <DetailWindow kind="think">
          <div className="think-window" ref={bodyRef} onScroll={handleBodyScroll}>
            {bodyText ? (
              active ? (
                <pre className="thinking-text">{foldPlainThinkingText(bodyText.slice(0, shownLen))}</pre>
              ) : truncated && !showAll ? (
                <>
                  <ThinkContent text={truncHead} />
                  <div className="think-trunc" aria-hidden="true">
                    <span className="think-trunc-line" />
                    <span className="think-trunc-label">
                      {t('内容较长，已显示前 {n} 字符', { n: truncHead.length.toLocaleString() })}
                    </span>
                    <span className="think-trunc-line" />
                  </div>
                  <button className="think-expand-btn" onClick={() => setShowAll(true)}>
                    {t('显示全部（{lines} 行 / {chars} 字符）', {
                      lines: lineCount.toLocaleString(),
                      chars: bodyText.length.toLocaleString(),
                    })}
                  </button>
                </>
              ) : (
                <ThinkContent text={bodyText} />
              )
            ) : row.lazy ? (
              <span className="flow-empty">
                {row.loading
                  ? t('正在加载思考全文…{extra}', {
                      extra: row.lazy.len ? t('（约 {n} 字）', { n: row.lazy.len }) : '',
                    })
                  : t('加载失败，点击标题重试')}
              </span>
            ) : active ? null : (
              <span className="flow-empty">{t('（该阶段未输出思考原文）')}</span>
            )}
          </div>
        </DetailWindow>
      )}
      </div>
    </>
  );
});
