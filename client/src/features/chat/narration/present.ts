
import { completeSentences, hasBlockMarkdown } from './pipeline';

export function stripRedundantTail(text: string, purpose: string): string {
  const norm = (s: string) => s.replace(/[\s。．.!！?？：:；;，,、~～…（）()]/g, '');
  const target = norm(purpose);
  if (!target || target.length < 4) return text;
  let cut = -1;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (c === '。' || c === '！' || c === '？' || c === '.' || c === '!' || c === '?' || c === '\n') cut = i;
  }
  if (cut < 0) return text;
  const head = text.slice(0, cut + 1);
  const tail = text.slice(cut + 1);
  if (!head.trim()) return text;
  return norm(tail) === target ? head.trimEnd() : text;
}

export function stablePart(text: string): string {
  const base = completeSentences(text);
  let end = base.length;
  let depth = 0;
  let lastOpen = -1;
  for (let i = 0; i < base.length; i++) {
    const c = base[i];
    if (c === '（' || c === '(') {
      if (depth === 0) lastOpen = i;
      depth++;
    } else if (c === '）' || c === ')') {
      depth = Math.max(0, depth - 1);
    }
  }
  if (depth > 0 && lastOpen >= 0) end = Math.min(end, lastOpen);
  const stars = base.split('**');
  if (stars.length % 2 === 0) {
    const lastStar = base.lastIndexOf('**');
    if (lastStar >= 0) end = Math.min(end, lastStar);
  }
  return base.slice(0, end).trimEnd();
}

function handoffCut(text: string): number {
  let q = text.search(/[；;]/);
  while (q >= 0) {
    const after = text.slice(q + 1).trimStart();
    const isListNext =
      /^[0-9０-９][.、．)）:：]/.test(after) ||
      /^[①②③④⑤⑥⑦⑧⑨⑩]/.test(after) ||
      /^[一二三四五六七八九十][、．.)）]/.test(after) ||
      /^第[一二三四五六七八九十]+步/.test(after);
    if (!isListNext) return q;
    const n1 = text.indexOf('；', q + 1);
    const n2 = text.indexOf(';', q + 1);
    const nn = n1 < 0 ? n2 : n2 < 0 ? n1 : Math.min(n1, n2);
    if (nn < 0) return -1;
    q = nn;
  }
  return -1;
}

function cutAtReadBoundary(base: string, maxChars: number): string {
  const head = base.slice(0, maxChars);
  const bounds = ['。', '！', '？', '；', ';', '，', '、', '\n', ' '];
  let cut = -1;
  for (const b of bounds) {
    const i = head.lastIndexOf(b);
    if (i > cut) cut = i;
  }
  if (cut > maxChars * 0.5) return base.slice(0, cut + 1).trimEnd();
  const slack = Math.ceil(maxChars * 1.2);
  const m = base.slice(maxChars, slack).search(/[。！？；;，、\n ]/);
  if (m >= 0) return base.slice(0, maxChars + m + 1).trimEnd();
  const sp = head.lastIndexOf(' ');
  if (sp > maxChars * 0.6) return base.slice(0, sp).trimEnd();
  return head.trimEnd();
}

export function narrationLead(text: string, maxChars: number): string {
  const t = text.trim();
  if (!t) return '';
  const p = handoffCut(t);
  const base = p >= 0 ? t.slice(0, p) : t;
  if (base.length <= maxChars) return base;
  return cutAtReadBoundary(base, maxChars);
}

const DANGLING_TAIL =
  /(?:[\s、，,;；：:]*)(?:要|准备|打算|计划|改用|换成|然后|接着|接下来|再去|再|比如|也就是|以及|和|与|或|并|并且|而且|于是|所以|因为|由于|如果|的话|进行|开始|继续|先|等|等等|以下|如下)$/;
export function tidyTail(text: string): string {
  let t = text.trimEnd();
  let prev = '';
  while (t !== prev && t.length > 2) {
    prev = t;
    t = t.replace(DANGLING_TAIL, '').trimEnd();
  }
  return t;
}

/** Closing colon for narration: it hands the sentence off to the tool row below, whose title
 *  carries the next step. Trailing punctuation goes first so we never emit "。：".
 *  Skipped when the text ends in a table row or a code fence - the colon would land outside
 *  the closed block and break the structure the renderer just built. */
