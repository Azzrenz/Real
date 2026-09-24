
import { useState } from 'react';
import { Icon } from './icons';
import { COPIED_RESET_MS, copyText } from '../lib/clipboard';
import { useT } from '../../shared/i18n';

interface Props {
  text: string;
  className?: string;
  size?: number;
  title?: string;
  label?: string;
}

export function CopyButton({ text, className = '', size = 13, title, label }: Props) {
  const t = useT();
  const [copied, setCopied] = useState(false);
  // The default lives here rather than in the parameter list: a default value is evaluated outside
  // the component, so it could never follow a language switch.
  const titleText = title ?? t('复制');
  return (
    <button
      className={`copy-btn${copied ? ' copied' : ''}${className ? ` ${className}` : ''}`}
      onClick={() => {
        void copyText(text).then((ok) => {
          if (!ok) return;
          setCopied(true);
          window.setTimeout(() => setCopied(false), COPIED_RESET_MS);
        });
      }}
      title={copied ? t('已复制') : titleText}
      aria-label={label ?? titleText}
    >
      <Icon name={copied ? 'check' : 'copy'} size={size} />
    </button>
  );
}
