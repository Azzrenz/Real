// ConfirmModal — danger-operation confirmation / workspace picker (backend ConfirmGate).

import { useEffect, useRef, useState } from 'react';
import { useChatStore } from '../features/chat/store/chatStore';
import { useSettings } from '../features/settings/store/settingsStore';
import { Icon } from '../shared/ui/icons';
import { useT } from '../shared/i18n';
import {
  getTrustRule,
  setTrustRule,
  ACTION_DESC,
  actionLabel,
  type TrustScope,
} from '../features/chat/lib/confirmTrust';

export function ConfirmModal() {
  const t = useT();
  const confirm = useChatStore((s) => s.confirm);
  const respondConfirm = useChatStore((s) => s.respondConfirm);
  const panelRef = useRef<HTMLDivElement>(null);
  const [wsPath, setWsPath] = useState('');
  const [choice, setChoice] = useState('');
  const [note, setNote] = useState('');
  const [rememberSession, setRememberSession] = useState(false);
  const [rememberGlobal, setRememberGlobal] = useState(false);
  const answeredRef = useRef<string | null>(null);

  useEffect(() => {
    setWsPath('');
    setNote('');
    setRememberSession(false);
    setRememberGlobal(false);
    // Decision gate: preselect the model's own pick -- its stance is already on the
    // table, so the user only confirms or overrides. Starting empty would make the
    // user re-derive what the model already decided.
    const rec = confirm?.options?.find((o) => o.recommended);
    setChoice(rec ? rec.label : '');
  }, [confirm?.requestId]);

  const isWorkspacePick = confirm?.action === 'select_workspace';
  const askOptions = confirm?.options ?? [];
  const isAsk = confirm?.action === 'ask' && askOptions.length > 0;

  useEffect(() => {
    if (!confirm || isWorkspacePick || isAsk) return;
    if (answeredRef.current === confirm.requestId) return;
    const autoApprove = useSettings.getState().autoApproveDanger;
    const rule = getTrustRule(confirm.sessionId, confirm.action);
    if (!autoApprove && !rule) return;
    answeredRef.current = confirm.requestId;
    useChatStore.getState().respondConfirm(rule ? rule === 'allow' : true, false);
  }, [confirm, isWorkspacePick, isAsk]);

  const deny = () => {
    if (!confirm) return;
    if (rememberSession || rememberGlobal) {
      setTrustRule(confirm.sessionId, confirm.action, 'deny', rememberGlobal ? 'global' : 'session');
    }
    void respondConfirm(false, false);
  };
  const allowOnce = () => {
    if (!confirm) return;
    if (rememberSession || rememberGlobal) {
      const scope: TrustScope = rememberGlobal ? 'global' : 'session';
      setTrustRule(confirm.sessionId, confirm.action, 'allow', scope);
    }
    // 把"记住"标志一并回传后端（曾经写死 false，后端收到的永远是"不记住"）。
    void respondConfirm(true, rememberSession || rememberGlobal);
  };
  const pickWorkspace = () => {
    if (!confirm) return;
    void respondConfirm(true, false, wsPath.trim() || undefined);
  };
/** Decision gate: hand the answer back. The backend returns it to the model as that */
  const submitChoice = () => {
    if (!confirm) return;
    const picked = choice.trim();
    const typed = note.trim();
    if (!picked && !typed) return;
    void respondConfirm(true, false, picked || undefined, typed || undefined);
  };
  const cancelAsk = () => {
    void respondConfirm(false, false);
  };

  useEffect(() => {
    if (!confirm) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.stopPropagation();
        if (isAsk) cancelAsk();
        else if (isWorkspacePick) void respondConfirm(false, false);
        else deny();
      }
      if (e.key === 'Enter' && isAsk) {
        // Enter inside the note box is a newline; Ctrl/Cmd+Enter submits from there.
        const inNote = (e.target as HTMLElement | null)?.tagName === 'TEXTAREA';
        if (inNote && !(e.ctrlKey || e.metaKey)) return;
        if (!choice && !note.trim()) return;
        e.stopPropagation();
        submitChoice();
      }
      if (e.key === 'Enter' && confirm.action === 'select_workspace') {
        e.stopPropagation();
        pickWorkspace();
      }
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  }, [confirm, rememberSession, rememberGlobal, isWorkspacePick, isAsk, choice, note]);

  if (!confirm) return null;

  const risk = confirm.riskLabel === 'warn' ? 'warn' : confirm.riskLabel === 'info' ? 'info' : 'danger';
  const actionText = t(actionLabel(confirm.action));
  const rawDesc = ACTION_DESC[confirm.action];
  const actionDesc = rawDesc ? t(rawDesc) : undefined;
  const RISK_TXT = { danger: t('高风险'), warn: t('需确认'), info: t('提示') };
  const RISK_ICON = { danger: 'x' as const, warn: 'wrench' as const, info: 'target' as const };

  const hintText =
    confirm.impact || actionDesc || t('Agent 正在等待你的决定，未响应时它不会继续。');

  return (
    <div className="confirm-overlay" onClick={isWorkspacePick || isAsk ? undefined : deny}>
      <div
        ref={panelRef}
        className={`confirm-modal risk-${risk}${isAsk ? ' is-ask' : ''}`}
        role="alertdialog"
        aria-modal="true"
        aria-label={isAsk ? t('需要你定一下') : isWorkspacePick ? t('选择工作区') : t('危险操作确认')}
        onClick={(e) => e.stopPropagation()}
      >
        <div className={`confirm-banner risk-${risk}`}>
          <span className={`confirm-badge confirm-badge--${risk}`}>
            <Icon name={isAsk ? 'target' : RISK_ICON[risk]} size={13} />
            {isAsk ? t('决策') : isWorkspacePick ? t('工作区') : RISK_TXT[risk]}
          </span>
          <span className="confirm-action">
            {isAsk ? confirm.target : isWorkspacePick ? t('选择一个工作区继续') : actionText}
          </span>
        </div>

        <div className="confirm-body">
          {/* The question already owns the banner; a mono target box would just repeat it. */}
          {!isAsk && (
            <div className="confirm-target-wrap">
              <div className="confirm-target-label">{isWorkspacePick ? t('当前无效路径') : t('对象')}</div>
              <div className="confirm-target" title={confirm.target}>
                {confirm.target}
              </div>
            </div>
          )}
          {!isAsk && hintText && <div className="confirm-impact">{hintText}</div>}

          {isAsk ? (
            <div className="ask-gate">
              {/* Top row states only "which one"; the consequences live in the pane
                  below. Repeating the detail next to every label is what makes a list
                  of options unreadable once there are four of them. */}
              <div className="ask-tabs" role="tablist" aria-label={t('候选项')}>
                {askOptions.map((o, i) => (
                  <button
                    key={o.label}
                    type="button"
                    role="tab"
                    aria-selected={choice === o.label}
                    className={`ask-tab${choice === o.label ? ' is-on' : ''}`}
                    onClick={() => setChoice(choice === o.label ? '' : o.label)}
                  >
                    <span className="ask-tab-idx">{i + 1}</span>
                    <span className="ask-tab-label">{o.label}</span>
                    {o.recommended ? <span className="ask-tab-rec">{t('推荐')}</span> : null}
                  </button>
                ))}
              </div>

              {/* Bottom pane carries the picked option's cost -- or, when nothing is
                  picked yet, a line saying the box below is an answer too. Never blank. */}
              <div className="ask-pane">
                {choice ? (
                  <p className="ask-pane-detail">
                    {askOptions.find((o) => o.label === choice)?.detail}
                  </p>
                ) : (
                  <p className="ask-pane-detail ask-pane-idle">
                    {t('点一个选项看它的代价，或者在下面直接写你想怎么做。')}
                  </p>
                )}
              </div>

              <div className="ask-note-wrap">
                <label className="ask-note-label" htmlFor="ask-note">
                  {t('我想怎么做')}
                </label>
                <textarea
                  id="ask-note"
                  className="ask-note"
                  value={note}
                  onChange={(e) => setNote(e.target.value)}
                  placeholder={t('不选也行。写清你的想法，它就按你写的走。')}
                  rows={2}
                  spellCheck={false}
                />
              </div>
            </div>
          ) : isWorkspacePick ? (
            <div className="ws-picker">
              {confirm.workspaceCandidates && confirm.workspaceCandidates.length > 0 && (
                <div className="ws-picker-candidates">
                  {(confirm.workspaceCandidates as string[]).map((c) => (
                    <button
                      key={c}
                      type="button"
                      className="ws-pick-chip"
                      onClick={() => setWsPath(c)}
                      title={c}
                    >
                      {c}
                    </button>
                  ))}
                </div>
              )}
              <input
                className="ws-pick-input"
                value={wsPath}
                onChange={(e) => setWsPath(e.target.value)}
                placeholder={t('或直接输入工作区绝对路径，如 D:\\projects\\my-app')}
                spellCheck={false}
                autoFocus
              />
            </div>
          ) : (
            <div className="confirm-trust">
              <label className={`trust-opt${rememberSession ? ' checked' : ''}`}>
                <input
                  type="checkbox"
                  checked={rememberSession}
                  onChange={(e) => setRememberSession(e.target.checked)}
                />
                <span className="trust-opt-text">
                  {t('本会话内')}<b>{t('同类操作不再询问')}</b>
                  <span className="trust-opt-sub">{t('关闭会话即失效，最安全')}</span>
                </span>
              </label>
              <label className={`trust-opt${rememberGlobal ? ' checked' : ''}`}>
                <input
                  type="checkbox"
                  checked={rememberGlobal}
                  onChange={(e) => setRememberGlobal(e.target.checked)}
                />
                <span className="trust-opt-text">
                  {t('之后')}<b>{t('所有会话同类操作都不再询问')}</b>
                  <span className="trust-opt-sub">{t('影响全部任务，可在设置里清除')}</span>
                </span>
              </label>
            </div>
          )}

          <div className="confirm-hint">
            {isAsk
              ? t('会一直等你——不做选择，任务就停在这一步。')
              : t('会一直等你——不答复，任务就停在这一步。')}
          </div>
        </div>

        {isAsk ? (
          <div className="confirm-actions">
            <button className="btn" onClick={cancelAsk}>
              {t('先不做（Esc）')}
            </button>
            <button
              className="btn primary"
              onClick={submitChoice}
              disabled={!choice && !note.trim()}
              title={choice || note.trim() ? '' : t('先选一项，或写下你想怎么做')}
            >
              {t('就按这个走')}
            </button>
          </div>
        ) : isWorkspacePick ? (
          <div className="confirm-actions">
            <button className="btn sm" onClick={() => void respondConfirm(false, false)}>
              {t('拒绝（Esc）')}
            </button>
            <button
              className="btn primary sm"
              onClick={pickWorkspace}
              disabled={!wsPath.trim()}
              title={wsPath.trim() ? t('写入会话工作区并用它重跑命令') : t('先选择或输入一个路径')}
            >
              {t('以此路径继续')}
            </button>
          </div>
        ) : (
          <div className="confirm-actions">
            <button className="btn sm" onClick={deny} autoFocus>
              {t('拒绝')}
            </button>
            <button
              className={`btn primary sm${risk === 'danger' ? ' danger-allow' : ''}`}
              onClick={allowOnce}
            >
              {rememberSession || rememberGlobal ? t('允许（记住）') : t('允许本次')}
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
