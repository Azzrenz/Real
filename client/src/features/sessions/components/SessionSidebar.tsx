import { useEffect, useMemo, useState } from 'react';
import type { Session } from '../../../services/contracts';
import { Icon } from '../../../shared/ui/icons';
import { fmtTime, groupSessions } from '../lib/sessionList';
import { useIsMobile } from '../../../shared/lib/viewport';
import { useChatStore } from '../../chat/store/chatStore';
import { useT } from '../../../shared/i18n';

interface Props {
  sessions: Session[];
  currentId: string | null;
  onSelect: (id: string) => void;
  onNew: () => void;
  onDelete: (id: string) => void;
  onRename: (id: string, title: string) => void;
  /** 手机版抽屉底部的「设置」入口。桌面不传（顶部左上角那个品牌按钮就是设置入口）。 */
  onOpenSettings?: () => void;
  /** 手机版抽屉底部的连接信息（我连的是哪台电脑）。桌面为空。 */
  connHost?: string;
}

/** 后端把这些状态读作「这个任务正在跑」。用于把转圈和"需要我"分开。 */
const RUNNING_STATUSES = new Set(['planning', 'executing', 'solving', 'reflecting', 'replanning']);

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
  onOpenSettings,
  connHost,
}: Props) {
  const t = useT();
  const isMobile = useIsMobile();
  // 确认门是全局单例（同一时刻只可能有一个待处理），它绑在哪个会话上，哪一行就标"等你审批"。
  const confirmSessionId = useChatStore((s) => (s.confirm ? s.confirm.sessionId : null));
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
                        {/* 手机端把「在跑」和「需要我」分开：
                            在跑 = 转圈（绿点只说"活着"，说不清是干活还是卡住等回复）；
                            等你审批 = 一颗琥珀点（最该被点开的那一行）。
                            两者都只占 .status-dot 那个 6px 槽（用负 margin 抵消宽度差），
                            否则行的左边缘会随状态跳动。桌面完全走原来的点。 */}
                        {isMobile && s.id === confirmSessionId ? (
                          <span className="session-need" title={t('等你审批')} aria-label={t('等你审批')} />
                        ) : isMobile && RUNNING_STATUSES.has(s.status) ? (
                          /* Icon 的默认 strokeWidth=1.8 是给 24 viewBox 的，缩到 16px 后实际只有
                             ~1.2px 细，白底上看不出是个转圈。这里加粗到等效 ~1.9px。 */
                          <Icon name="spinner" size={16} strokeWidth={2.8} className="session-spin" aria-hidden />
                        ) : (
                          <span className={`status-dot st-${s.status}`} />
                        )}
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

      {/* 手机版：抽屉底部的设置入口（对标移动端对话产品的「我的」区）。
          桌面不渲染 —— 桌面左上角那个品牌按钮就是设置入口。 */}
      {onOpenSettings && (
        <div className="m-drawer-foot">
          {/* 对标参考图抽屉底部的「天籁行者 / 高级版」那块：这里是「我在哪台电脑上」。
              标题栏也有一份 —— 那份是干活时随时可见，这份是管理任务时顺手确认。 */}
          {connHost && (
            <div className="m-drawer-conn">
              <span className="m-drawer-conn-dot" aria-hidden />
              {t('已连接 · {host}', { host: connHost })}
            </div>
          )}
          <button
            type="button"
            className="m-drawer-foot-btn"
            onClick={onOpenSettings}
          >
            <Icon name="sliders" size={18} />
            <span>{t('设置')}</span>
          </button>
        </div>
      )}

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
