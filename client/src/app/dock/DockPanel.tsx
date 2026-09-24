import { useCallback, useEffect, useRef, useState } from 'react';
import type { PointerEvent as ReactPointerEvent } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { FilePreview } from '../FilePreview';
import { FilesPanel } from './FilesPanel';
import { TaskPanel } from './TaskPanel';
import { ArtifactsPanel } from './ArtifactsPanel';
import { Icon } from '../../shared/ui/icons';
import type { IconName } from '../../shared/ui/icons';
import { CopyButton } from '../../shared/ui/CopyButton';
import { openPathExternal, revealInFolder } from '../../shared/lib/openExternal';
import { useT } from '../../shared/i18n';
import { useDockPanel, DOCK_PANEL_MIN, clampPanelWidth } from './dockPanelStore';
import '../dock.css';

type TabId = 'files' | 'task' | 'artifacts';

const TABS: { id: TabId; label: string; icon: IconName }[] = [
  { id: 'files', label: '文件', icon: 'folder' },
  { id: 'task', label: '任务', icon: 'list' },
  { id: 'artifacts', label: '产物', icon: 'doc' },
];

function baseName(p: string): string {
  const parts = p.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? p;
}

export function DockPanel() {
  const t = useT();
  const open = useDockPanel((s) => s.open);
  const storedWidth = useDockPanel((s) => s.width);
  // The width is remembered, and the window may get narrower after it was stored
  // (including the shell giving room to the detached dock window). Clamp again at
  // render time so the chat column can never be eaten; the stored preference comes
  // back on its own once the window is wide again.
  const [vw, setVw] = useState(() => window.innerWidth);
  useEffect(() => {
    const onResize = () => setVw(window.innerWidth);
    window.addEventListener('resize', onResize);
    return () => window.removeEventListener('resize', onResize);
  }, []);
  const width = clampPanelWidth(storedWidth, vw);
  const setWidth = useDockPanel((s) => s.setWidth);
  const setOpen = useDockPanel((s) => s.setOpen);
  const path = useDockPanel((s) => s.path);
  const files = useDockPanel((s) => s.files);
  const selectFile = useDockPanel((s) => s.selectFile);
  const closeFile = useDockPanel((s) => s.closeFile);

  const [tab, setTab] = useState<TabId>('files');
  const [railOpen, setRailOpen] = useState(true);
  const dragFrom = useRef<{ x: number; w: number } | null>(null);

  const onSeamDown = useCallback(
    (e: ReactPointerEvent) => {
      if (e.button !== 0) return;
      e.preventDefault();
      dragFrom.current = { x: e.clientX, w: width };
      e.currentTarget.setPointerCapture?.(e.pointerId);
    },
    [width],
  );

  const onSeamMove = useCallback(
    (e: ReactPointerEvent) => {
      const from = dragFrom.current;
      if (!from) return;
      setWidth(from.w + (from.x - e.clientX));
    },
    [setWidth],
  );

  const onSeamUp = useCallback((e: ReactPointerEvent) => {
    dragFrom.current = null;
    e.currentTarget.releasePointerCapture?.(e.pointerId);
  }, []);

  const detach = () => {
    void invoke('open_dock_window', { path, focus: true })
      .then(() => setOpen(false))
      .catch(() => useDockPanel.getState().setOpen(true));
  };

  if (!open) return null;

  return (
    <aside className="dock-panel" style={{ width }} aria-label={t('侧栏')}>
      <span
        className="dock-seam"
        role="separator"
        aria-orientation="vertical"
        aria-label={t('拖动调整宽度')}
        onPointerDown={onSeamDown}
        onPointerMove={onSeamMove}
        onPointerUp={onSeamUp}
        onDoubleClick={() => setWidth(DOCK_PANEL_MIN)}
      />
      <div className="dock-panel-head">
        <span className="dock-panel-title">{t('侧栏')}</span>
        <span className="dock-head-actions">
          <button
            type="button"
            className="dock-head-btn"
            onClick={detach}
            title={t('分离为独立窗口')}
            aria-label={t('分离为独立窗口')}
          >
            <Icon name="target" size={13} />
          </button>
          <button
            type="button"
            className="dock-head-btn is-close"
            onClick={() => setOpen(false)}
            title={t('关闭侧栏')}
            aria-label={t('关闭侧栏')}
          >
            <Icon name="x" size={13} />
          </button>
        </span>
      </div>

      <div className="dock-main">
        <nav className={`dock-rail${railOpen ? '' : ' is-closed'}`} role="tablist" aria-label={t('侧栏')}>
          {TABS.map((x) => (
            <button
              key={x.id}
              type="button"
              role="tab"
              aria-selected={tab === x.id}
              className={`dock-tab${tab === x.id ? ' on' : ''}`}
              onClick={() => {
                setTab(x.id);
                useDockPanel.setState({ path: null });
              }}
            >
              <Icon name={x.icon} size={18} />
              <span className="dock-tab-label">{t(x.label)}</span>
            </button>
          ))}
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
          {files.length > 0 && (
            <div className="dock-tabbar">
              <div className="dock-tabs" role="tablist">
                {files.map((f) => (
                  <span key={f} className={`dock-file-tab${f === path ? ' on' : ''}`}>
                    <button type="button" className="dft-name" onClick={() => selectFile(f)} title={f}>
                      {baseName(f)}
                    </button>
                    <button
                      type="button"
                      className="dft-x"
                      onClick={() => closeFile(f)}
                      title={t('关闭')}
                      aria-label={t('关闭')}
                    >
                      <Icon name="x" size={22} />
                    </button>
                  </span>
                ))}
              </div>
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
                {path && <CopyButton text={path} className="dock-act" title={t('复制路径')} />}
              </span>
            </div>
          )}

          {path ? (
            <FilePreview path={path} onOpenPath={selectFile} />
          ) : (
            <>
              {tab === 'files' && <FilesPanel activePath={path} onOpen={selectFile} />}
              {tab === 'task' && <TaskPanel onOpen={selectFile} />}
              {tab === 'artifacts' && <ArtifactsPanel onOpen={selectFile} />}
            </>
          )}
        </main>
      </div>
    </aside>
  );
}