export function endAtColon(text: string): string {
  const t = text.trimEnd();
  if (t.length < 4) return t;
  if (/[|`]$/.test(t)) return t;
  return t.replace(/[\s,，。．.、;；:：!！?？~～…]+$/, '') + '：';
}

export function shapeNarration(text: string, maxChars: number, handoff = false): string {
  const base = text.trim();
  if (!base) return '';
  const stripped = stripFirstPerson(base);
  const lead = narrationLead(stripped, maxChars);
  const tidied = tidyTail(lead);
  if (!handoff || tidied.length < 4) return tidied;
  return endAtColon(tidied);
}

export function narrationBudget(ctx: {
  reasoningChars: number;
  toolCount: number;
  thinkCount: number;
}): number {
  const { reasoningChars, toolCount, thinkCount } = ctx;
  if (reasoningChars >= 24000 || toolCount >= 10 || thinkCount >= 8) return 300;
  if (reasoningChars >= 8000 || toolCount >= 5 || thinkCount >= 4) return 200;
  if (reasoningChars >= 1500 || toolCount >= 2 || thinkCount >= 2) return 120;
  return 64;
}

export function hasPlanStructure(text: string): boolean {
  if (!text) return false;
  const items = text.match(
    /(?:^|[\s；;。！？!?\n])(?:\d{1,2}[.、．)）]|[①②③④⑤⑥⑦⑧⑨⑩]|[一二三四五六七八九十][、．.)）])/g,
  );
  if (items && items.length >= 2) return true;
  const seq = /(?:首先|其次|然后|接着|再|最后|第一步|第二步|第三步|最后一步|其一|其二)/.test(text);
  const plan = /(?:计划|打算|准备|接下来|步骤|安排|方案|分[几三两四五六七八]步|如下|按以下|分[几三]步走)/.test(text);
  return seq && plan;
}

const ASCII_WORD = /[A-Za-z0-9._-]/;
function bridgeSpace(last: string, next: string): string {
  if (!last || !next) return '';
  return ASCII_WORD.test(last) === ASCII_WORD.test(next) ? '' : ' ';
}

/** A head ending on terminal punctuation is a complete clause, so a following parenthesis
 *  is only an aside and can go. A colon does NOT count as closing: it promises the answer is
 *  still coming, and the parenthesis usually is that answer. */
const HEAD_CLOSED = /[。！？…；，、;,!?》」』”）)]$/;
/**
 * When the head ends on a predicate still waiting for its object (none / is / has / needs …),
 * the parenthesis IS that object. Dropping it leaves the predicate dangling and can invert the
 * meaning: "Environment: none (Python 3.14 available …)" becomes "Environment: none".
 * This is a closed word class (function words), not a chase after the model's wording.
 */
const DANGLING_OBJECT = /(?:没有|无|非|否|未|不|是|有|见|含|共|缺|少|需|要|查|看|读|用|为|走)$/;
/** A parenthesis is "long" above this many chars: then it is dropped only when parenDroppable
 *  says the head already stands on its own. Anything shorter just loses its brackets and keeps
 *  every word - fewer brackets, nothing lost. */
const PAREN_LONG = 4;

/** Whether a parenthesis can go: yes if the head is a complete clause, no if it dangles. */
function parenDroppable(head: string): boolean {
  const h = head.replace(/[*_`~\s]+$/, '').trimEnd();
  if (!h) return false;
  const last = h.slice(-1);
  if (HEAD_CLOSED.test(last)) return true;
  return !DANGLING_OBJECT.test(h);
}

function stripParens(text: string): string {
  return text
    .replace(/（([^）]*)）/g, (m, inner: string, offset: number, s: string) => {
      const body = inner.trim();
      const last = s.slice(0, offset).slice(-1);
      if (!body || body.length > PAREN_LONG) {
        if (body.length > PAREN_LONG && !parenDroppable(s.slice(0, offset))) return m;
        const next = s.slice(offset + m.length)[0] ?? '';
        return bridgeSpace(last, next);
      }
      return bridgeSpace(last, body[0] ?? '') + body;
    })
    .replace(/\(([^()]*)\)/g, (m, inner: string, offset: number, s: string) => {
      if (!/[\u4e00-\u9fff]/.test(inner)) return m;
      const body = inner.trim();
      const last = s.slice(0, offset).slice(-1);
      if (!body || body.length > PAREN_LONG) {
        if (body.length > PAREN_LONG && !parenDroppable(s.slice(0, offset))) return m;
        return bridgeSpace(last, s.slice(offset + m.length)[0] ?? '');
      }
      return bridgeSpace(last, body[0] ?? '') + body;
    });
}

/** Safe clean, shared by BOTH render paths (plain text and markdown).
 *  Touches only "text layer" concerns: brackets, stray spaces, repeated punctuation,
 *  leading/trailing punctuation, spaces squeezed between CJK chars.
 *  Never touches markdown structure - asterisks (bold), backticks (inline code), newlines
 *  (list and paragraph separators), pipes and colons (table syntax), hashes (headings).
 *  That is why it collapses only [ \t] and never folds \s down to a single space. */
