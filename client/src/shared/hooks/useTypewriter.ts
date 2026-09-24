
import { useEffect, useRef, useState } from 'react';

const MAX_STEP = 8;

export function useTypewriter(target: number, active: boolean): number {
  const [shown, setShown] = useState(active ? 0 : target);
  const shownRef = useRef(shown);
  const targetRef = useRef(target);
  const rafRef = useRef<number | null>(null);

  useEffect(() => {
    targetRef.current = target;

    if (!active) {
      if (rafRef.current != null) {
        cancelAnimationFrame(rafRef.current);
        rafRef.current = null;
      }
      shownRef.current = target;
      setShown(target);
      return;
    }

    if (rafRef.current == null) {
      const tick = () => {
        rafRef.current = requestAnimationFrame(tick);
        const t = targetRef.current;
        const cur = shownRef.current;
        if (cur >= t) return;
        const next = Math.min(t, cur + Math.max(1, Math.min(MAX_STEP, Math.ceil((t - cur) / 40))));
        shownRef.current = next;
        setShown(next);
      };
      rafRef.current = requestAnimationFrame(tick);
    }
  }, [target, active]);

  useEffect(
    () => () => {
      if (rafRef.current != null) cancelAnimationFrame(rafRef.current);
    },
    [],
  );

  return shown;
}
