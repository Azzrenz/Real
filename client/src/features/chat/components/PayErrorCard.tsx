
import { useState } from 'react';
import { Icon } from '../../../shared/ui/icons';
import { openExternal } from '../../../shared/lib/openExternal';
import './pay-error.css';
import { useT } from '../../../shared/i18n';

export function isBalanceError(text: string): boolean {
  return /402|insufficient|balance|quota|余额|充值|余额不足|资源包/i.test(text);
}

const TOPUP_LINKS = [
  { key: 'deepseek', label: 'DeepSeek 开放平台', url: 'https://platform.deepseek.com/top_up' },
  { key: 'zhipu', label: '智谱 GLM 开放平台', url: 'https://open.bigmodel.cn/console/overview' },
] as const;

type ProviderHint = 'deepseek' | 'zhipu' | null;

function providerHint(message: string): ProviderHint {
  const t = message.toLowerCase();
  if (/glm|bigmodel|zhipu|智谱/.test(t)) return 'zhipu';
  if (/deepseek/.test(t)) return 'deepseek';
  return null;
}

export function PayErrorCard({ message }: { message: string }) {
  const t = useT();
  const [open, setOpen] = useState(false);
  const hint = providerHint(message);
  const ordered = hint
    ? [...TOPUP_LINKS].sort((a, b) => (a.key === hint ? -1 : b.key === hint ? 1 : 0))
    : [...TOPUP_LINKS];

  return (
    <div className="pay-error-card" role="alert">
      <div className="pay-error-head">
        <Icon name="wallet" size={16} className="pay-error-icon" />
        <span className="pay-error-title">{t('账户余额不足')}</span>
      </div>
      <div className="pay-error-body">
        {t('当前模型 API 余额用完了，任务先停在这里（已完成的进度不会丢）。')}
        {t('充完值回来，把刚才那句话重新发一遍就能接着跑——不用重建任务。')}
      </div>
      <div className="pay-error-actions">
        {ordered.map((l) => (
          <button
            key={l.key}
            className={`pay-topup-btn${hint === l.key ? ' primary' : ''}`}
            onClick={() => void openExternal(l.url)}
            aria-label={t('打开{name}充值页面', { name: t(l.label) })}
          >
            <span className="pay-topup-label">
              {t(l.label)}
              {hint === l.key && <span className="pay-topup-tag">{t('检测到该家欠费')}</span>}
            </span>
            <Icon name="external" size={13} className="pay-topup-arrow" />
          </button>
        ))}
      </div>
      <button className="pay-error-toggle" onClick={() => setOpen(!open)}>
        {open ? t('收起错误详情') : t('查看错误详情')}
      </button>
      {open && <pre className="pay-error-raw">{message}</pre>}
    </div>
  );
}
