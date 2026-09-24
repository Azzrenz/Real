import { useEffect, useRef, useState } from 'react';
import { Icon } from '../../../shared/ui/icons';
import { useT } from '../../../shared/i18n';
import { fmtTime } from '../../sessions/lib/sessionList';
import './chat-history.css';

/** One entry per user question in the current task; the id is the scroll anchor. */
export interface ChatIndexItem {
  id: string;
  text: string;
  time: string;
}

export function ChatHistory({
  items,
  onJump,
}: {
  items: ChatIndexItem[];
  onJump: (id: string) => void;
}) {
  const t = useT();
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      if (wrapRef.current && !wrapRef.current.contains(e.target as Node)) setOpen(false);
    };
    const onEsc = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false);
    };
    document.addEventListener('mousedown', onDoc);
    document.addEventListener('keydown', onEsc);
    return () => {
      document.removeEventListener('mousedown', onDoc);
      document.removeEventListener('keydown', onEsc);
    };
  }, [open]);

  return (
    <div className="hist-wrap" ref={wrapRef}>
      <button
        className={`hdock-btn${open ? ' open' : ''}`}
        onClick={() => setOpen((v) => !v)}
        title={t('历史提问')}
        aria-label={t('历史提问')}
        aria-expanded={open}
      >
        <Icon name="message" size={17} />
        <span>{t('历史提问')}</span>
      </button>
      {open && (
        <div className="hist-pop" role="dialog" aria-label={t('历史提问')}>
          <div className="hist-head">
            <span className="hist-head-title">{t('历史提问')}</span>
            <span className="hist-head-count">{items.length}</span>
          </div>
          <div className="hist-list">
            {items.length === 0 && <div className="hist-empty">{t('还没有提问')}</div>}
            {items.map((it, i) => (
              <button
                key={it.id}
                className="hist-item"
                onClick={() => {
                  onJump(it.id);
                  setOpen(false);
                }}
              >
                <span className="hist-seq">{i + 1}</span>
                <span className="hist-item-title">{it.text}</span>
                <span className="hist-item-time">{fmtTime(it.time)}</span>
                <span className="hist-tip" role="tooltip">
                  {it.text}
                </span>
              </button>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
