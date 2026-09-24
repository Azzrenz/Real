
import { useMemo, useState } from 'react';
import { BODY_CHAR_BUDGET } from './contracts';
import { blockChars, parseThinking, splitInline, type ThinkBlock } from './parseThinking';
import { CodeWindow } from './CodeWindow';
import { useT } from '../../shared/i18n';

interface Props {
  text: string;
  streaming?: boolean;
  shownLen?: number;
}

function InlineMarks({ text }: { text: string }) {
  const parts = useMemo(() => splitInline(text), [text]);
  return (
    <>
      {parts.map((p, i) =>
        p.code ? (
          <code key={i} className="dt-inline-code">
            {p.code}
          </code>
        ) : p.bold ? (
          <strong key={i} className="dt-inline-bold">
            {p.bold}
          </strong>
        ) : (
          <span key={i}>{p.text}</span>
        ),
      )}
    </>
  );
}

function BlockView({ block }: { block: ThinkBlock }) {
  switch (block.kind) {
    case 'heading':
      return (
        <p className="dt-heading" data-level={block.level}>
          <InlineMarks text={block.text} />
        </p>
      );
    case 'list':
      return block.ordered ? (
        <ol className="dt-list dt-list-ordered">
          {block.items.map((it, i) => (
            <li key={i} className="dt-list-item">
              <InlineMarks text={it} />
            </li>
          ))}
        </ol>
      ) : (
        <ul className="dt-list">
          {block.items.map((it, i) => (
            <li key={i} className="dt-list-item">
              <InlineMarks text={it} />
            </li>
          ))}
        </ul>
      );
    case 'quote':
      return (
        <blockquote className="dt-quote">
          <InlineMarks text={block.text} />
        </blockquote>
      );
    case 'code':
      return <CodeWindow lang={block.lang} code={block.code} streaming={!block.closed} />;
    default:
      return (
        <p className="dt-para">
          <InlineMarks text={block.text} />
        </p>
      );
  }
}

export function ThinkingBody({ text, streaming, shownLen }: Props) {
  const [showAll, setShowAll] = useState(false);

  if (streaming) {
    return (
      <pre className="dt-raw">
        {text.slice(0, shownLen ?? text.length)}
        <span className="dt-cursor" aria-hidden="true" />
      </pre>
    );
  }

  return <StructuredBody text={text} showAll={showAll} onShowAll={() => setShowAll(true)} />;
}

function StructuredBody({
  text,
  showAll,
  onShowAll,
}: {
  text: string;
  showAll: boolean;
  onShowAll: () => void;
}) {
  const t = useT();
  const blocks = useMemo(() => parseThinking(text), [text]);
  const { visible, hiddenBlocks, hiddenChars } = useMemo(() => {
    if (showAll) return { visible: blocks, hiddenBlocks: 0, hiddenChars: 0 };
    let used = 0;
    const out: ThinkBlock[] = [];
    let hidden = 0;
    let hiddenChars = 0;
    for (const b of blocks) {
      const c = blockChars(b);
      if (used > 0 && used + c > BODY_CHAR_BUDGET) {
        hidden += 1;
        hiddenChars += c;
        continue;
      }
      used += c;
      out.push(b);
    }
    return { visible: out, hiddenBlocks: hidden, hiddenChars };
  }, [blocks, showAll]);

  return (
    <div className="dt-body">
      {visible.map((b, i) => (
        <BlockView key={i} block={b} />
      ))}
      {hiddenBlocks > 0 && (
        <div className="dt-more">
          <span className="dt-more-line" />
          <span className="dt-more-label">
            {t('还有 {blocks} 段 · {chars} 字未显示', {
              blocks: hiddenBlocks,
              chars: hiddenChars.toLocaleString(),
            })}
          </span>
          <span className="dt-more-line" />
          <button className="dt-more-btn" onClick={onShowAll}>
            {t('显示全部（{n} 字）', { n: text.length.toLocaleString() })}
          </button>
        </div>
      )}
    </div>
  );
}
