import { memo, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { Icon } from '../../shared/ui/icons';
import { useChatStore } from '../chat/store/chatStore';
import { useSessionStore } from '../sessions/store/sessionStore';
import { isThinkNoteNoise, type ThinkRow } from './contracts';
import { summarizeThinking } from './summarize';
import { ThinkingBody } from './ThinkingBody';
import { useTypewriter } from '../../shared/hooks/useTypewriter';
import { countLines } from '../../shared/lib/format';
import './thinking.css';
import { useT } from '../../shared/i18n';

/* Raw source text, not a translated string: a module-level t() freezes the language at import.
   The comparison below stays against the raw value, which is also what the backend sends. */
const DEFAULT_LABEL = '深度思考';

export const DeepThinkRow = memo(function DeepThinkRow({ row }: { row: ThinkRow }) {
  // memo()'d: without a subscription a language switch would keep the old wording on screen.
  const t = useT();
  const active = row.status === 'thinking';
  const bodyText = row.body ?? '';
  const [open, setOpen] = useState(active);

  useEffect(() => {
    setOpen(active);
  }, [active]);

  // 收起的那一帧（推理结束自动收起，或用户点标题收起）同步知会滚动层（useChatScroll 监听）：
  // 思考窗口收掉的高度若不用补白顶住，浏览器会随即把 scrollTop 夹到新的最大值 —— 屏幕上已有的
  // 内容整体下坠一截，看起来就是"又跳回最底端"。用 useLayoutEffect 是为了赶在浏览器按新高度
  // 夹紧之前把补白设好（CustomEvent 的派发是同步的，监听器当场生效）。
  const prevOpenRef = useRef(open);
  useLayoutEffect(() => {
    const was = prevOpenRef.current;
    prevOpenRef.current = open;
    if (was && !open) window.dispatchEvent(new CustomEvent('chat:think-collapsed'));
  }, [open]);

  const shownLen = useTypewriter(bodyText.length, active);

  const toggle = () => {
    const next = !open;
    setOpen(next);
    if (next && row.lazy && !row.loading && !row.body) {
      const sid = useSessionStore.getState().currentId;
      if (sid) void useChatStore.getState().loadThinking(sid, row.id);
    }
  };

  const summary = useMemo(() => summarizeThinking(bodyText), [bodyText]);

  const stats = useMemo(
    () => ({ chars: bodyText.length, lines: active ? 0 : countLines(bodyText) }),
    [bodyText, active],
  );
  const meta =
    stats.chars === 0
      ? ''
      : active
        ? t('{n} 字', { n: stats.chars.toLocaleString() })
        : t('{lines} 行 · {chars} 字', { lines: stats.lines, chars: stats.chars.toLocaleString() });

  const title =
    row.label && row.label !== DEFAULT_LABEL ? row.label : active ? t('正在推理中') : t(DEFAULT_LABEL);

  const showNote = !!row.note && !isThinkNoteNoise(row.note);

  const ghost = !active && !showNote && !summary && !meta && !row.lazy && !row.loading;
  if (ghost) return null;

  const placeholder = row.loading
    ? t('正在加载思考全文…{extra}', {
        extra: row.lazy?.len ? t('（约 {n} 字）', { n: row.lazy.len }) : '',
      })
    : row.lazy
      ? t('加载失败，点击标题重试')
      : active
        ? t('正在推理中')
        : t('（该阶段未输出思考原文）');

  const [copied, setCopied] = useState(false);
  const copyAll = () => {
    void navigator.clipboard.writeText(bodyText).then(() => {
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1200);
    });
  };

  return (
    <>
      {showNote && <div className="dt-note">{row.note}</div>}
      <section
        className={`dt-row${active ? ' is-live' : ''}${open ? ' is-open' : ''}`}
        aria-label={t(DEFAULT_LABEL)}
      >
        <div className="dt-head">
          <button
            className="dt-head-main"
            onClick={toggle}
            aria-expanded={open}
            title={open ? t('收起思考') : t('展开思考')}
          >
            <Icon name="target" size={14} className="dt-icon" />
            <span className="dt-title">{title}</span>
            {summary && <span className="dt-summary">{summary}</span>}
            {meta && <span className="dt-meta">{meta}</span>}
            <Icon name="chevron" size={14} className={`dt-caret${open ? ' open' : ''}`} />
          </button>
          {open && bodyText && (
            <button
              className={`dt-copy${copied ? ' copied' : ''}`}
              onClick={copyAll}
              title={copied ? t('已复制思考全文') : t('复制思考全文')}
              aria-label={t('复制思考全文')}
            >
              <Icon name={copied ? 'check' : 'copy'} size={13} />
            </button>
          )}
        </div>
        {open && (
          <div className="dt-window">
            {bodyText ? (
              <ThinkingBody text={bodyText} streaming={active} shownLen={shownLen} />
            ) : (
              <span className="dt-empty">{placeholder}</span>
            )}
          </div>
        )}
      </section>
    </>
  );
});

export default DeepThinkRow;
