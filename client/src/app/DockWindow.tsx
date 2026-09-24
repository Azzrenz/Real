import { useCallback, useEffect, useState } from 'react';
import type { MouseEvent as ReactMouseEvent } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { emit, listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { FilePreview } from './FilePreview';
import { FilesPanel } from './dock/FilesPanel';
import { TaskPanel } from './dock/TaskPanel';
import { ArtifactsPanel } from './dock/ArtifactsPanel';
import { Icon } from '../shared/ui/icons';
import type { IconName } from '../shared/ui/icons';
import { CopyButton } from '../shared/ui/CopyButton';
import { useSettings } from '../features/settings/store/settingsStore';
import { DOCK_RESIZE_MAIN } from './dock/mainResize';
import { useDockThemeSync } from './dock/changedFiles';
import { openPathExternal, revealInFolder } from '../shared/lib/openExternal';
import { useT } from '../shared/i18n';
import './dock.css';

declare global {
  interface Window {
    /** Injected by Rust before load: the file this dock should open, if any. */
    __REAL_DOCK_FILE__?: string | null;
  }
}

type TabId = 'files' | 'task' | 'artifacts';

/** Mirrors the shell's GitState. Status codes are passed through raw (` M` / `??` / `A `), because */
interface GitChange {
  status: string;
  path: string;
}
interface GitState {
  root: string;
  branch: string;
  changes: GitChange[];
}

/** One tab per kind of information: where the files are, what this run is doing, what it produced. */
const TABS: { id: TabId; label: string; icon: IconName }[] = [
  { id: 'files', label: '文件', icon: 'folder' },
  { id: 'task', label: '任务', icon: 'list' },
  { id: 'artifacts', label: '产物', icon: 'doc' },
];

/** Last segment of a path -- what a file tab shows. */
function baseName(p: string): string {
  const parts = p.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? p;
}

/** The right-dock window: a third webview of the same bundle, built by Rust and snapped to the */
export function DockWindow() {
  // useT subscribes to the language, so the panel re-renders when the main window pushes a change.
  const t = useT();
  const win = () => getCurrentWindow();
  const [tab, setTab] = useState<TabId>('files');
  // Every opened file is a tab, and the last one opened is the one on screen -- the same model the
  // editor next door uses. Opening a second file must never mean losing the first.
  const [openFiles, setOpenFiles] = useState<string[]>(() =>
    window.__REAL_DOCK_FILE__ ? [window.__REAL_DOCK_FILE__] : [],
  );
  const [path, setPath] = useState<string | null>(window.__REAL_DOCK_FILE__ ?? null);
  const [docked, setDocked] = useState(true);
  const [railOpen, setRailOpen] = useState(true);
  const [git, setGit] = useState<GitState | null>(null);
  const [commitOpen, setCommitOpen] = useState(false);
  const [commitMsg, setCommitMsg] = useState('');
  const [commitNote, setCommitNote] = useState<string | null>(null);
  const [commitBusy, setCommitBusy] = useState(false);
  const theme = useSettings((s) => s.theme);

  // Follow a theme switch made in the main window (separate window, separate store -- see the hook).
  useDockThemeSync();

  // Which repository is this file in, and what is uncommitted there? Asked per file, not once at
  // startup: the tree starts at "This PC", so the dock can be pointed at any folder on the disk.
  // A folder that is not a repository is the normal case, so a refusal clears the state silently.
  useEffect(() => {
    setCommitOpen(false);
    if (!path) {
      setGit(null);
      return;
    }
    let alive = true;
    invoke<GitState>('git_state', { path })
      .then((s) => {
        if (alive) setGit(s);
      })
      .catch(() => {
        if (alive) setGit(null);
      });
    return () => {
      alive = false;
    };
  }, [path]);

/** Commit the repository the current file lives in. Nothing happens without a typed message -- */
  const runCommit = () => {
    if (!path || !commitMsg.trim() || commitBusy) return;
    setCommitBusy(true);
    setCommitNote(null);
    invoke<string>('git_commit', { path, message: commitMsg })
      .then((hash) => {
        setCommitMsg('');
        setCommitOpen(false);
        setCommitNote(`${t('已提交')} ${hash}`);
        window.setTimeout(() => setCommitNote(null), 5000);
        return invoke<GitState>('git_state', { path }).then(setGit);
      })
      .catch((e) => setCommitNote(String(e)))
      .finally(() => setCommitBusy(false));
  };

  // Every window is its own document, so the data-theme App sets on the main window never reaches
  // this one -- without this the dock falls back to the :root default and renders dark.
  useEffect(() => {
    document.documentElement.setAttribute('data-theme', theme);
  }, [theme]);

/** The single entry point for "show me this file": the tree, a path clicked in the chat, and a */
  const openFile = useCallback((p: string) => {
    setOpenFiles((prev) => (prev.includes(p) ? prev : [...prev, p]));
    setPath(p);
  }, []);


  const closeTab = useCallback(
    (p: string) => {
      const idx = openFiles.indexOf(p);
      const next = openFiles.filter((x) => x !== p);
      setOpenFiles(next);
      if (path === p) {
        setPath(next.length ? next[Math.min(idx, next.length - 1)] : null);
      }
    },
    [openFiles, path],
  );

  // The window is transparent so the CSS radius can show, but html/body still paint their own
  // background -- without this the rounded corners sit on a white wedge.
  useEffect(() => {
    const h = document.documentElement.style.background;
    const b = document.body.style.background;
    document.documentElement.style.background = 'transparent';
    document.body.style.background = 'transparent';
    return () => {
      document.documentElement.style.background = h;
      document.body.style.background = b;
    };
  }, []);

  // A file handed over from the chat takes over the surface -- and joins the tabs, so whatever was
  // already open is one click away instead of gone.
  useEffect(() => {
    const pending = listen<string>('dock:open-file', (e) => openFile(e.payload));
    return () => {
      void pending.then((un) => un());
    };
  }, [openFile]);

  // Separation is a button, not a drag: dragging the panel away would leave it floating at some
  // arbitrary offset from the window it belongs to. Docked state is owned by Rust and reflected
  // here, so the two never disagree.
  useEffect(() => {
    const pending = listen<boolean>('dock:docked-changed', (e) => setDocked(e.payload));
    return () => {
      void pending.then((un) => un());
    };
  }, []);

  const redock = () => {
    const next = !docked;
    setDocked(next);
    void invoke('dock_set_docked', { docked: next }).catch(() => setDocked(!next));
  };

/** What is being viewed decides how wide the panel has to be -- PDF needs a page width, an HTML */
  useEffect(() => {
    void invoke('dock_fit', { path }).catch(() => undefined);
  }, [path]);

/** Start an edge resize from one of the grips below. */
  const startResize = (edge: 'north' | 'east' | 'south' | 'west') => {
    if (docked && edge !== 'east') {
      void emit(DOCK_RESIZE_MAIN, edge).catch(() => undefined);
      return;
    }
    const dir =
      edge === 'north' ? 'North' : edge === 'south' ? 'South' : edge === 'west' ? 'West' : 'East';
    void win().startResizeDragging(dir).catch(() => undefined);
  };

/** Left button only: the system resize loop is a left-button drag, so accepting anything else */
  const gripDown = (edge: 'north' | 'east' | 'south' | 'west') => (e: ReactMouseEvent) => {
    if (e.button !== 0) return;
    e.preventDefault();
    startResize(edge);
  };

  return (
    <>
      {/* Resize edges. Deliberately OUTSIDE .dock-root: that element clips to its own box
          (overflow:hidden), which would cut off any grip reaching into the window's transparent
          shadow margin -- and that 32px margin is the free space that makes the grab area big.
          A thin strip is what the pointer has to find while the panel is already edge-to-edge.
          Order matters only in the 6px corners: the edge grips come last, so they win there. */}
      {(['north', 'south'] as const).map((edge) => (
        <span
          key={edge}
          className={`dock-grip dock-grip--${edge === 'north' ? 'n' : 's'}`}
          role="separator"
          aria-orientation="horizontal"
          onMouseDown={gripDown(edge)}
        />
      ))}
      <span
        className="dock-grip dock-grip--w"
        role="separator"
        aria-orientation="vertical"
        onMouseDown={gripDown('west')}
      />
      <span
        className="dock-grip dock-grip--e"
        role="separator"
        aria-orientation="vertical"
        onMouseDown={gripDown('east')}
      />

      <div className="dock-root">
      {/* The dock's only chrome row. It used to be two -- a title bar plus an empty toolbar, 68px
          of nothing above the content. Merged into one 36px row the panel gets that space back,
          and the row still lines up with .chat-head across the seam.
          It drags only when detached: while docked the window is pinned to the seam, and letting
          it be dragged would just make it fight the snap-back. */}
      <header
        className="dock-head"
        onMouseDown={() => {
          if (docked) return;
          void win().startDragging();
        }}
      >
        <span className="dock-head-drag" />
        <span className="dock-head-actions" onMouseDown={(e) => e.stopPropagation()}>
          {/* Always present: this is the only way to separate or re-attach, so it must be
              reachable in both states. */}
          <button
            type="button"
            className={`dock-head-btn${docked ? '' : ' is-off'}`}
            onClick={redock}
            title={docked ? t('与主窗口分离') : t('吸附到主窗口')}
            aria-label={docked ? t('与主窗口分离') : t('吸附到主窗口')}
            aria-pressed={!docked}
          >
            <Icon name="target" size={13} />
          </button>
          <button
            type="button"
            className="dock-head-btn"
            onClick={() => void win().minimize()}
            title={t('最小化')}
            aria-label={t('最小化')}
          >
            <Icon name="minimize" size={13} />
          </button>
          <button
            type="button"
            className="dock-head-btn is-close"
            onClick={() => void win().close()}
            title={t('关闭')}
            aria-label={t('关闭')}
          >
            <Icon name="x" size={13} />
          </button>
        </span>
      </header>

      <div className="dock-main">
        <nav
          className={`dock-rail${railOpen ? '' : ' is-closed'}`}
          role="tablist"
          aria-label={t('侧栏')}
        >
          {TABS.map((x) => (
            <button
              key={x.id}
              type="button"
              role="tab"
              aria-selected={tab === x.id}
              className={`dock-tab${tab === x.id ? ' on' : ''}`}
              onClick={() => {
                setTab(x.id);
                setPath(null);
              }}
            >
              <Icon name={x.icon} size={18} />
              <span className="dock-tab-label">{t(x.label)}</span>
            </button>
          ))}
          {/* The toggle lives at the foot of the rail it collapses, so the control sits with the
              thing it acts on, and it only changes the rail -- the window and body keep their size. */}
          <button
            type="button"
            className="dock-rail-toggle"
            onClick={() => setRailOpen(!railOpen)}
            title={railOpen ? t('收起列表') : t('展开列表')}
            aria-label={railOpen ? t('收起列表') : t('展开列表')}
            aria-expanded={railOpen}
          >
            <Icon name="chevron" size={13} />
          </button>
        </nav>

        <main className="dock-body" role="tabpanel">
          {openFiles.length > 0 && (
            <div className="dock-tabbar">
              <div className="dock-tabs" role="tablist">
                {openFiles.map((f) => (
                  <span key={f} className={`dock-file-tab${f === path ? ' on' : ''}`}>
                    <button type="button" className="dft-name" onClick={() => setPath(f)} title={f}>
                      {baseName(f)}
                    </button>
                    <button
                      type="button"
                      className="dft-x"
                      onClick={() => closeTab(f)}
                      title={t('关闭')}
                      aria-label={t('关闭')}
                    >
                    <Icon name="x" size={22} />
                  </button>
                </span>
              ))}
              </div>
              {/* Two things you do *to the file you are looking at*, so they sit with the file tabs
                  rather than up in the window bar: show it in the file manager, or hand it to
                  whichever program owns it. Both are about the current file, so they are disabled
                  when no file is showing -- the empty tab row has nothing to act on. */}
              <span className="dock-tab-actions">
                <button
                  type="button"
                  className="dock-act"
                  disabled={!path}
                  onClick={() => {
                    if (path) void revealInFolder(path);
                  }}
                  title={t('打开所在文件夹')}
                  aria-label={t('打开所在文件夹')}
                >
                  <Icon name="folder" size={13} />
                </button>
                <button
                  type="button"
                  className="dock-act"
                  disabled={!path}
                  onClick={() => {
                    if (path) void openPathExternal(path);
                  }}
                  title={t('在外部打开')}
                  aria-label={t('在外部打开')}
                >
                  <Icon name="external" size={13} />
                </button>
                {/* The path is what the chat talks in -- a report says "line 42 is wrong in
                    utils.rs" and the answer needs the whole path back -- so copying it is a
                    first-class action here rather than something to do by hand. */}
                {path && <CopyButton text={path} className="dock-act" title={t('复制路径')} />}
                {/* Commit lives here rather than in a menu: this panel is where the work is looked
                    at, so "keep this" is the natural next move. The file count rides on the button
                    because the commit covers the whole repository, not just this file. */}
                {path && git && git.changes.length > 0 && (
                  <button
                    type="button"
                    className={`dock-act dock-act-text${commitOpen ? ' on' : ''}`}
                    onClick={() => setCommitOpen((v) => !v)}
                    title={`${t('提交改动')} · ${git.branch} · ${git.changes.length} ${t('个文件')}`}
                    aria-label={t('提交改动')}
                    aria-expanded={commitOpen}
                  >
                    {t('提交')}
                    <span className="dock-act-badge">{git.changes.length}</span>
                  </button>
                )}
              </span>
            </div>
          )}

          {(commitOpen || commitNote) && (
            <div className="dock-commit">
              {commitOpen ? (
                <>
                  <input
                    className="dock-commit-input"
                    value={commitMsg}
                    placeholder={t('提交说明')}
                    autoFocus
                    onChange={(e) => setCommitMsg(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter') runCommit();
                      if (e.key === 'Escape') setCommitOpen(false);
                    }}
                  />
                  <span className="dock-commit-scope">
                    {t('将提交')} {git?.changes.length ?? 0}
                  </span>
                  <button
                    type="button"
                    className="dock-commit-go"
                    disabled={!commitMsg.trim() || commitBusy}
                    onClick={runCommit}
                  >
                    {commitBusy ? t('提交中…') : t('提交')}
                  </button>
                </>
              ) : (
                <span className="dock-commit-note">{commitNote}</span>
              )}
            </div>
          )}

          {path ? (
            <FilePreview path={path} onOpenPath={openFile} />
          ) : (
            <>
              {tab === 'files' && <FilesPanel activePath={path} onOpen={openFile} />}
              {tab === 'task' && <TaskPanel onOpen={openFile} />}
              {tab === 'artifacts' && <ArtifactsPanel onOpen={openFile} />}
            </>
          )}
        </main>
      </div>
      </div>
    </>
  );
}
