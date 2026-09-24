import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { Icon } from '../../../shared/ui/icons';
import type { Session } from '../../../services/contracts';
import { useSessionStore } from '../../sessions/store/sessionStore';
import { fmtTime, groupSessions } from '../../sessions/lib/sessionList';
import { useT } from '../../../shared/i18n';
import { ChatHistory, type ChatIndexItem } from './ChatHistory';
import './head-dock.css';

interface MenuState {
  id: string;
  x: number;
  y: number;
}

interface Props {
  chatIndex?: ChatIndexItem[];
  onJumpChat?: (id: string) => void;
}

// Deleting a task can remount this component (deleting the open task switches session), so
let dockTasksOpen = false;

/** The two mid-bar entries. The start page renders it with no question data — there is no */
export function HeadDock({ chatIndex = [], onJumpChat = () => undefined }: Props = {}) {
  const t = useT();
  const sessions = useSessionStore((s) => s.sessions);
  const currentId = useSessionStore((s) => s.currentId);
  const selectSession = useSessionStore((s) => s.selectSession);
  const deleteSession = useSessionStore((s) => s.deleteSession);
  const renameSession = useSessionStore((s) => s.renameSession);

  const [tasksOpen, setTasksOpenState] = useState(dockTasksOpen);
  const setTasksOpen = useCallback((v: boolean) => {
    dockTasksOpen = v;
    setTasksOpenState(v);
  }, []);
  const [menu, setMenu] = useState<MenuState | null>(null);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [draft, setDraft] = useState('');

  const tasksRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      setTasksOpen(false);
      setMenu(null);
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, []);

  useEffect(() => {
    if (!tasksOpen) return;
    const onDoc = (e: MouseEvent) => {
      if (menu || editingId) return;
      if (tasksRef.current && !tasksRef.current.contains(e.target as Node)) setTasksOpen(false);
    };
    document.addEventListener('mousedown', onDoc);
    return () => document.removeEventListener('mousedown', onDoc);
  }, [tasksOpen, menu, editingId]);

  useEffect(() => {
    if (!menu) return;
    const close = () => setMenu(null);
    window.addEventListener('click', close);
    return () => window.removeEventListener('click', close);
  }, [menu]);

  const groups = useMemo(() => groupSessions(sessions), [sessions]);

  const pick = (id: string) => {
    void selectSession(id);
    setTasksOpen(false);
  };

  /** "New task" returns to the start page — creating a task happens there, in its input. */
  const addNew = () => {
    useSessionStore.getState().clearCurrent();
    setTasksOpen(false);
  };

  const commitRename = (id: string, fallback: string) => {
    setEditingId(null);
    const next = draft.trim();
    if (next && next !== fallback) void renameSession(id, next);
  };

  const renderRow = (s: Session) =>
    editingId === s.id ? (
      <input
        key={s.id}
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
      />
    ) : (
      <button
        key={s.id}
        type="button"
        className={`hdock-item${s.id === currentId ? ' active' : ''}`}
        onClick={() => pick(s.id)}
        onContextMenu={(e) => {
          e.preventDefault();
          setMenu({ id: s.id, x: e.clientX, y: e.clientY });
        }}
      >
        <span className={`status-dot st-${s.status}`} />
        <span className="hdock-item-title">{s.title || t('新任务')}</span>
        <span className="hdock-item-time">{fmtTime(s.updated_at)}</span>
      </button>
    );

  const listBody = (
    <>
      {groups.map((g) => (
        <div key={g.label}>
          <div className="hdock-group">{g.label}</div>
          {g.items.map((s) => renderRow(s))}
        </div>
      ))}
      {sessions.length === 0 && <div className="hdock-empty">{t('还没有任务')}</div>}
    </>
  );

  return (
    <>
      <div className="hdock">
      <div
        className="hdock-slot"
        ref={tasksRef}
      >
        <button
          type="button"
          className={`hdock-btn${tasksOpen ? ' open' : ''}`}
          onClick={() => setTasksOpen(!tasksOpen)}
          aria-label={t('任务列表')}
          aria-expanded={tasksOpen}
          aria-haspopup="dialog"
        >
          <Icon name="list" size={15} />
          <span>{t('任务列表')}</span>
        </button>
        {tasksOpen && (
          <div
            className="hdock-pop hdock-pop--tasks"
            role="dialog"
            aria-label={t('任务列表')}
          >
            <button type="button" className="hdock-new" onClick={() => void addNew()}>
              <Icon name="plus" size={14} /> {t('新建任务')}
            </button>
            {/* The list renders even with zero tasks: its height is fixed at three rows, so the
                popover keeps the same size whether or not tasks exist. Unmounting it collapses
                the panel to a single row the moment the last task is deleted. */}
            <div className="hdock-sep" />
            <div className="hdock-list">{listBody}</div>
          </div>
        )}
      </div>

      <ChatHistory items={chatIndex} onJump={onJumpChat} />

      {menu && createPortal(
        <ul
          className="ctx-menu"
          style={{
            // .hdock carries a transform, which would make it the containing block of a
            // fixed child — so this menu is portalled to body to get real viewport
            // coordinates, and clamped so it can never open past the window edge.
            left: Math.min(menu.x, window.innerWidth - 170),
            top: Math.min(menu.y, window.innerHeight - 96),
          }}
        >
          <li
            className="ctx-item"
            onClick={() => {
              const s = sessions.find((x) => x.id === menu.id);
              setMenu(null);
              setEditingId(menu.id);
              setDraft(s?.title ?? '');
            }}
          >
            <Icon name="pencil" size={15} /> {t('重命名')}
          </li>
          <li
            className="ctx-item danger"
            onClick={() => {
              const id = menu.id;
              setMenu(null);
              void deleteSession(id);
            }}
          >
            <Icon name="trash" size={15} /> {t('删除任务')}
          </li>
        </ul>,
        document.body,
      )}
      </div>

    </>
  );
}
