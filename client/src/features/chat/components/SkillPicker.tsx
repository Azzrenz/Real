import { useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { invoke } from '@tauri-apps/api/core';
import { skillsApi, type SkillEntry } from '../../../services/domains/skills';
import { placeCard } from './skillCardLayout';
import { useToastStore } from '../../../shared/store/toastStore';
import { t } from '../../../shared/i18n';
import './skill-picker.css';

function summarize(desc: string, max = 56): string {
  let s = (desc || '').trim().replace(/^[|>'"`\s]+/, '');
  if (!s) return t('（无说明）');
  const first = s.split(/[。；;！!？?\n]/)[0].trim();
  s = first || s;
  return s.length > max ? `${s.slice(0, max)}…` : s;
}

function fullDesc(desc: string, max = 240): string {
  let s = (desc || '').trim().replace(/^[|>'"`\s]+/, '');
  if (!s) return t('（无说明）');
  return s.length > max ? `${s.slice(0, max)}…` : s;
}

/** Pinning is a per-machine UI preference, so it lives in localStorage: the skill
 *  list itself is a filesystem scan and has no place to keep a user's ordering. */
const PIN_KEY = 'real.skills.pinned';

function readPinned(): string[] {
  try {
    const v: unknown = JSON.parse(localStorage.getItem(PIN_KEY) || '[]');
    return Array.isArray(v) ? v.map(String) : [];
  } catch {
    return [];
  }
}

function writePinned(list: string[]): void {
  try {
    localStorage.setItem(PIN_KEY, JSON.stringify(list));
  } catch {
    /* Private mode / quota: pinning is best-effort, never breaks the picker. */
  }
}

/** Opens the standalone detail window. Rust builds it (src-tauri/src/lib.rs) and
 *  reuses the existing one when the same skill is already open. */
async function openDetailWindow(name: string) {
  try {
    await invoke('open_skill_window', { name });
  } catch (e) {
    useToastStore.getState().show((e as Error).message || t('打开详情窗口失败'), 'error');
  }
}

function badgeOf(name: string): string {
  const c = (name || '?').trim().charAt(0) || '?';
  return /[a-z]/i.test(c) ? c.toUpperCase() : c;
}

let cache: SkillEntry[] | null = null;
export function preloadSkills() {
  if (cache) return;
  void skillsApi
    .list()
    .then((r) => {
      cache = r.skills;
    })
    .catch(() => {});
}

export function SkillPicker({
  anchorRect,
  onPick,
  onManage,
  onClose,
}: {
  anchorRect: DOMRect;
  onPick: (name: string) => void;
  onManage: () => void;
  onClose: () => void;
}) {
  const [skills, setSkills] = useState<SkillEntry[]>(cache ?? []);
  const [loading, setLoading] = useState(cache === null);
  const [q, setQ] = useState('');
  const [active, setActive] = useState(0);
  const [ctx, setCtx] = useState<{ name: string; x: number; y: number } | null>(null);
  const [pinned, setPinned] = useState(false);
  const [pinnedSkills, setPinnedSkills] = useState<string[]>(() => readPinned());
  const [confirming, setConfirming] = useState<string | null>(null);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [renameVal, setRenameVal] = useState('');
  const [tip, setTip] = useState<{
    name: string;
    cat: string;
    desc: string;
    summary: string;
    outline: string[];
    left: number;
    width: number;
    top: number;
    /** Matches the popover height so the two read as one panel. */
    height: number;
  } | null>(null);
  const tipTimer = useRef<number | null>(null);
  const hideTimer = useRef<number | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const renameRef = useRef<HTMLInputElement>(null);
  const popRef = useRef<HTMLDivElement>(null);

  const reload = () => {
    void skillsApi
      .list()
      .then((r) => {
        cache = r.skills;
        setSkills(r.skills);
      })
      .catch(() => {
        if (!cache) setSkills([]);
      })
      .finally(() => setLoading(false));
  };
  useEffect(reload, []);

  const shown = useMemo(() => {
    const kw = q.trim().toLowerCase();
    const hit = kw
      ? skills.filter(
          (s) =>
            s.name.toLowerCase().includes(kw) ||
            (s.description || '').toLowerCase().includes(kw),
        )
      : skills;
    return [...hit].sort((a, b) => {
      // Pinned skills lead, and keep name order among themselves. Search filtering
      // runs first, so a pinned skill still only shows when it matches the query.
      const pa = pinnedSkills.includes(a.name);
      const pb = pinnedSkills.includes(b.name);
      if (pa !== pb) return pa ? -1 : 1;
      return a.name.localeCompare(b.name, 'zh-Hans-CN');
    });
  }, [skills, q, pinnedSkills]);

  useEffect(() => setActive(0), [q]);

  // Anchored to the popover itself, not to the hovered row: same top and height,
  // flush against one of its sides. Anchoring to the row made the card slide and
  // re-wrap as the pointer moved down the list, which read as layout noise.
  const showTip = (s: SkillEntry) => {
    if (pinned) return;
    if (tipTimer.current) window.clearTimeout(tipTimer.current);
    if (hideTimer.current) window.clearTimeout(hideTimer.current);
    const pop = popRef.current?.getBoundingClientRect();
    if (!pop) return;
    const { left, width } = placeCard(pop, window.innerWidth);
    setTip({
      name: s.name,
      cat: s.category ?? '',
      desc: fullDesc(s.description),
      summary: s.summary ?? '',
      outline: s.outline ?? [],
      left,
      width,
      top: pop.top,
      height: pop.height,
    });
  };
  const hideTip = () => {
    if (pinned) return;
    if (tipTimer.current) window.clearTimeout(tipTimer.current);
    if (hideTimer.current) window.clearTimeout(hideTimer.current);
    hideTimer.current = window.setTimeout(() => setTip(null), 320);
  };
  const keepTip = () => {
    if (hideTimer.current) window.clearTimeout(hideTimer.current);
  };
  useEffect(
    () => () => {
      if (tipTimer.current) window.clearTimeout(tipTimer.current);
      if (hideTimer.current) window.clearTimeout(hideTimer.current);
    },
    [],
  );

  const openedAt = useRef(Date.now());
  useEffect(() => {
    const onDoc = (e: MouseEvent) => {
      const t = e.target as HTMLElement;
      if (ctx && !t.closest('.skill-ctx')) setCtx(null);
      if (confirming && !t.closest('.skill-pop-confirm')) setConfirming(null);
      if (renaming && !t.closest('.skill-pop-rename')) setRenaming(null);
      if (!t.closest('.skill-pop') && !t.closest('.skill-tip') && !t.closest('[data-skill-toggle]')) onClose();
    };
    const onScroll = () => {
      if (Date.now() - openedAt.current > 250) onClose();
    };
    document.addEventListener('mousedown', onDoc);
    window.addEventListener('scroll', onScroll);
    window.addEventListener('resize', onScroll);
    return () => {
      document.removeEventListener('mousedown', onDoc);
      window.removeEventListener('scroll', onScroll);
      window.removeEventListener('resize', onScroll);
    };
  }, [onClose, ctx, confirming, renaming]);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  useEffect(() => {
    if (renaming) renameRef.current?.select();
  }, [renaming]);

  const togglePin = (name: string) => {
    setPinnedSkills((prev) => {
      const next = prev.includes(name) ? prev.filter((n) => n !== name) : [...prev, name];
      writePinned(next);
      return next;
    });
  };

  const doRemove = (s: SkillEntry) => {
    void skillsApi
      .remove(s.name)
      .then(() => {
        cache = null;
        setConfirming(null);
        setTip(null);
        setPinned(false);
        reload();
      })
      .catch((e) => useToastStore.getState().show((e as Error).message, 'error'));
  };

  const doRename = (old: string) => {
    const nn = renameVal.trim();
    if (!nn || nn === old) {
      setRenaming(null);
      return;
    }
    void skillsApi
      .rename(old, nn)
      .then(() => {
        useToastStore.getState().show(t('已重命名为「{name}」', { name: nn }), 'ok');
        cache = null;
        setRenaming(null);
        reload();
      })
      .catch((e) => useToastStore.getState().show((e as Error).message, 'error'));
  };

  const openDir = (name: string) => {
    void skillsApi
      .openDir(name)
      .then((r) => {
        useToastStore.getState().show(t('已打开：{path}', { path: r.path ?? name }), 'ok');
      })
      .catch((e) => useToastStore.getState().show((e as Error).message, 'error'));
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setActive((i) => Math.min(i + 1, shown.length - 1));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      setActive((i) => Math.max(i - 1, 0));
    } else if (e.key === 'Enter') {
      e.preventDefault();
      e.stopPropagation();
      if (renaming) {
        doRename(renaming);
        return;
      }
      if (confirming) return;
      const s = shown[active];
      if (s) {
        onPick(s.name);
        onClose();
      }
    } else if (e.key === 'Escape') {
      e.preventDefault();
      if (renaming) setRenaming(null);
      else if (confirming) setConfirming(null);
      else if (ctx) setCtx(null);
      else if (pinned) {
        setPinned(false);
        setTip(null);
      } else onClose();
    }
  };

  // Must match .skill-pop width (400) plus its 8px gutter in skill-picker.css.
  const left = Math.max(8, Math.min(anchorRect.left, window.innerWidth - 408));

  return (
    <div
      ref={popRef}
      className="skill-pop"
      style={{ position: 'fixed', left, bottom: window.innerHeight - anchorRect.top + 6 }}
      role="menu"
      aria-label={t('技能列表')}
      onKeyDown={onKeyDown}
    >
      <div className="skill-pop-head">
        <svg className="skill-pop-icon" viewBox="0 0 16 16" aria-hidden="true">
          <circle cx="7" cy="7" r="4.5" fill="none" stroke="currentColor" strokeWidth="1.6" />
          <path d="M10.5 10.5 L14 14" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
        </svg>
        <input
          ref={inputRef}
          className="skill-pop-search"
          value={q}
          onChange={(e) => setQ(e.target.value)}
          placeholder={t('搜索技能')}
          aria-label={t('搜索技能')}
        />
      </div>

      <div className="skill-pop-list" role="none">
        {shown.map((s, i) =>
          renaming === s.name ? (
            <div className="skill-pop-confirm skill-pop-rename" key={s.name}>
              <input
                ref={renameRef}
                className="skill-pop-rename-input"
                value={renameVal}
                onChange={(e) => setRenameVal(e.target.value)}
                onKeyDown={(e) => {
                  if (e.nativeEvent.isComposing || e.keyCode === 229) return;
                  if (e.key === 'Enter') {
                    e.preventDefault();
                    e.stopPropagation();
                    doRename(s.name);
                  }
                }}
                aria-label={t('新技能名')}
              />
              <button
                type="button"
                className="skill-pop-btn"
                onClick={(e) => {
                  e.stopPropagation();
                  setRenaming(null);
                }}
              >
                {t('取消')}
              </button>
              <button
                type="button"
                className="skill-pop-btn go"
                onClick={(e) => {
                  e.stopPropagation();
                  doRename(s.name);
                }}
              >
                {t('保存')}
              </button>
            </div>
          ) : confirming === s.name ? (
            <div className="skill-pop-confirm" key={s.name} role="alertdialog" aria-label={t('删除确认')}>
              <span className="skill-pop-confirm-text">{t('删除「{name}」？', { name: s.name })}</span>
              <button
                type="button"
                className="skill-pop-btn"
                onClick={(e) => {
                  e.stopPropagation();
                  setConfirming(null);
                }}
              >
                {t('取消')}
              </button>
              <button
                type="button"
                className="skill-pop-btn danger"
                onClick={(e) => {
                  e.stopPropagation();
                  doRemove(s);
                }}
              >
                {t('删除')}
              </button>
            </div>
          ) : (
            <button
              key={s.name}
              type="button"
              className={`skill-pop-item${i === active ? ' is-active' : ''}${
                pinnedSkills.includes(s.name) ? ' is-pinned' : ''
              }`}
              role="menuitem"
              tabIndex={-1}
              onClick={() => {
                onPick(s.name);
                onClose();
              }}
              onContextMenu={(e) => {
                e.preventDefault();
                hideTip();
                setCtx({ name: s.name, x: e.clientX, y: e.clientY });
              }}
              onMouseEnter={() => showTip(s)}
              onMouseLeave={hideTip}
            >
              <span className="skill-pop-badge" aria-hidden="true">
                {badgeOf(s.name)}
              </span>
              <span className="skill-pop-main">
                <span className="skill-pop-title">
                  {s.category ? <span className="skill-pop-cat">{s.category}</span> : null}
                  <span className="skill-pop-name">{s.name}</span>
                  {pinnedSkills.includes(s.name) ? (
                    <svg className="skill-pop-pin" viewBox="0 0 12 12" aria-hidden="true">
                      <path
                        d="M6 10.4 V2 M2.7 5.3 L6 2 L9.3 5.3"
                        fill="none"
                        stroke="currentColor"
                        strokeWidth="1.5"
                        strokeLinecap="round"
                        strokeLinejoin="round"
                      />
                    </svg>
                  ) : null}
                </span>
                <span className="skill-pop-desc">{summarize(s.description)}</span>
              </span>
              <span
                className="skill-pop-del"
                role="button"
                tabIndex={-1}
                aria-label={t('删除技能 {name}', { name: s.name })}
                title={t('删除（需确认）')}
                onClick={(e) => {
                  e.stopPropagation();
                  setConfirming(s.name);
                }}
              >
                <svg viewBox="0 0 12 12" aria-hidden="true">
                  <path
                    d="M3 3 L9 9 M9 3 L3 9"
                    stroke="currentColor"
                    strokeWidth="1.5"
                    strokeLinecap="round"
                  />
                </svg>
              </span>
            </button>
          ),
        )}

        {shown.length === 0 && (
          <div className="skill-pop-empty">
            {loading
              ? t('加载中…')
              : q.trim()
                ? t('没有匹配「{q}」的技能', { q: q.trim() })
                : t('暂无技能')}
          </div>
        )}
      </div>

      <div className="skill-pop-foot">
        <button type="button" className="skill-pop-manage" onClick={onManage}>
          {t('管理技能…')}
        </button>
        <span className="skill-pop-count">{t('{n} 个技能', { n: skills.length })}</span>
      </div>

      {tip &&
        createPortal(
          <div
            className="skill-tip"
            role="tooltip"
            onMouseEnter={keepTip}
            onMouseLeave={() => {
              if (!pinned) setTip(null);
            }}
            onClick={(e) => {
              e.stopPropagation();
              if (pinned) {
                setPinned(false);
                setTip(null);
              } else {
                setPinned(true);
              }
            }}
            style={{
              position: 'fixed',
              left: tip.left,
              top: tip.top,
              // Narrowed only when neither side of the popover can hold the full card.
              width: tip.width,
              // Same height as the popover: pinned to its box, not sized to content,
              // so the two panels stay flush regardless of which skill is hovered.
              height: tip.height,
              maxHeight: Math.max(160, window.innerHeight - tip.top - 12),
            }}
          >
            <div className="skill-tip-head">
              {tip.cat ? <span className="skill-pop-cat">{tip.cat}</span> : null}
              <span className="skill-tip-name">{tip.name}</span>
            </div>
            <p className="skill-tip-desc">{tip.desc}</p>
            {tip.summary ? <p className="skill-tip-sum">{tip.summary}</p> : null}
            <div className="skill-tip-meta">
              <code className="skill-tip-invoke">/{tip.name}</code>
              {tip.outline.length > 0 && (
                <span className="skill-tip-count">{t('{n} 步', { n: tip.outline.length })}</span>
              )}
            </div>
            {tip.outline.length > 0 && (
              <ul className="skill-tip-list">
                {tip.outline.map((o) => (
                  <li key={o}>{o}</li>
                ))}
              </ul>
            )}
            <div className={`skill-tip-foot${pinned ? ' is-pinned' : ''}`}>
              <span>{pinned ? t('已固定 · 点击取消') : t('点击固定')}</span>
              <button
                type="button"
                className="skill-tip-open"
                onClick={(e) => {
                  e.stopPropagation();
                  void openDetailWindow(tip.name);
                }}
              >
                {t('独立窗口')}
              </button>
            </div>
          </div>,
          document.body,
        )}

      {ctx && (
        <div
          className="skill-ctx"
          role="menu"
          aria-label={t('技能操作')}
          style={{
            position: 'fixed',
            left: Math.max(8, Math.min(ctx.x, window.innerWidth - 176)),
            top: Math.max(8, Math.min(ctx.y, window.innerHeight - 172)),
          }}
        >
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              openDir(ctx.name);
              setCtx(null);
            }}
          >
            {t('打开文件夹')}
          </button>
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              togglePin(ctx.name);
              setCtx(null);
            }}
          >
            {pinnedSkills.includes(ctx.name) ? t('取消置顶') : t('置顶到最前')}
          </button>
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              setRenameVal(ctx.name);
              setRenaming(ctx.name);
              setCtx(null);
            }}
          >
            {t('重命名…')}
          </button>
          <button
            type="button"
            role="menuitem"
            className="danger"
            onClick={() => {
              setConfirming(ctx.name);
              setCtx(null);
            }}
          >
            {t('删除技能…')}
          </button>
        </div>
      )}
    </div>
  );
}
