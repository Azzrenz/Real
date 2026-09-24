
interface Props {
  kind?: 'think' | 'tool';
  children: React.ReactNode;
  bodyRef?: React.Ref<HTMLDivElement>;
  onBodyScroll?: React.UIEventHandler<HTMLDivElement>;
}

export function DetailWindow({ kind = 'tool', children, bodyRef, onBodyScroll }: Props) {
  return (
    <div className={`detail-window detail-window--${kind}`}>
      <div className="detail-window-body" ref={bodyRef} onScroll={onBodyScroll}>
        {children}
      </div>
    </div>
  );
}
