import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { SessionSidebar } from './features/sessions/components/SessionSidebar';
import { ChatView } from './features/chat/ChatView';
import { Composer } from './features/chat/components/Composer';
import type { Attachment } from './features/chat/lib/attachments';
import { HeadDock } from './features/chat/components/HeadDock';
import { SettingsPanel } from './features/settings/components/SettingsPanel';
import { useDockSyncPublisher } from './app/dock/changedFiles';
import { useMainResizeFromDock } from './app/dock/mainResize';
import { ConfirmModal } from './app/ConfirmModal';
import { Toast } from './shared/ui/Toast';
import { ErrorBoundary } from './app/ErrorBoundary';
import { TitleBar } from './app/TitleBar';
import { Icon, type IconName } from './shared/ui/icons';
import { useSessionStore } from './features/sessions/store/sessionStore';
import { useSettings } from './features/settings/store/settingsStore';
import { useT } from './shared/i18n';
import { openExternal } from './shared/lib/openExternal';

/** Start-page guide: where things live, and what the thing is for.
 *
 *  The Chinese text is stored raw and translated at render time. Calling t() here would freeze it:
 *  a module-level constant is evaluated once, at import, so the cards would keep whatever language
 *  was active when the module loaded and never follow a later switch. */
const GUIDE: Array<{ k: string; v: string; icon: IconName }> = [
  {
    icon: 'sidebar',
    k: '左侧的任务',
    v: '鼠标移到窗口最左边，点露出的箭头拉出侧栏；展开后每一条就是一个任务。',
  },
  {
    icon: 'message',
    k: '切换与回溯',
    v: '标题栏正中两个入口：左边「任务列表」换任务，右边「历史提问」回到这个任务里问过的任何一句。',
  },
  {
    icon: 'sun',
    k: '设置与外观',
    v: '左上角 Real Agent 打开设置——主题、语言、模型都在里面。',
  },
  {
    icon: 'spark',
    k: '它能干什么',
    v: '读你的项目、跑命令、查日志，也能联网检索；每一步过程与花费都摊在面板上。',
  },
];

