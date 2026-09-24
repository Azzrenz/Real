
import { memo, useEffect, useRef, useState } from 'react';
import { enTermify, handleProseClick, renderMarkdown } from '../../../shared/lib/markdown';
import { narrationDisplay } from '../narration/present';
import './narration.css';


export const NarrationLine = memo(function NarrationLine({
  text,
  live,
}: {
  text: string;
  live: boolean;
}) {
  const [shownLen, setShownLen] = useState(() => text.length);
  const shownRef = useRef(text.length);
  const targetRef = useRef(text.length);
  const lastAdvanceRef = useRef(Date.now());
  const clockRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const stopClock = () => {
    if (clockRef.current) {
      clearInterval(clockRef.current);
      clockRef.current = null;
    }
  };
  useEffect(() => {
    targetRef.current = text.length;
    if (!live) {
      stopClock();
      if (shownRef.current !== targetRef.current) {
        shownRef.current = targetRef.current;
        setShownLen(targetRef.current);
      }
      return;
    }
    if (targetRef.current > shownRef.current && Date.now() - lastAdvanceRef.current > 2000) {
      shownRef.current = targetRef.current;
      lastAdvanceRef.current = Date.now();
      setShownLen(targetRef.current);
    }
    if (targetRef.current <= shownRef.current) {
      stopClock();
      return;
    }
    if (!clockRef.current) {
      clockRef.current = setInterval(() => {
        const gap = targetRef.current - shownRef.current;
        if (gap <= 0) {
          stopClock();
          return;
        }
        const step = Math.max(1, Math.ceil(gap / 10));
        shownRef.current = Math.min(targetRef.current, shownRef.current + step);
        lastAdvanceRef.current = Date.now();
        setShownLen(shownRef.current);
      }, 50);
    }
  }, [text, live]);
  useEffect(() => stopClock, []);
  const { md: useMd, text: settledText } = narrationDisplay(text, live);
  const shown = live
    ? useMd
      ? settledText
      : text.slice(0, shownLen)
    : settledText;
  if (useMd) {
    return (
      <span
        className="narration-md"
        onClick={handleProseClick}
        dangerouslySetInnerHTML={{ __html: enTermify(renderMarkdown(shown)) }}
      />
    );
  }
  return <>{shown}</>;
});
