
import { useState } from 'react';
import type { EvolutionCard } from '../types';
import { skillsApi } from '../../../services/domains/skills';
import './evolution-card.css';
import { useT } from '../../../shared/i18n';

export function EvolutionCardView({ card }: { card: EvolutionCard }) {
  const t = useT();
  return (
    <div className="evo-card">
      <div className="evo-head">
        <span className="evo-head-title">{t('技能沉淀 · {n} 条', { n: card.items.length })}</span>
        <span className="evo-badge">{t('预览')}</span>
      </div>
      <ul className="evo-list">
        {card.items.map((it, i) => (
          <EvolutionItem key={it.name + '-' + i} item={it} />
        ))}
      </ul>
      <p className="evo-foot">{t('每条独立写入；未点写入前不进技能库（防模型自评注水）')}</p>
    </div>
  );
}

function EvolutionItem({ item }: { item: NonNullable<EvolutionCard['items']>[number] }) {
  const t = useT();
  const [state, setState] = useState<'idle' | 'saving' | 'done' | 'error'>('idle');
  const [errMsg, setErrMsg] = useState('');

  const absorb = async () => {
    setState('saving');
    try {
      await skillsApi.absorb({
        name: item.name,
        summary: item.summary,
        evidence: item.evidence,
        category: item.category || undefined,
        merge_into: item.merge_into || undefined,
      });
      setState('done');
    } catch (e) {
      setErrMsg((e as Error).message);
      setState('error');
    }
  };

  return (
    <li className="evo-item">
      <div className="evo-title-row">
        <span className={'evo-type' + (item.type === 'skill' ? ' is-skill' : ' is-memory')}>
          {item.type === 'skill' ? t('技能') : t('记忆')}
        </span>
        <span className="evo-name">{item.name}</span>
      </div>
      <div className="evo-cat-row">
        {item.category ? <span className="evo-cat">{t(item.category)}</span> : <span className="evo-cat evo-cat-none">{t('未分类')}</span>}
        {item.merge_into ? (
          <span className="evo-merge">{t('写入后并入「{name}」技能', { name: item.merge_into })}</span>
        ) : (
          <span className="evo-merge">{t('写入后新建独立技能')}</span>
        )}
      </div>
      <p className="evo-sum">{item.summary}</p>
      <p className="evo-evidence">{t('证据：{v}', { v: item.evidence })}</p>
      <div className="evo-actions">
        {state === 'done' ? (
          <span className="evo-done">
            {item.merge_into ? t('已并入技能 ✓') : t('已写入技能库 ✓')}
          </span>
        ) : (
          <>
            <button className="evo-absorb-btn" onClick={() => void absorb()} disabled={state === 'saving'}>
              {state === 'saving' ? t('写入中…') : t('写入技能库')}
            </button>
            {state === 'error' && <span className="evo-err">{errMsg}</span>}
          </>
        )}
      </div>
    </li>
  );
}
