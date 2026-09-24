
import { useT } from '../../../shared/i18n';
import type { Round } from '../store/chatStore';
import type { LLMCall } from '../../../services/contracts';

export function CostLine({ totals, usage }: { totals: Round['totals']; usage?: LLMCall[] }) {
  // Subscribed rather than module-level: this line is rendered inside a memo()'d round, so a
  // language switch would otherwise leave the old wording in place.
  const t = useT();
  // token 汇总（2026-09-21）：输入 / 输出 / 未命中，按千（k）显示，供悬停详情展开。
  const tokens = (() => {
    const rows = usage ?? [];
    let input = 0, output = 0, miss = 0, cached = 0, costOut = 0;
    for (const c of rows) {
      input += c.input ?? 0;
      output += c.output ?? 0;
      cached += c.cached ?? 0;
      miss += Math.max(0, (c.input ?? 0) - (c.cached ?? 0));
      costOut += c.cost_output ?? 0;
    }
    return { input, output, miss, cached, costOut };
  })();
  const ktok = (n: number) => (n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n));
  return (
    <div className="cost-lines">
      <div className="cost-brief">
        {t('API {n} 次', { n: totals.llm_calls ?? 0 })}
        {typeof totals.cost_yuan === 'number' && (
          <span className="cost-price">¥{String(totals.cost_yuan)}</span>
        )}
      </div>
      <div className="cost-full">
        <div className="cost-line">
          {t('API {n} 次', { n: totals.llm_calls ?? 0 })}
          {typeof totals.cost_yuan === 'number' && (
            <span className="cost-price">
              {' · '}
              {t('共消耗 ¥{n}', { n: String(totals.cost_yuan) })}
            </span>
          )}
          <span className="cost-tokens">
            {' · '}
            {t('输入 {i} · 输出 {o} · 输出花费 ¥{oc} · 未命中 {mi}', {
              i: ktok(tokens.input),
              o: ktok(tokens.output),
              oc: tokens.costOut.toFixed(3),
              mi: ktok(tokens.miss),
            })}
          </span>
          {tokens.input > 0 && (
            <span className="cost-cache">
              {' · '}
              {t('缓存命中 {n}%', { n: ((tokens.cached / tokens.input) * 100).toFixed(1) })}
            </span>
          )}
        </div>
      </div>
    </div>
  );
}