export default function App() {
  const t = useT();
  const { sessions, currentId, fetchSessions, selectSession, createSession, deleteSession, renameSession, error } =
    useSessionStore();
  const theme = useSettings((s) => s.theme);
  const sidebarCollapsed = useSettings((s) => s.sidebarCollapsed);
  const toggleSidebar = useSettings((s) => s.toggleSidebar);
  const defaultSystemPrompt = useSettings((s) => s.defaultSystemPrompt);
  const loadFromBackend = useSettings((s) => s.loadFromBackend);
  const [booting, setBooting] = useState(true);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [startText, setStartText] = useState('');
  const [startAtts, setStartAtts] = useState<Attachment[]>([]);
  const [startSkills, setStartSkills] = useState<string[]>([]);
  const [starting, setStarting] = useState(false);
  const startInputRef = useRef<HTMLTextAreaElement>(null);

  // The dock is a separate webview with empty stores; this is what feeds it.
  useDockSyncPublisher();
  // ...and this is how a drag on its top/bottom edge reaches the window it has to resize.
  useMainResizeFromDock();

  useEffect(() => {
    document.documentElement.setAttribute('data-theme', theme);
  }, [theme]);

  // Last line of defence for links. In a Tauri webview the default behaviour of <a href> is to
  // navigate the CURRENT window, so a single unhandled link replaces the entire app with the target
  // page and the user has no way back. Components route their own links (and call preventDefault),
  // and this runs on the bubble phase -- after React's delegated handlers -- so those are skipped
  // rather than opened twice.
  useEffect(() => {
    const onClick = (e: MouseEvent) => {
      if (e.defaultPrevented) return;
      const a = (e.target as HTMLElement)?.closest?.('a[href]');
      if (!a) return;
      const href = a.getAttribute('href') ?? '';
      // An in-page anchor is the one default worth keeping.
      if (!href || href.startsWith('#')) return;
      e.preventDefault();
      if (/^https?:\/\//i.test(href)) void openExternal(href);
    };
    document.addEventListener('click', onClick);
    return () => document.removeEventListener('click', onClick);
  }, []);

  useEffect(() => {
    loadFromBackend().catch(() => undefined);
    // No auto-open of the last task: launch lands on the start page, which is where the
    // guide lives. Picking a task stays an explicit move (task list in the title bar).
    fetchSessions().finally(() => setBooting(false));
  }, [fetchSessions, loadFromBackend]);

  useEffect(() => {
    const RUNNING = ['planning', 'executing', 'solving', 'reflecting'];
    const hasRunning = sessions.some((s) => !!s.status && RUNNING.includes(s.status));
    if (!hasRunning) return;
    // Refresh sidebar status only. Following whichever task happens to be running is
    // deliberately gone: it yanks the panel away from what the user is reading or typing.
    const t = window.setInterval(() => {
      fetchSessions().catch(() => undefined);
    }, 15_000);
    return () => window.clearInterval(t);
  }, [sessions, fetchSessions]);

  useEffect(() => {
    if (!currentId || sessions.length === 0) return;
    if (sessions.some((s) => s.id === currentId)) return;
    const best = sessions.reduce((a, b) =>
      new Date(b.updated_at) > new Date(a.updated_at) ? b : a,
    );
    selectSession(best.id);
  }, [sessions, currentId, selectSession]);

  /** Create a task and, when the start page supplied a line, hand it to ChatView which
   *  sends it once mounted — so there is exactly one send path in the app. */
  // The start page is "the chat before a task exists": focus the box so typing works at once.
  useEffect(() => {
    if (!currentId && !booting) startInputRef.current?.focus();
  }, [currentId, booting]);

  const newTask = async (first?: string, atts?: Attachment[]) => {
    // No text and no attachment means the entry point was the "new task" button, not a send:
    // go back to the start page instead of opening an empty session, so both entry points agree.
    if (!first && !(atts && atts.length > 0)) {
      useSessionStore.getState().clearCurrent();
      return;
    }
    const session = await createSession(t('新任务'), defaultSystemPrompt || undefined);
    // Attachments ride with the first line (ChatView sends it after mount); drop them here and
    useSessionStore.getState().setPendingFirst({ id: session.id, text: first ?? '', atts });
    await selectSession(session.id);
  };

  const startTask = () => {
    const raw = startText.trim();
    // Skill prefix is built here so the start page's first line carries the same `/skill text`
    const prefix = startSkills.map((n) => `/${n}`).join(' ');
    const text = prefix ? `${prefix} ${raw}`.trim() : raw;
    if (!text && startAtts.length === 0) return;
    const atts = startAtts;
    setStartText('');
    setStartAtts([]);
    setStartSkills([]);
    setStarting(true);
    void newTask(text, atts).finally(() => setStarting(false));
  };

  if (booting) {
    return (
      <div className="boot">
        <Icon name="flame" size={26} className="boot-flame" />
        <span className="boot-name">Real</span>
        <span className="boot-hint">{t('火种点燃中…')}</span>
      </div>
    );
  }

  return (
    <div className={`app${sidebarCollapsed ? ' sidebar-collapsed' : ''}`}>
      <TitleBar onOpenSettings={() => setSettingsOpen(true)} />
      <div className="app-body">
        <button
          type="button"
          className="sidebar-hotzone"
          onMouseDown={(e) => e.preventDefault()}
          onClick={toggleSidebar}
          title={sidebarCollapsed ? t('展开侧栏') : t('收起侧栏')}
          aria-label={t('开关侧栏')}
        >
          {/* Same icon as the dock's hotzone on the other edge -- one affordance, mirrored. The
              chevron points into the panel while it is closed and back out while it is open. */}
          <span className="hz-arrow" aria-hidden="true">
            <Icon name="chevron" size={19} strokeWidth={2.6} />
          </span>
        </button>
        <SessionSidebar
          sessions={sessions}
          currentId={currentId}
          onSelect={selectSession}
          onNew={() => void newTask()}
          onDelete={deleteSession}
          onRename={renameSession}
        />
        <main className="main">
          {error && <div className="banner-error" role="alert">{error}</div>}
          {currentId ? (
            <ErrorBoundary>
              {/* No key on purpose: remounting ChatView on every session switch tears down
                  its children too, which makes the task panel blink when the open task is
                  deleted. Session changes are handled by ChatView's own [sessionId] effect. */}
              <ChatView sessionId={currentId} />
            </ErrorBoundary>
          ) : (
            <div className="chat">
              <div className="chat-head">
                <span className="spacer" />
                <HeadDock />
              </div>
              <div className="empty-state">
                <div className="empty-inner">
                  <div className="empty-brand">
                    <Icon name="flame" size={44} className="mark" />
                    <h2>Real</h2>
                    <p className="slogan">{t('星火可以燎原')}</p>
                  </div>

                  <div className="empty-guide">
                    {GUIDE.map((g) => (
                      <div className="guide-card" key={g.k}>
                        <div className="guide-head">
                          <Icon name={g.icon} size={15} />
                          <span className="guide-k">{t(g.k)}</span>
                        </div>
                        <p className="guide-v">{t(g.v)}</p>
                      </div>
                    ))}
                  </div>

                  {/* One composer for the start page and for a task: drag/paste attachments, model
                      picker, skills and the danger-confirm toggle come along with it. */}
                  <div className="start-composer">
                    <Composer
                      value={startText}
                      onChange={setStartText}
                      attachments={startAtts}
                      onAttachmentsChange={setStartAtts}
                      onSend={startTask}
                      onCancel={() => undefined}
                      onKeyDown={(e) => {
                        // IME: Enter inside a candidate window commits text, it must not submit.
                        if (e.nativeEvent.isComposing || e.keyCode === 229) return;
                        if (e.key === 'Enter' && !e.shiftKey) {
                          e.preventDefault();
                          startTask();
                        }
                      }}
                      inputRef={startInputRef}
                      busy={false}
                      sending={starting}
                      showJumpDown={false}
                      onJumpToBottom={() => undefined}
                      onManageSkills={() => setSettingsOpen(true)}
                      pickedSkill={startSkills}
                      onPickSkill={(n) => setStartSkills((cur) => (cur.includes(n) ? cur : [...cur, n]))}
                      onRemoveSkill={(n) => setStartSkills((cur) => cur.filter((x) => x !== n))}
                      sendLabelIdle={t('开始')}
                    />
                  </div>
                </div>
              </div>
            </div>
          )}
        </main>
        {/* Mirrors .sidebar-hotzone on the opposite edge: the right dock opens from a hover
            affordance at the seam, not from a permanent button in the toolbar. */}
        <button
          type="button"
          className="dock-hotzone"
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => void invoke('open_dock_window')}
          title={t('打开右侧栏')}
          aria-label={t('打开右侧栏')}
        >
          <span className="hz-arrow" aria-hidden="true">
            <Icon name="chevron" size={19} strokeWidth={2.6} />
          </span>
        </button>
      </div>
      {settingsOpen && <SettingsPanel onClose={() => setSettingsOpen(false)} />}
 {/* Global layers: danger-op confirmation must stay visible even after switching
 sessions (backend stalls the task 120s then denies if nobody answers). */}
      <ConfirmModal />
      <Toast />
    </div>
  );
}
