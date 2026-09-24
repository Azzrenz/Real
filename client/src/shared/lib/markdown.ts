// Markdown rendering: marked parse + DOMPurify sanitize.

import { marked } from 'marked';
import DOMPurify from 'dompurify';
import hljs from 'highlight.js/lib/core';
import python from 'highlight.js/lib/languages/python';
import typescript from 'highlight.js/lib/languages/typescript';
import javascript from 'highlight.js/lib/languages/javascript';
import bash from 'highlight.js/lib/languages/bash';
import json from 'highlight.js/lib/languages/json';
import css from 'highlight.js/lib/languages/css';
import xml from 'highlight.js/lib/languages/xml';
import rust from 'highlight.js/lib/languages/rust';
import type { MouseEvent as ReactMouseEvent } from 'react';
import { t } from '../i18n';
import { openExternal } from './openExternal';
import { openInDock } from './openInDock';

// Registered one language at a time: the full bundle is ~1 MB, these eight cover every code block
for (const [name, lang] of [
  ['python', python], ['typescript', typescript], ['javascript', javascript], ['bash', bash],
  ['json', json], ['css', css], ['xml', xml], ['rust', rust],
] as const) {
  hljs.registerLanguage(name, lang);
}

marked.setOptions({
  gfm: true,
  breaks: true,
});

