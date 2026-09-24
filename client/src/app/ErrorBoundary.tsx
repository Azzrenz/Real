
import { Component, type ReactNode } from 'react';
import { t } from '../shared/i18n';

interface Props {
  children: ReactNode;
}

interface State {
  error: Error | null;
}

export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: unknown) {
    console.error('[ErrorBoundary] 渲染崩溃:', error, info);
  }

  render() {
    if (this.state.error) {
      return (
        <div
          style={{
            padding: '24px',
            margin: '16px',
            borderRadius: '10px',
            border: '1px solid var(--danger)',
            background: 'var(--danger-soft)',
            color: 'var(--text)',
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--fs-micro)',
            lineHeight: 1.6,
            whiteSpace: 'pre-wrap',
            wordBreak: 'break-all',
          }}
        >
          <div style={{ fontWeight: 700, marginBottom: 8, color: 'var(--danger, #c96a6a)' }}>
            {t('界面渲染出错')}
          </div>
          <div>{String(this.state.error?.message ?? this.state.error)}</div>
          <button
            onClick={() => this.setState({ error: null })}
            style={{
              marginTop: 12,
              padding: '6px 14px',
              border: '1px solid var(--border, #ccc)',
              borderRadius: 6,
              background: 'var(--bg, #fff)',
              cursor: 'pointer',
            }}
          >
            {t('重试')}
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}
