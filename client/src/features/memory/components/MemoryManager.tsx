
import { useEffect, useState } from 'react';
import { memoryApi } from '../../../services/domains/memory';
import type { MemoryRow, MemoryStats, Persona, PreferenceRow } from '../../../services/contracts';
import { Icon } from '../../../shared/ui/icons';
import { useT } from '../../../shared/i18n';

const EMPTY_PERSONA: Persona = { identity: '', needs: '', principles: '' };

export function MemoryManager() {
  const t = useT();
  const [persona, setPersona] = useState<Persona>(EMPTY_PERSONA);
  const [themes, setThemes] = useState<MemoryRow[]>([]);
  const [prefs, setPrefs] = useState<PreferenceRow[]>([]);
  const [stats, setStats] = useState<MemoryStats | null>(null);
  const [newPref, setNewPref] = useState('');
  const [msg, setMsg] = useState('');
  const [loaded, setLoaded] = useState(false);

  const flash = (msg: string) => {
    setMsg(msg);
    window.setTimeout(() => setMsg(''), 2500);
  };

  const loadThemes = () => {
    void memoryApi.themes
      .list()
      .then(setThemes)
      .catch(() => undefined);
  };
  const loadPrefs = () => {
    void memoryApi.preferences
      .list()
      .then(setPrefs)
      .catch(() => undefined);
  };

  const reload = () => {
    void memoryApi.persona
      .get()
      .then((p) => setPersona({ ...EMPTY_PERSONA, ...p }))
      .catch(() => undefined);
    loadThemes();
    loadPrefs();
    void memoryApi.stats()
      .then(setStats)
      .catch(() => undefined);
  };

  useEffect(() => {
    reload();
    setLoaded(true);
 // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const savePersona = async () => {
    try {
      await memoryApi.persona.put(persona);
    } catch {
      flash(t('画像保存失败'));
    }
  };

  const addPref = async () => {
    const v = newPref.trim();
    if (!v) return;
    try {
      const r = await memoryApi.preferences.add('', v);
      if (!r.ok) {
        flash(r.error ?? t('添加失败'));
        return;
      }
      setNewPref('');
      loadPrefs();
    } catch {
      flash(t('偏好添加失败'));
    }
  };

  const delPref = async (key: string) => {
    try {
      await memoryApi.preferences.remove('', key);
      setPrefs((rows) => rows.filter((r) => r.key !== key));
    } catch {
      flash(t('删除失败'));
    }
  };

  const pinTheme = async (id: string | number) => {
    try {
      await memoryApi.themes.pin('', id);
      loadThemes();
    } catch {
      flash(t('操作失败'));
    }
  };

  const delTheme = async (id: string | number) => {
    try {
      await memoryApi.themes.remove(id);
      setThemes((rows) => rows.filter((r) => r.id !== id));
    } catch {
      flash(t('删除失败'));
    }
  };

  return (
    <>
      <section className="settings-sec">
        <div className="sec-title">{t('用户核心画像（L3 永恒记忆）')}</div>
        <label className="field">
          <span className="field-label">{t('🧭 身份（你是谁）')}</span>
          <input
            className="input"
            value={persona.identity}
            placeholder={t('如：开发者 / 音乐人……')}
            onChange={(e) => setPersona({ ...persona, identity: e.target.value })}
          />
        </label>
        <label className="field">
          <span className="field-label">{t('🎯 最深需求（为什么用 Agent）')}</span>
          <input
            className="input"
            value={persona.needs}
            placeholder={t('如：要能独立完成完整功能交付')}
            onChange={(e) => setPersona({ ...persona, needs: e.target.value })}
          />
        </label>
        <label className="field">
          <span className="field-label">{t('📏 核心原则（永远遵守的标准）')}</span>
          <input
            className="input"
            value={persona.principles}
            placeholder={t('如：改动前先拿测试基线；修复必须带验证')}
            onChange={(e) => setPersona({ ...persona, principles: e.target.value })}
          />
        </label>
        <div className="field-actions">
          <button className="btn ghost sm" onClick={() => void savePersona()}>{t('保存画像')}</button>
        </div>
        <p className="field-note">{t('全局唯一、每轮注入、永不过期——回答会始终贴合这里的内容。')}</p>
      </section>

      <section className="settings-sec">
        <div className="sec-title">
          {t('主题加权')}{stats ? t('（共 {total} 条 · 永久 {perm} · 今日 {today}）', { total: stats.themes_total, perm: stats.themes_permanent, today: stats.freshness.today }) : ''}
        </div>
        {themes.length === 0 && <p className="field-note">{t('还没有主题——多轮深聊同一话题会自动提炼并累积重要度（架构/方向类话题自动标永久）。')}</p>}
        <div className="mem-table">
          {themes.map((th) => (
            <div className="mem-row" key={th.id}>
              <span className="mem-value" title={th.value}>{th.value}</span>
              <span className={`mem-pill${th.permanent ? ' perm' : ''}`}>{th.permanent ? t('永久') : t('重要度 {n}', { n: th.priority })}</span>
              {!th.permanent && (
                <button className="mem-act" title={t('标为永久重要（永不淘汰）')} onClick={() => void pinTheme(th.id)}>
                  <Icon name="target" size={13} />
                </button>
              )}
              <button className="mem-act" title={t('删除')} onClick={() => void delTheme(th.id)}>
                <Icon name="x" size={13} />
              </button>
            </div>
          ))}
        </div>
        <p className="field-note">{t('每轮对话自动提炼总提纲；同一话题反复深聊 → 重要度累积（≥90 定性永恒，永不淘汰）。')}</p>
      </section>

      <section className="settings-sec">
        <div className="sec-title">{t('偏好记忆')}{stats ? t('（{n} 条）', { n: stats.prefs_count }) : ''}</div>
        <div className="mem-table">
          {prefs.map((p) => (
            <div className="mem-row" key={p.key}>
              <span className="mem-value" title={p.value}>{p.value}</span>
              <button className="mem-act" title={t('删除')} onClick={() => void delPref(p.key)}>
                <Icon name="x" size={13} />
              </button>
            </div>
          ))}
        </div>
        <div className="field">
          <div className="field-actions">
            <input
              className="input"
              value={newPref}
              placeholder={t('手动添加偏好，如：回答不要用英文')}
              onChange={(e) => setNewPref(e.target.value)}
              onKeyDown={(e) => { if (e.nativeEvent.isComposing || e.keyCode === 229) return; if (e.key === 'Enter') void addPref(); }}
            />
            <button className="btn ghost sm" onClick={() => void addPref()} disabled={!newPref.trim()}>{t('添加')}</button>
          </div>
        </div>
        <p className="field-note">{t('对话里说"以后要…/不要…"也会自动记进来；跨会话生效，无需重申。')}</p>
      </section>

      {loaded && msg && <div className="save-msg">{msg}</div>}
    </>
  );
}
