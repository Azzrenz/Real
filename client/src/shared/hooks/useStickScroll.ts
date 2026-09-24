
import { useCallback, useRef } from 'react';

const STICK_THRESHOLD = 24;

export function useStickScroll<T extends HTMLElement>(threshold = STICK_THRESHOLD) {
  const ref = useRef<T | null>(null);
  const sticking = useRef(true);

  const onScroll = useCallback(() => {
    const el = ref.current;
    if (!el) return;
    sticking.current = el.scrollHeight - el.scrollTop - el.clientHeight < threshold;
  }, [threshold]);

  const stickToBottom = useCallback(() => {
    const el = ref.current;
    if (el && sticking.current) el.scrollTop = el.scrollHeight;
  }, []);

  return { ref, onScroll, stickToBottom };
}
