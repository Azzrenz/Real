/* ═══════════════════════════════════════════════════════════════
   Custom title bar (Tauri decorations:false): drag area plus min/max/close buttons.
   Button order matches the native one; close minimises to the taskbar, it does not quit.
   ═════════════════════════════════════════════════════════════════ */
import { useEffect, useRef, useState } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { Icon } from '../shared/ui/icons';
import { useT } from '../shared/i18n';
import brandLogo from '../shared/assets/brand.png';

interface ViewToggle {
  label?: string;
  onClick: () => void;
}
interface TitleBarProps {
  title?: string;
  viewToggle?: ViewToggle;
  onHome?: () => void;
  onOpenSettings?: () => void;
}

export function TitleBar({ viewToggle, onHome, onOpenSettings }: TitleBarProps) {
  const t = useT();
  const win = () => getCurrentWindow();
  const minimize = () => win().minimize().catch(() => undefined);
  const toggleMax = () => win().toggleMaximize().catch(() => undefined);
  const closeToTray = () => win().hide().catch(() => undefined);

  const [openMenu, setOpenMenu] = useState<'edit' | 'help' | null>(null);
  const [aboutOpen, setAboutOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!openMenu) return;
    const onDoc = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) setOpenMenu(null);
    };
    document.addEventListener('mousedown', onDoc);
    return () => document.removeEventListener('mousedown', onDoc);
  }, [openMenu]);

  const runEdit = (cmd: 'undo' | 'redo' | 'cut' | 'copy' | 'paste') => {
    const ae = document.activeElement as HTMLElement | null;
    const editable = ae && (ae.tagName === 'TEXTAREA' || ae.tagName === 'INPUT' || ae.isContentEditable);
    if (!editable) {
      const ta = document.querySelector<HTMLTextAreaElement>('.chat-input-box textarea');
      if (ta) ta.focus();
    }
    document.execCommand(cmd);
    setOpenMenu(null);
  };

  const EDIT_ITEMS: Array<{ label: string; cmd?: 'undo' | 'redo' | 'cut' | 'copy' | 'paste'; sep?: boolean }> = [
    { label: t('撤销'), cmd: 'undo' },
    { label: t('重做'), cmd: 'redo' },
    { label: '', sep: true },
    { label: t('剪切'), cmd: 'cut' },
    { label: t('复制'), cmd: 'copy' },
    { label: t('粘贴'), cmd: 'paste' },
  ];

  return (
    <div className="titlebar">
      <button
        type="button"
        className="titlebar-brand"
        onMouseDown={(e) => e.preventDefault()}
        onClick={() => onOpenSettings?.()}
        title={t('设置')}
        aria-label={t('打开设置')}
      >
        <img src={brandLogo} alt="Real" className="titlebar-logo brand-mark" />
        <b>Real Agent</b>
      </button>
      <nav className="titlebar-menu" ref={menuRef}>
        <div className={`titlebar-menu-item-wrap${openMenu === 'edit' ? ' open' : ''}`}>
          <button
            type="button"
            className="titlebar-menu-item"
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => setOpenMenu(openMenu === 'edit' ? null : 'edit')}
            aria-haspopup="menu"
            aria-expanded={openMenu === 'edit'}
          >
            {t('编辑')}
          </button>
          {openMenu === 'edit' && (
            <div className="titlebar-menu-pop" role="menu">
              {EDIT_ITEMS.map((it, idx) =>
                it.sep ? (
                  <span key={idx} className="menu-sep" role="separator" />
                ) : (
                  <button
                    key={it.label}
                    type="button"
                    className="menu-item"
                    role="menuitem"
                    onClick={() => it.cmd && runEdit(it.cmd)}
                  >
                    {it.label}
                  </button>
                ),
              )}
            </div>
          )}
        </div>
        <div className={`titlebar-menu-item-wrap${openMenu === 'help' ? ' open' : ''}`}>
          <button
            type="button"
            className="titlebar-menu-item"
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => setOpenMenu(openMenu === 'help' ? null : 'help')}
            aria-haspopup="menu"
            aria-expanded={openMenu === 'help'}
          >
            {t('帮助')}
          </button>
          {openMenu === 'help' && (
            <div className="titlebar-menu-pop" role="menu">
              <button
                type="button"
                className="menu-item"
                role="menuitem"
                onClick={() => {
                  setOpenMenu(null);
                  setAboutOpen(true);
                }}
              >
                {t('关于 Real Agent…')}
              </button>
            </div>
          )}
        </div>
        {onHome && (
          <button type="button" className="titlebar-home" onMouseDown={(e) => e.preventDefault()} onClick={onHome} data-tip={t('返回项目入口')}>
            <Icon name="enter" size={16} strokeWidth={1.9} />
          </button>
        )}
      </nav>
      <div className="titlebar-drag" data-tauri-drag-region />
      {viewToggle && (
        <button
          type="button"
          className="titlebar-view"
          onMouseDown={(e) => e.preventDefault()}
          onClick={viewToggle.onClick}
          data-tip={t('切换视图')}
        >
          <Icon name="swap" size={16} strokeWidth={1.7} />
        </button>
      )}
      <div className="titlebar-actions">
        <button
          type="button"
          className="titlebar-btn"
          onMouseDown={(e) => e.preventDefault()}
          onClick={minimize}
          aria-label={t('最小化')}
        >
          <Icon name="minimize" size={14} strokeWidth={1.7} />
        </button>
        <button
          type="button"
          className="titlebar-btn"
          onMouseDown={(e) => e.preventDefault()}
          onClick={toggleMax}
          aria-label={t('最大化')}
        >
          <Icon name="maximize" size={14} strokeWidth={1.7} />
        </button>
        <button
          type="button"
          className="titlebar-btn titlebar-btn--close"
          onMouseDown={(e) => e.preventDefault()}
          onClick={closeToTray}
          aria-label={t('关闭（缩到系统托盘）')}
        >
          <Icon name="x" size={14} strokeWidth={1.7} />
        </button>
      </div>

      {aboutOpen && (
        <div
          className="about-overlay"
          role="dialog"
          aria-modal="true"
          aria-label={t('关于 Real Agent')}
          onMouseDown={() => setAboutOpen(false)}
        >
          <div className="about-card" onMouseDown={(e) => e.stopPropagation()}>
            <div className="about-head">
              <img src={brandLogo} alt="Real Agent" className="titlebar-logo about-logo brand-mark" />
              <div>
                <div className="about-name">{t('Real Agent 工作台')}</div>
                <div className="about-sub">v0.1.0</div>
              </div>
              <button
                type="button"
                className="about-close"
                onClick={() => setAboutOpen(false)}
                aria-label={t('关闭')}
              >
                <Icon name="x" size={15} strokeWidth={1.7} />
              </button>
            </div>
            <div className="about-desc">
              {t('本地优先的 AI 执行伙伴。你下指令，它想清楚再动手：深度思考过程、每一步工具执行与旁白都透明摊在面板上——没有黑箱，跑完还能回头逐行看它为什么这么做、卡在哪、花了多少钱。')}
            </div>
            <div className="about-grid">
              <div className="about-col">
                <div className="about-k">{t('它擅长')}</div>
                <ul className="about-ul">
                  <li>{t('本地项目操作：改代码、跑命令、查日志')}</li>
                  <li>{t('联网检索与网页读取')}</li>
                  <li>{t('过程全透明：思考 / 工具 / 旁白 / 账单')}</li>
                  <li>{t('取消与失败也不赖账：已耗 token 与金额可见')}</li>
                </ul>
              </div>
              <div className="about-col">
                <div className="about-k">{t('技术构成')}</div>
                <ul className="about-ul">
                  <li>{t('壳：Tauri 2（自绘标题栏 / 托盘常驻）')}</li>
                  <li>{t('界面：React + TypeScript')}</li>
                  <li>{t('编排：Rust 后端（收敛循环 + 工具执行）')}</li>
                  <li>{t('模型：DeepSeek / 智谱 GLM（BigModel）')}</li>
                </ul>
              </div>
            </div>
            <div className="about-foot">
              <span>{t('本地数据优先 · 会话与事件存于本机数据库')}</span>
              <span className="about-copy">© 2026 Real Agent</span>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
