import { useEffect, useMemo, useState } from 'react';
import type { Session } from '../../../services/contracts';
import { Icon } from '../../../shared/ui/icons';
import { fmtTime, groupSessions } from '../lib/sessionList';
import { useT } from '../../../shared/i18n';

interface Props {
  sessions: Session[];
  currentId: string | null;
  onSelect: (id: string) => void;
  onNew: () => void;
  onDelete: (id: string) => void;
  onRename: (id: string, title: string) => void;
}

interface MenuState {
  id: string;
  x: number;
  y: number;
}

export function SessionSidebar({
  sessions,
  currentId,
  onSelect,
  onNew,
  onDelete,
  onRename,
}: Props) {
  const t = useT();
  const groups = useMemo(() => groupSessions(sessions), [sessions]);

  const [editingId, setEditingId] = useState<string | null>(null);
  const [draft, setDraft] = useState('');
  const [menu, setMenu] = useState<MenuState | null>(null);

  const commitRename = (id: string, fallback: string) => {
    setEditingId(null);
    if (draft.trim() && draft.trim() !== fallback) onRename(id, draft.trim());
  };

  useEffect(() => {
    if (!menu) return;
    const close = () => setMenu(null);
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && close();
    window.addEventListener('click', close);
    window.addEventListener('scroll', close, true);
    window.addEventListener('resize', close);
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('click', close);
      window.removeEventListener('scroll', close, true);
      window.removeEventListener('resize', close);
      window.removeEventListener('keydown', onKey);
    };
  }, [menu]);

  const openMenu = (e: React.MouseEvent, id: string) => {
    e.preventDefault();
    e.stopPropagation();
    setMenu({ id, x: e.clientX, y: e.clientY });
  };

  const startRename = (id: string, title: string) => {
    setMenu(null);
    setEditingId(id);
    setDraft(title || '');
  };

  const doDelete = (id: string) => {
    setMenu(null);
    onDelete(id);
  };

  return (
    <aside className="sidebar">
      <div className="sidebar-new">
        <button className="new-task-btn" onClick={onNew} title={t('新建任务')} aria-label={t('新建任务')}>
          <Icon name="compose" size={18} />
          <span>{t('新建任务')}</span>
        </button>
      </div>

      <div className="sidebar-list">
        {groups.map((g) => (
          <div key={g.label} className="session-group">
            <div className="session-group-label">{g.label}</div>
            {g.items.map((s) => {
              const active = s.id === currentId;
              return (
                <div
                  key={s.id}
                  className={`session-item${active ? ' active' : ''}`}
                  onClick={() => onSelect(s.id)}
                  onContextMenu={(e) => openMenu(e, s.id)}
                  role="button"
                  tabIndex={0}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter' || e.key === ' ') onSelect(s.id);
                  }}
                >
                  <div className="session-row">
                    {editingId === s.id ? (
                      <input
                        className="rename-input"
                        value={draft}
                        autoFocus
                        onFocus={(e) => e.currentTarget.select()}
                        onChange={(e) => setDraft(e.target.value)}
                        onKeyDown={(e) => {
                          if (e.nativeEvent.isComposing || e.keyCode === 229) return;
                          if (e.key === 'Enter') commitRename(s.id, s.title);
                          if (e.key === 'Escape') setEditingId(null);
                        }}
                        onBlur={() => commitRename(s.id, s.title)}
                        onClick={(e) => e.stopPropagation()}
                      />
                    ) : (
                      <>
                        <span className={`status-dot st-${s.status}`} />
                        <span className="session-title">{s.title || t('新任务')}</span>
                        {s.area && <span className="session-area">{s.area}</span>}
                        <span className="session-time">{fmtTime(s.updated_at)}</span>
                      </>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
        ))}
        {sessions.length === 0 && <div className="sidebar-empty">{t('点击上方「新建任务」开始')}</div>}
      </div>

      {menu && (
        <ul
          className="ctx-menu"
          style={{ left: menu.x, top: menu.y }}
          onClick={(e) => e.stopPropagation()}
          onContextMenu={(e) => e.preventDefault()}
        >
          <li
            className="ctx-item"
            onClick={() => {
              const s = sessions.find((x) => x.id === menu.id);
              startRename(menu.id, s?.title ?? '');
            }}
          >
            <Icon name="pencil" size={15} /> {t('重命名')}
          </li>
          <li
            className="ctx-item danger"
            onClick={() => doDelete(menu.id)}
          >
            <Icon name="trash" size={15} /> {t('删除任务')}
          </li>
        </ul>
      )}
    </aside>
  );
}
