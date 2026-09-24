
export type ThinkBlock =
  | { kind: 'heading'; level: number; text: string }
  | { kind: 'para'; text: string }
  | { kind: 'list'; ordered: boolean; items: string[] }
  | { kind: 'quote'; text: string }
  | { kind: 'code'; lang: string; code: string; closed: boolean };

const FENCE_RE = /^\s*(`{3,}|~{3,})\s*([^\s`]*)\s*$/;
const HEADING_RE = /^(#{1,6})\s+(.*)$/;
const BULLET_RE = /^\s*[-*+•]\s+(.*)$/;
const ORDERED_RE = /^\s*(\d{1,2})[.)、]\s+(.*)$/;
const QUOTE_RE = /^\s*>\s?(.*)$/;

export function blockChars(b: ThinkBlock): number {
  switch (b.kind) {
    case 'code':
      return b.code.length;
    case 'list':
      return b.items.reduce((n, s) => n + s.length + 2, 0);
    default:
      return b.text.length;
  }
}

/** max chars per folded paragraph (~5 rendered lines) before starting a new one */
const PARA_MAX_CHARS = 220;

/** structural lines (list item / heading / quote): always keep their own line */
const STRUCT_LINE_RE = /^(#{1,6}\s|[-*+•]\s|\d{1,2}[.)、]\s|>\s?)/;

/**
 * Display fold for plain thinking text -- the SINGLE implementation both renderers
 * share (the `<pre>` legacy view uses it directly; the structured view uses it to
 * fold runs of body paragraphs).
 *
 * The model writes reasoning as "one sentence, one blank line". With
 * `white-space: pre-wrap` every blank line renders as a whole empty line, which is
 * the loose leading the user reported ("one sentence per paragraph -> huge line gap").
 * A blank line should therefore never occupy a full line -- but it is also the ONLY
 * paragraph boundary the model gives us, so it must still close the paragraph:
 *  - a blank line ends the current paragraph (it does not render as an empty line:
 *    blocks join on a SINGLE newline below);
 *  - lines inside one paragraph still fold together (up to PARA_MAX_CHARS);
 *  - list / heading / quote lines keep their own line (structure is not swallowed).
 *
 * Swallowing the blank line instead made consecutive paragraphs glue into one
 * run-on line, space-joined: "Done. Meanwhile I should check main.rs… hmm, list the dir
 * first. Done." -- four separate thoughts reading as one broken sentence.
 */
export function foldPlainThinkingText(text: string): string {
  const blocks: string[] = [];
  let prose = '';
  let struct: string[] = [];
  const flushProse = () => {
    if (prose) blocks.push(prose);
    prose = '';
  };
  const flushStruct = () => {
    if (struct.length) blocks.push(struct.join('\n'));
    struct = [];
  };
  for (const raw of text.split('\n')) {
    const line = raw.trim();
    if (!line) {
      flushProse();
      continue;
    }
    if (STRUCT_LINE_RE.test(line)) {
      flushProse();
      struct.push(line);
      continue;
    }
    flushStruct();
    if (!prose) {
      prose = line;
      continue;
    }
    if (prose.length + line.length + 1 <= PARA_MAX_CHARS) {
      prose = `${prose} ${line}`;
      continue;
    }
    flushProse();
    prose = line;
  }
  flushProse();
  flushStruct();
  return blocks.join('\n');
}

/** structured view: fold consecutive body paragraphs with the same rule */
function mergeShortParas(blocks: ThinkBlock[]): ThinkBlock[] {
  const out: ThinkBlock[] = [];
  let run: string[] = [];
  const flushRun = () => {
    if (!run.length) return;
    // foldPlainThinkingText now separates blocks with a single newline (a paragraph
    // boundary must not cost a whole blank line), so split on that, not on "\n\n".
    for (const t of foldPlainThinkingText(run.join('\n\n')).split('\n')) {
      if (t) out.push({ kind: 'para', text: t });
    }
    run = [];
  };
  for (const b of blocks) {
    if (b.kind === 'para') {
      run.push(b.text);
      continue;
    }
    flushRun();
    out.push(b);
  }
  flushRun();
  return out;
}

export function parseThinking(text: string): ThinkBlock[] {
  if (!text) return [];
  const lines = text.split('\n');
  const blocks: ThinkBlock[] = [];

  let para: string[] = [];
  let quote: string[] = [];
  let list: { ordered: boolean; items: string[] } | null = null;

  const flushPara = () => {
    if (!para.length) return;
    const t = para.join('\n').trim();
    para = [];
    if (t) blocks.push({ kind: 'para', text: t });
  };
  const flushQuote = () => {
    if (!quote.length) return;
    const t = quote.join('\n').trim();
    quote = [];
    if (t) blocks.push({ kind: 'quote', text: t });
  };
  const flushList = () => {
    if (!list) return;
    if (list.items.length) blocks.push({ kind: 'list', ordered: list.ordered, items: list.items });
    list = null;
  };
  const flushAll = () => {
    flushPara();
    flushQuote();
    flushList();
  };

  let i = 0;
  while (i < lines.length) {
    const line = lines[i];

    const fence = FENCE_RE.exec(line);
    if (fence) {
      flushAll();
      const marker = fence[1];
      const buf: string[] = [];
      i++;
      let closed = false;
      while (i < lines.length) {
        if (lines[i].trim().startsWith(marker)) {
          closed = true;
          i++;
          break;
        }
        buf.push(lines[i]);
        i++;
      }
      blocks.push({ kind: 'code', lang: fence[2] ?? '', code: buf.join('\n').replace(/\s+$/, ''), closed });
      continue;
    }

    if (!line.trim()) {
      flushAll();
      i++;
      continue;
    }

    const h = HEADING_RE.exec(line);
    if (h) {
      flushAll();
      blocks.push({ kind: 'heading', level: h[1].length, text: h[2].trim() });
      i++;
      continue;
    }

    const q = QUOTE_RE.exec(line);
    if (q) {
      flushPara();
      flushList();
      quote.push(q[1]);
      i++;
      continue;
    }

    const b = BULLET_RE.exec(line);
    const o = ORDERED_RE.exec(line);
    if (b || o) {
      flushPara();
      flushQuote();
      const ordered = !!o && !b;
      const item = (ordered ? (o as RegExpExecArray)[2] : (b as RegExpExecArray)[1]).trim();
      if (!list || list.ordered !== ordered) {
        flushList();
        list = { ordered, items: [] };
      }
      list.items.push(item);
      i++;
      continue;
    }

    flushQuote();
    flushList();
    para.push(line);
    i++;
  }

  flushAll();
  return mergeShortParas(blocks);
}

export function splitInline(text: string): Array<{ code?: string; bold?: string; text?: string }> {
  const out: Array<{ code?: string; bold?: string; text?: string }> = [];
  const re = /(`[^`\n]+`|\*\*[^*\n]+\*\*)/g;
  let last = 0;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text)) !== null) {
    if (m.index > last) out.push({ text: text.slice(last, m.index) });
    const tok = m[0];
    if (tok.startsWith('`')) out.push({ code: tok.slice(1, -1) });
    else out.push({ bold: tok.slice(2, -2) });
    last = m.index + tok.length;
  }
  if (last < text.length) out.push({ text: text.slice(last) });
  return out;
}
