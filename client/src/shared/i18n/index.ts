// Minimal runtime i18n. The Chinese source text IS the key: any entry missing

import { useCallback } from 'react';
import { en } from './en';
import { useLang } from './lang';

export { LANGS, useLang } from './lang';
export type { Lang } from './lang';

type Vars = Record<string, string | number>;

export function translate(zh: string, lang: string, vars?: Vars): string {
  let s = lang === 'zh' ? zh : en[zh] ?? zh;
  if (vars) {
    for (const k of Object.keys(vars)) s = s.split('{' + k + '}').join(String(vars[k]));
  }
  return s;
}

/** The shape shared by t() and useT()'s return, for helpers that take one as an argument. */
export type T = (zh: string, vars?: Vars) => string;

/** For non-component call sites (stores, handlers, helpers). Components use useT(). */
export function t(zh: string, vars?: Vars): string {
  return translate(zh, useLang.getState().lang, vars);
}

/** Subscribes to the language, so the component re-renders on switch. */
export function useT(): T {
  const lang = useLang((s) => s.lang);
  return useCallback((zh: string, vars?: Vars) => translate(zh, lang, vars), [lang]);
}
