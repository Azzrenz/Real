
import { useEffect, useState } from 'react';
import { Icon } from '../../shared/ui/icons';
import { useStickScroll } from '../../shared/hooks/useStickScroll';
import { useT } from '../../shared/i18n';

interface Props {
  lang: string;
  code: string;
  streaming?: boolean;
}

export function CodeWindow({ lang, code, streaming }: Props) {
  const t = useT();
  const [open, setOpen] = useState(true);
  const [copied, setCopied] = useState(false);
  const { ref, onScroll, stickToBottom } = useStickScroll<HTMLPreElement>();
  const lines = code ? code.split('\n').length : 0;

  useEffect(() => {
    if (streaming) stickToBottom();
  }, [code, streaming, stickToBottom]);

  const copy = () => {
    void navigator.clipboard.writeText(code).then(() => {
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1200);
    });
  };

  return (
    <div className={`dt-code${open ? ' is-open' : ''}`}>
      <div className="dt-code-head">
        <button
          className="dt-code-toggle"
          onClick={() => setOpen((v) => !v)}
          aria-expanded={open}
          title={open ? t('收起代码块') : t('展开代码块')}
        >
          <Icon name="chevron" size={12} className={`dt-code-caret${open ? ' open' : ''}`} />
          <span className="dt-code-lang">{lang || 'code'}</span>
          <span className="dt-code-meta">{t('{n} 行', { n: lines })}</span>
        </button>
        <button
          className={`dt-code-copy${copied ? ' copied' : ''}`}
          onClick={copy}
          title={copied ? t('已复制') : t('复制代码')}
          aria-label={t('复制代码')}
        >
          <Icon name={copied ? 'check' : 'copy'} size={12} />
        </button>
      </div>
      {open && (
        <pre className="dt-code-body" ref={ref} onScroll={onScroll}>
          <code>{code}</code>
        </pre>
      )}
    </div>
  );
}