export function cleanSoft(text: string): string {
  return stripParens(
    text
      .replace(/(?:、\s*)+等等/g, '')
      .replace(/(?:等等)+/g, '')
      .replace(/…+[ \t]*$/g, '')
      .replace(/([;；。、，,.!?！？])([ \t]*[;；。、，,.!?！？])+/g, '$1')
      .replace(/^[\s,，;；、.…·]+/, '')
      .replace(/[\s,，;；、]+$/, '')
      .replace(/[ \t]{2,}/g, ' ')
      .replace(/[ \t]+([，。、；：！？,.!?;:])/g, '$1')
      .replace(/([\u4e00-\u9fff])[ \t]+([\u4e00-\u9fff])/g, '$1$2')
      .trim(),
  );
}

/** Plain-text clean, for the path that does NOT go through markdown: it strips the markers
 *  themselves too, so the user never sees asterisks / backticks / quotes that a renderer
 *  would have eaten. Markers come off first, then cleanSoft; brackets are handled last
 *  (inside cleanSoft) because the keep-or-drop decision reads the surrounding context. */
export function cleanNarration(text: string): string {
  return cleanSoft(
    text
      .replace(/[`“”‘’「」『』"']/g, '')
      // No mid-sentence colon in narration; the closing one is added by endAtColon.
      .replace(/([\u4e00-\u9fff）)"】』])[ \t]*[：:][ \t]*/g, '$1')
      .replace(/([A-Za-z0-9])[ \t]*[：:][ \t]*(?=[\u4e00-\u9fff])/g, '$1 '),
  );
}

/** Inline list marker: `-①` / `- 1.` / `- 2)` — the ordinal may follow the dash anywhere. */
const INLINE_LIST_MARK = /[ \t]*-[ \t]*(?=[①-⑩]|\d{1,2}\s*[.、)）]\s*\S)/g;

/**
 * Restore a list that the model squeezed onto one line into one item per line.
 *
 * Markers like `-①done-②fail` carry neither a space nor a line break after the dash, so the
 * renderer sees no list boundary and the whole list collapses into one garbled line.
 * This only RESTORES boundaries — it inserts line breaks and the space after `-`, never infers
 * meaning or reorders content, so ordinary prose is left alone. Fewer than two markers returns
 * the text unchanged (a lone hyphen is usually intra-word, as in a-b).
 */
export function splitInlineListItems(text: string): string {
  const hits = text.match(INLINE_LIST_MARK);
  if (!hits || hits.length < 2) return text;
  return text.replace(INLINE_LIST_MARK, '\n- ');
}

const FIRST_PERSON_HEAD =
  /^(?:我们|我)(?:先|再|接着|然后|现在|就|马上|来|去|还|又|也|已经|已)?(?=[一-鿿])/;
export function stripFirstPerson(text: string): string {
  const t = text.trim();
  const m = t.match(FIRST_PERSON_HEAD);
  if (!m) return t;
  const rest = t.slice(m[0].length).trim();
  return rest.length >= 4 ? rest : t;
}

export function narrationPunct(text: string): string {
  const parts = text.split(/(`[^`]*`)/g);
  return parts
    .map((seg, i) => {
      if (i % 2 === 1) return seg;
      let t = seg;
      t = t.replace(/([。．；;！!？?，,、…])\1+/g, '$1');
      t = t.replace(/ +([。；;！!？?，,、…])/g, '$1');
      // Leading half-width comma becomes full-width, and the spaces after it collapse.
      // Only horizontal space is absorbed: \s would also eat the newline, folding a
      // list or table back into a single line before the renderer ever sees it.
      t = t.replace(/([，;；!！?])[ \t]*/g, (_m, pc) => (pc === ',' ? '，' : pc));
      t = t.replace(/[ \t]{2,}/g, ' ');
      return t;
    })
    .join('');
}

/** Does this narration need the markdown renderer?
 *  Block-level structure is delegated to hasBlockMarkdown so there is ONE definition of
 *  "carries multi-line structure" shared by the absorbed-title check and this gate.
 *  Everything below is what that function deliberately ignores: a single line of bold or
 *  inline code still has to reach the renderer, otherwise the markers show up literally. */
export function needsMarkdown(text: string): boolean {
  if (!text) return false;
  if (hasBlockMarkdown(text)) return true;
  // Numbering appended directly after CJK text, with or without a line break between them.
  if (/[\u4e00-\u9fa5]\s*\d{1,2}[.、)）](?!\d)/.test(text)) return true;
  // CJK numeral markers - hasBlockMarkdown only knows ASCII ordering.
  if (/(^|\n)\s*[一二三四五六七八九十]{1,2}[、.]\s*\S/.test(text)) return true;
  if (/\*\*[^*]+\*\*/.test(text)) return true;
  if (/`[^`\n]+`/.test(text)) return true;
  if (/\n[ \t]*\n/.test(text)) return true;
  return false;
}

export function narrationDisplay(text: string, live: boolean): { md: boolean; text: string } {
  if (live) return { md: needsMarkdown(text), text };
  const settled = endAtColon(narrationPunct(text));
  return { md: needsMarkdown(settled), text: settled };
}
