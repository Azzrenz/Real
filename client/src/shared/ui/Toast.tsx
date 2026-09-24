// Toast stack, bottom-right. Auto-dismiss handled in toastStore (4s).

import { useToastStore } from '../store/toastStore';
import { Icon } from './icons';
import { t } from '../../shared/i18n';

const ICON: Record<string, 'x' | 'check' | 'flame'> = { error: 'x', ok: 'check', info: 'flame' };

export function Toast() {
  const toasts = useToastStore((s) => s.toasts);
  const dismiss = useToastStore((s) => s.dismiss);
  if (toasts.length === 0) return null;
  return (
    <div className="toast-stack">
      {toasts.map((item) => (
        <div key={item.id} className={`toast toast--${item.kind}`} role={item.kind === 'error' ? 'alert' : 'status'}>
          <Icon name={ICON[item.kind]} size={14} className="toast-icon" />
          <span className="toast-text">{item.text}</span>
          <button className="toast-close" onClick={() => dismiss(item.id)} aria-label={t('关闭提示')}>
            <Icon name="x" size={12} />
          </button>
        </div>
      ))}
    </div>
  );
}
