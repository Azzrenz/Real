
import { useCallback, useEffect, useState } from 'react';
import { skillsApi, type SkillEntry } from '../services/domains/skills';
import { useToastStore } from '../shared/store/toastStore';
import { t } from '../shared/i18n';

/* The category string is data -- it is stored and sent to the backend -- so the list stays in
   Chinese and only the option label is translated. */
const SKILL_CATEGORIES = ['架构', 'UI', '工程', '排障', '应用'];

export function SkillDialog({ onClose, onManage }: { onClose: () => void; onManage?: () => void }) {
  const [skills, setSkills] = useState<SkillEntry[]>([]);
  const [tab, setTab] = useState<'list' | 'add'>('list');
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [category, setCategory] = useState('架构');
  const [prompt, setPrompt] = useState('');
  const [busy, setBusy] = useState(false);

  const reload = useCallback(() => {
    void skillsApi.list().then((r) => setSkills(r.skills));
  }, []);
  useEffect(reload, [reload]);

  const add = async () => {
    if (!name.trim() || !prompt.trim()) {
      useToastStore.getState().show(t('技能名和正文必填'), 'error');
      return;
    }
    setBusy(true);
    try {
      await skillsApi.create(name.trim(), description.trim(), prompt, category);
      useToastStore.getState().show(t('技能「{name}」已创建，聊天里输入 /{name} 调用', { name: name.trim() }), 'ok');
      setName(''); setDescription(''); setPrompt('');
      setTab('list');
      reload();
      onManage?.();
    } catch (e) {
      useToastStore.getState().show((e as Error).message, 'error');
    } finally {
      setBusy(false);
    }
  };

  const del = async (n: string) => {
    try {
      await skillsApi.remove(n);
      reload();
      onManage?.();
    } catch (e) {
      useToastStore.getState().show((e as Error).message, 'error');
    }
  };

  return (
    <div className="confirm-overlay" onClick={onClose}>
      <div
        className="confirm-modal"
        role="dialog"
        aria-modal="true"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="confirm-head">
          <span className="confirm-badge confirm-badge--info">{t('技能')}</span>
          <span className="confirm-action">
            {tab === 'list' ? t('已安装技能') : t('添加技能')}
          </span>
        </div>

        <div className="confirm-body" style={{ minWidth: 380 }}>
          {tab === 'list' ? (
            <>
              {skills.length === 0 && (
                <p style={{ margin: '8px 0', opacity: 0.7 }}>
                  {t('还没有技能。技能 = 一份工作流话术（skills/<名>/SKILL.md），')}
                  {t('聊天输入 /技能名 即按它执行。')}
                </p>
              )}
              {skills.map((s) => (
                <div
                  key={s.name}
                  style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '8px 0', borderBottom: '1px solid var(--border, #333)' }}
                >
                  <div style={{ flex: 1, minWidth: 0 }}>
                    <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                      {s.category ? <span className="skill-pop-cat">{s.category}</span> : null}
                      <span style={{ fontWeight: 600 }}>{s.name}</span>
                    </div>
                    <div
                      style={{
                        fontSize: 'var(--fs-sm)', lineHeight: 1.65, marginTop: 3,
                        color: 'color-mix(in srgb, var(--muted, #999) 82%, transparent)',
                      }}
                    >
                      {s.description || t('（无说明）')}
                    </div>
                  </div>
                  <button className="btn ghost sm" onClick={() => void del(s.name)} title={t('删除技能')}>
                    {t('删除')}
                  </button>
                </div>
              ))}
            </>
          ) : (
            <>
              <div style={{ marginBottom: 8 }}>
                <label style={{ fontSize: 'var(--fs-sm)', opacity: 0.8 }}>{t('技能名（字母/数字/中文/-/_）')}</label>
                <input
                  className="input"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  placeholder={t('如：重启排障')}
                  style={{ width: '100%' }}
                />
              </div>
              <div style={{ marginBottom: 8 }}>
                <label style={{ fontSize: 'var(--fs-sm)', opacity: 0.8 }}>{t('分类（它属于哪一块，列表里显示在标题前）')}</label>
                <select
                  className="input"
                  value={category}
                  onChange={(e) => setCategory(e.target.value)}
                  style={{ width: '100%' }}
                >
                  {SKILL_CATEGORIES.map((c) => (
                    <option key={c} value={c}>
                      {t(c)}
                    </option>
                  ))}
                  <option value="">{t('不分类')}</option>
                </select>
              </div>
              <div style={{ marginBottom: 8 }}>
                <label style={{ fontSize: 'var(--fs-sm)', opacity: 0.8 }}>{t('一句话说明（什么时候用它）')}</label>
                <input
                  className="input"
                  value={description}
                  onChange={(e) => setDescription(e.target.value)}
                  placeholder={t('如：引擎/hub 起不来时的排查步骤')}
                  style={{ width: '100%' }}
                />
              </div>
              <div>
                <label style={{ fontSize: 'var(--fs-sm)', opacity: 0.8 }}>{t('工作流正文（确定性步骤，模型照此执行）')}</label>
                <textarea
                  className="input"
                  value={prompt}
                  onChange={(e) => setPrompt(e.target.value)}
                  rows={8}
                  placeholder={t('1. 先看 hub.log 尾部 50 行…\n2. 探测 9560 /api/health…\n判完成：…')}
                  style={{ width: '100%' }}
                />
              </div>
            </>
          )}
        </div>

        <div className="confirm-actions" style={{ display: 'flex', gap: 8, justifyContent: 'flex-end', marginTop: 12 }}>
          {tab === 'list' ? (
            <>
              <button className="btn ghost sm" onClick={onClose}>{t('关闭')}</button>
              <button className="btn primary sm" onClick={() => setTab('add')}>{t('＋ 添加技能')}</button>
            </>
          ) : (
            <>
              <button className="btn ghost sm" onClick={() => setTab('list')}>{t('返回列表')}</button>
              <button className="btn primary sm" disabled={busy} onClick={() => void add()}>
                {busy ? t('保存中…') : t('保存技能')}
              </button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