function normalizeBlockBoundaries(text: string): string {
  return text
    .replace(/(^|\n)(#{1,6})([^\s#])/g, '$1$2 $3')
    .replace(/([^\n])\n(#{1,6}\s)/g, '$1\n\n$2')
    .replace(/([^\n])\n(\s*[-*+]\s)/g, '$1\n\n$2')
    .replace(/([^\n])\n(\s*\d+[.)、]\s)/g, '$1\n\n$2')
    .replace(/([\u4e00-\u9fa5])\s*(\d{1,2}[.、)）](?!\d))/g, '$1\n$2');
}

export function renderMarkdown(content: string): string {
  const normalized = normalizeBlockBoundaries(content ?? '');
  const raw = marked.parse(normalized, { async: false }) as string;
  const clean = DOMPurify.sanitize(raw);
  const doc = new DOMParser().parseFromString(clean, 'text/html');
  for (const table of doc.querySelectorAll('table')) {
    const first = [...table.querySelectorAll('tbody tr')].map(
      (tr) => tr.querySelector('td')?.textContent?.trim() ?? '',
    );
    if (first.length > 0 && first.every((c) => /^\d{1,4}$/.test(c))) {
      table.classList.add('index-col');
    }
  }
  // A code block gets a header bar with its language and a copy button. The bar is built here,
  // after sanitising, because DOMPurify strips the attributes a React onClick would need --
  // the copy action is handled by delegation on the message container instead.
  for (const pre of [...doc.querySelectorAll('pre')]) {
    const code = pre.querySelector('code');
    const lang = (code?.className.match(/language-([\w+#-]+)/) ?? [])[1] ?? '';
    if (code && lang && hljs.getLanguage(lang)) {
      // Runs after sanitising, so the span markup highlight.js emits survives; doing it earlier
      // would have DOMPurify strip the classes and leave the block flat.
      code.innerHTML = hljs.highlight(code.textContent ?? '', { language: lang }).value;
      code.classList.add('hljs');
    }
    const wrap = doc.createElement('div');
    wrap.className = 'code-wrap';
    const head = doc.createElement('div');
    head.className = 'code-head';
    const label = doc.createElement('span');
    label.className = 'code-lang';
    label.textContent = lang || 'text';
    const btn = doc.createElement('button');
    btn.type = 'button';
    btn.className = 'code-copy';
    btn.setAttribute('data-copy', 'code');
    btn.setAttribute('aria-label', t('复制代码'));
    btn.setAttribute('title', t('复制代码'));
    // Icon only, no label: the header bar stays quiet, and the glyph is self-explanatory.
    btn.innerHTML =
      '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" ' +
      'stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' +
      '<rect x="9" y="9" width="13" height="13" rx="2"/>' +
      '<path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>';
    head.append(label, btn);
    pre.replaceWith(wrap);
    wrap.append(head, pre);
  }
  markPaths(doc);
  return doc.body.innerHTML;
}

/** What a path body may contain, minus the characters that cannot appear in a path written inside */
const PATH_BODY = String.raw`[^\s<>"'\`|*?（）：；，。！？【】…—]`;

/** A POSIX segment may not contain a slash, which is what keeps the `//` of a URL -- or of a */
const PATH_SEG = String.raw`[^\s<>"'\`|*?\/（）：；，。！？【】…—]`;

/** Drive letter or UNC share. The lookbehind is what stops the `s:` of `https:` being read as a */
const WIN_PATH = String.raw`(?<![A-Za-z0-9])(?:[A-Za-z]:[\\/]|\\\\)${PATH_BODY}+`;

/** POSIX, and the same idea one level down: the leading slash must not follow `:` or `/`, so a URL */
const POSIX_PATH = String.raw`(?<![:\/\w])\/${PATH_SEG}+(?:\/${PATH_SEG}+)*`;

/** Glob form of the two above, for splitting a text node into path / non-path pieces. */
const PATH_RE = `${WIN_PATH}|${POSIX_PATH}`;

/** Punctuation that may trail a match. `.` and `)` cannot be excluded from the body because real */
const TRAILING_PUNCT = /[.,;:!?、，。；：！？)\]）】》"'“”‘’`]+$/;

function trimPathTail(value: string): string {
  return value.replace(TRAILING_PUNCT, '');
}

/** Whether a whole string is a usable local path.
 *
 *  Used for inline code and for markdown links, both of which arrive already delimited by the
 *  author -- so the test can be looser than the prose one, and this is the only place a path with a
 *  space in it (`D:\My Documents\a.html`) still survives. */
export function looksLikePath(value: string): boolean {
  const v = trimPathTail(value.trim());
  if (v.length < 4) return false;
  if (/^[A-Za-z]:[\\/]/.test(v)) return true;
  if (/^\\\\[^\\/]+[\\/]/.test(v)) return true;
  return /^\/(?:[\w.@+~%-]+\/)+/.test(v);
}

/**
 * Marks anything that reads as a filesystem path so the message container can open it on click.
 * Inline code is checked first (its whole content is often exactly a path); bare paths in prose
 * are wrapped next. Code blocks are skipped -- there a path is source text, not a link target.
 * Runs after DOMPurify, which would otherwise drop the data-path attribute.
 */
function markPaths(doc: Document): void {
  const re = new RegExp(PATH_RE);
  for (const code of [...doc.querySelectorAll('code')]) {
    if (code.closest('pre')) continue;
    const value = trimPathTail((code.textContent ?? '').trim());
    // Inline code is the author saying "this whole thing is one token", so the loose check applies:
    // a path containing a space survives here and nowhere else.
    if (value && !value.includes('\n') && looksLikePath(value)) {
      code.classList.add('path-ref');
      code.setAttribute('data-path', value);
    }
  }
  // Walk text nodes after the inline-code pass, so already-marked nodes are skipped by the filter.
  const walker = doc.createTreeWalker(doc.body, NodeFilter.SHOW_TEXT);
  const targets: Text[] = [];
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    const text = (n as Text).nodeValue ?? '';
    const parent = n.parentElement;
    if (!text || !parent) continue;
    if (parent.closest('pre, a, .path-ref, .code-wrap, .code-head')) continue;
    if (re.test(text)) targets.push(n as Text);
  }
  for (const node of targets) {
    const text = node.nodeValue ?? '';
    const parts = text.split(new RegExp(`(${PATH_RE})`, 'g'));
    if (parts.length < 2) continue;
    const frag = doc.createDocumentFragment();
    for (const part of parts) {
      if (!part) continue;
      const exact = re.exec(part)?.[0] === part;
      const value = exact ? trimPathTail(part) : '';
      if (!value) {
        frag.append(doc.createTextNode(part));
        continue;
      }
      const span = doc.createElement('span');
      span.className = 'path-ref';
      span.setAttribute('data-path', value);
      span.textContent = value;
      frag.append(span);
      // A trimmed tail is prose again (`report.html.`), so it goes back in as plain text.
      if (value.length < part.length) frag.append(doc.createTextNode(part.slice(value.length)));
    }
    node.replaceWith(frag);
  }
}

/** Where a path click should go. The dock passes its own setter so that a path clicked inside a
 *  preview swaps the preview in place, rather than re-notifying a window that is already showing it. */
export interface ProseClickOptions {
  onOpenPath?: (path: string) => void;
}

/** Open handler for the path references above. Delegated for the same reason as the copy button
 *  -- they live in raw injected HTML, where a React onClick cannot be attached. Returns true
 *  when it handled the click. */
export function openPathFromEvent(
  e: ReactMouseEvent,
  onOpenPath?: (path: string) => void,
): boolean {
  const el = (e.target as HTMLElement)?.closest?.('.path-ref');
  if (!el) return false;
  const path = el.getAttribute('data-path');
  if (!path) return true;
  if (onOpenPath) onOpenPath(path);
  else void openInDock(path);
  return true;
}

/** Handler for links inside a rendered reply.
 *
 *  Doing nothing is NOT an option here: a plain `<a href>` in a Tauri webview navigates the current
 *  window, so an unhandled click replaces the entire app with the target page and leaves the user no
 *  way back. Every link click is therefore cancelled first and then routed -- http(s) to the system
 *  browser, a local file target to the dock. */
export function openLinkFromEvent(e: ReactMouseEvent): boolean {
  const a = (e.target as HTMLElement)?.closest?.('a[href]');
  if (!a) return false;
  const href = (a.getAttribute('href') ?? '').trim();
  // An in-page anchor is the one case where the default behaviour is the right one.
  if (!href || href.startsWith('#')) return true;
  e.preventDefault();
  if (/^https?:\/\//i.test(href)) {
    void openExternal(href);
  } else if (looksLikePath(href)) {
    void openInDock(trimPathTail(href));
  }
  return true;
}

/** Single entry point for clicks inside rendered markdown: code-copy button, then path references,
 *  then links. All three live in injected HTML, so a container delegates here rather than binding
 *  an onClick to each node. */
export function handleProseClick(e: ReactMouseEvent, opts?: ProseClickOptions): void {
  if (copyCodeFromEvent(e)) return;
  if (openPathFromEvent(e, opts?.onOpenPath)) return;
  openLinkFromEvent(e);
}

/** Copy handler for the code blocks above. Delegated because the button lives in raw HTML,
 *  where a React onClick can never be attached. Returns true when it handled the click. */
export function copyCodeFromEvent(e: ReactMouseEvent): boolean {
  const btn = (e.target as HTMLElement)?.closest?.('.code-copy');
  if (!btn) return false;
  const code = btn.closest('.code-wrap')?.querySelector('pre code');
  const text = code?.textContent ?? '';
  if (!text) return true;
  void navigator.clipboard.writeText(text).then(
    () => {
      btn.classList.add('ok');
      setTimeout(() => btn.classList.remove('ok'), 1200);
    },
    () => undefined,
  );
  return true;
}

function isTechTerm(tok: string): boolean {
  if (tok.length < 2) return false;
  if (/[_./#-]/.test(tok)) return true;
  if (/[a-z][A-Z]/.test(tok)) return true; // camelCase
  if (/[A-Z]{2,}/.test(tok)) return true;
  if (/^[A-Z][a-z]/.test(tok) && tok.length >= 3) return true; // PascalCase
  if (/\d/.test(tok) && /[A-Za-z]/.test(tok)) return true;
  return false;
}

export function enTermify(html: string): string {
  return html.replace(/(>)([^<]+)(<)/g, (_m, open, txt, close) =>
    open + txt.replace(/([A-Za-z][A-Za-z0-9_.+#/-]*)/g, (tok: string) =>
      isTechTerm(tok) ? `<span class="en-term">${tok}</span>` : tok,
    ) + close,
  );
}
