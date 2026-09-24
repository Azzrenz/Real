import { Fragment, useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useSettings } from '../store/settingsStore';
import { useSessionStore } from '../../sessions/store/sessionStore';
import { Icon } from '../../../shared/ui/icons';
import { prettyModel } from '../../../shared/lib/format';
import { t } from '../../../shared/i18n';

/** Right-edge clamp for the portaled menu; CSS `min-width` is 280. */
const MENU_CLAMP_W = 288;

export function ModelPicker() {
  // Model belongs to the session (migrations/0014): the chip shows this session's own
  // choice, falling back to the global default only when this session never picked one.
  // currentId comes from the sidebar selection, so the chip follows the session for free.
  const currentId = useSessionStore((s) => s.currentId);
  const sessionModel = useSessionStore(
    (s) => s.sessions.find((x) => x.id === s.currentId)?.model ?? null,
  );
  const setSessionModel = useSessionStore((s) => s.setSessionModel);
  const globalModel = useSettings((s) => s.model);
  const model = sessionModel || globalModel;
  const models = useSettings((s) => s.models);
  const modelLabels = useSettings((s) => s.modelLabels);
  const providers = useSettings((s) => s.providers);
  const providerHasKey = useSettings((s) => s.providerHasKey);
  const glmReady = useSettings((s) => s.glmReady);
  const switchModel = useSettings((s) => s.switchModel);

  const [open, setOpen] = useState(false);
  const [anchor, setAnchor] = useState<DOMRect | null>(null);
  const [pendingKey, setPendingKey] = useState<string | null>(null);
  const [keyInput, setKeyInput] = useState('');
  const [err, setErr] = useState<string | null>(null);
  const [switching, setSwitching] = useState(false);
  const chipRef = useRef<HTMLButtonElement>(null);

  const close = () => {
    setOpen(false);
    setPendingKey(null);
    setKeyInput('');
    setErr(null);
  };

  useEffect(() => {
    if (!open) return;
    // Menu is portaled to body so the input box's overflow:hidden cannot clip it.
    // That also means menu and chip no longer share a DOM subtree, so outside-click
    // is matched via closest() on both rather than container containment.
    const onDoc = (e: MouseEvent) => {
      const t = e.target as HTMLElement | null;
      if (t?.closest('.model-menu') || t?.closest('.model-chip')) return;
      close();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') close();
    };
    const onResize = () => {
      const r = chipRef.current?.getBoundingClientRect();
      if (r) setAnchor(r);
    };
    document.addEventListener('mousedown', onDoc);
    document.addEventListener('keydown', onKey);
    window.addEventListener('resize', onResize);
    return () => {
      document.removeEventListener('mousedown', onDoc);
      document.removeEventListener('keydown', onKey);
      window.removeEventListener('resize', onResize);
    };
  }, [open]);

  const all = models.length ? models : [model];

  // Groups come from the backend provider archive (providers/*.json), so adding a
  // vendor needs no change here. Empty archive (backend down) degrades to one group.
  const groups: Array<{ key: string; name: string; ids: string[] }> = (() => {
    if (!providers.length) return [{ key: '__all__', name: '', ids: all }];
    const known = new Set(providers.flatMap((p) => p.models.map((m) => m.id)));
    const gs = providers
      .map((p) => ({ key: p.id, name: p.name || p.id, ids: p.models.map((m) => m.id) }))
      .filter((g) => g.ids.length > 0);
    const rest = all.filter((m) => !known.has(m));
    if (rest.length) gs.push({ key: '__other__', name: t('其他'), ids: rest });
    return gs;
  })();

  // Provider that owns this model, per the archive; '' when unknown.
  const providerIdOf = (id: string): string =>
    providers.find((p) => p.models.some((m) => m.id === id))?.id ?? '';

  // Colour cue only - never a routing decision. New vendors share the brand colour.
  const dotClass = (id: string) => (providerIdOf(id) === 'glm' ? 'glm' : 'ds');

  // Does this model's provider still need a key? Without an archive, fall back to glmReady.
  const needsKey = (id: string): boolean => {
    const pid = providerIdOf(id);
    if (!pid) return id.startsWith('glm') && !glmReady;
    return !providerHasKey[pid];
  };

  const pendingProvider = providers.find((p) => p.models.some((m) => m.id === pendingKey));

  async function doSwitch(m: string, key?: string) {
    setErr(null);
    setSwitching(true);
    try {
      // Send the session id: the server writes only this session's row and leaves the
      // global default alone (that default is what never-picked sessions fall back to).
      await switchModel(m, key, currentId ?? undefined);
      if (currentId) setSessionModel(currentId, m);
      close();
    } catch (e) {
      setErr((e as Error).message || t('切换失败'));
    } finally {
      setSwitching(false);
    }
  }

  function pick(m: string) {
    if (m === model) {
      close();
      return;
    }
    if (needsKey(m)) {
      setPendingKey(m);
      setErr(null);
      return;
    }
    void doSwitch(m);
  }

  const renderItem = (m: string) => (
    <button
      key={m}
      className={`model-item${m === model ? ' active' : ''}`}
      onClick={() => pick(m)}
      disabled={switching}
    >
      <span className={`model-dot ${dotClass(m)}`} />
      <span className="model-item-name">{prettyModel(m, modelLabels)}</span>
      {m === model && <Icon name="check" size={13} className="model-item-check" />}
    </button>
  );

  return (
    <div className="model-picker">
      <button
        ref={chipRef}
        className="input-tool-chip model-chip"
        onClick={() => {
          if (open) {
            close();
            return;
          }
          const r = chipRef.current?.getBoundingClientRect();
          if (!r) return;
          setAnchor(r);
          setOpen(true);
        }}
        title={t('切换模型')}
        disabled={switching}
      >
        <span className={`model-dot ${dotClass(model)}`} />
        <span className="chip-model-name">{prettyModel(model, modelLabels)}</span>
        <Icon name="chevron" size={13} className="model-chip-caret" />
      </button>

      {open &&
        anchor &&
        createPortal(
          <div
            className="model-menu"
            style={{
              left: Math.max(8, Math.min(anchor.left, window.innerWidth - MENU_CLAMP_W)),
              bottom: window.innerHeight - anchor.top + 8,
            }}
          >
            {groups.map((g) => (
              <Fragment key={g.key}>
                {g.name && <div className="model-menu-group">{g.name}</div>}
                {g.ids.map(renderItem)}
              </Fragment>
            ))}

            {pendingKey && (
              <div className="model-key-row">
                <div className="model-key-hint">
                  {t('首次使用 {p}：粘贴一次该厂商的 API Key（{url} 获取，仅存本机后端，之后一键互切）', {
                    p: pendingProvider?.name || prettyModel(pendingKey, modelLabels),
                    url: pendingProvider?.base_url || t('厂商控制台'),
                  })}
                </div>
                <input
                  autoFocus
                  type="password"
                  value={keyInput}
                  placeholder={t('粘贴 API Key')}
                  onChange={(e) => setKeyInput(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.nativeEvent.isComposing || e.keyCode === 229) return;
                    if (e.key === 'Enter' && keyInput.trim()) void doSwitch(pendingKey, keyInput);
                  }}
                />
                <div className="model-key-actions">
                  <button className="btn primary sm" disabled={!keyInput.trim() || switching} onClick={() => void doSwitch(pendingKey, keyInput)}>
                    {t('确认并切换')}
                  </button>
                  <button className="btn sm" onClick={() => setPendingKey(null)} disabled={switching}>
                    {t('取消')}
                  </button>
                </div>
              </div>
            )}
            {err && <div className="model-menu-err">{err}</div>}
          </div>,
          document.body,
        )}
    </div>
  );
}
