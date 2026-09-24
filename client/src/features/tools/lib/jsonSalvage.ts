
export function salvageTruncatedJson(s: string): Record<string, unknown> | undefined {
  const t = s.trim();
  if (!t.startsWith('{')) return undefined;
  try {
    return JSON.parse(t) as Record<string, unknown>;
  } catch { /* trailing text or truncated mid-way: try to repair */ }
  const stack: string[] = [];
  const candidates: number[] = [];
  let inStr = false;
  let esc = false;
  for (let i = 0; i < t.length; i++) {
    const c = t[i];
    if (inStr) {
      if (esc) esc = false;
      else if (c === '\\') esc = true;
      else if (c === '"') inStr = false;
      continue;
    }
    if (c === '"') { inStr = true; continue; }
    if (c === '{') stack.push('}');
    else if (c === '[') stack.push(']');
    else if (c === '}' || c === ']') {
      stack.pop();
      if (stack.length === 0) candidates.push(i);
    }
  }
  for (let k = candidates.length - 1; k >= 0; k--) {
    try {
      const v = JSON.parse(t.slice(0, candidates[k] + 1)) as unknown;
      if (v && typeof v === 'object' && !Array.isArray(v)) {
        return v as Record<string, unknown>;
      }
    } catch { /* try a shorter candidate */ }
  }
  let fixed = t.slice(0, candidates.length > 0 ? candidates[0] + 1 : t.length);
  const stack2: string[] = [];
  inStr = false;
  esc = false;
  for (let i = 0; i < fixed.length; i++) {
    const c = fixed[i];
    if (inStr) {
      if (esc) esc = false;
      else if (c === '\\') esc = true;
      else if (c === '"') inStr = false;
      continue;
    }
    if (c === '"') { inStr = true; continue; }
    if (c === '{') stack2.push('}');
    else if (c === '[') stack2.push(']');
    else if (c === '}' || c === ']') stack2.pop();
  }
  if (inStr) {
    if (esc) fixed = fixed.slice(0, -1);
    fixed += '"';
  }
  fixed = fixed.replace(/[,:\s]+$/, '');
  while (stack2.length > 0) fixed += stack2.pop();
  try {
    const v = JSON.parse(fixed) as unknown;
    if (v && typeof v === 'object' && !Array.isArray(v)) {
      return v as Record<string, unknown>;
    }
  } catch { /* salvage failed */ }
  return undefined;
}

export function unwrapRawJson(name: string, text: string): string {
  const s = text.trimStart();
  if (!s.startsWith('{')) return text;
  let v: Record<string, unknown> | undefined;
  try {
    v = JSON.parse(s) as Record<string, unknown>;
  } catch {
    v = salvageTruncatedJson(s);
  }
  if (!v) return text;
  const truncatedMark = /…\[已截断/.test(s) ? '\n…[已截断，原结果更长]' : '';
  const data = v?.data;
  if (!data || typeof data !== 'object') return text;
  const d = data as Record<string, unknown>;
  const arr = (x: unknown): Array<Record<string, unknown>> =>
    Array.isArray(x) ? (x as Array<Record<string, unknown>>) : [];
  const str = (x: unknown) => (typeof x === 'string' ? x : '');
  const num = (x: unknown, fb = 0) => (typeof x === 'number' ? x : fb);
  switch (name) {
    case 'read': {
      const files = arr(d.files);
      if (files.length === 0) return text;
      const parts: string[] = [];
      for (const f of files) {
        let head = `${str(f.path) || '?'} · ${num(f.total_lines)} 行`;
        if (f.start_line != null && f.end_line != null) {
          head += ` · 本次读 ${num(f.start_line)}–${num(f.end_line)} 行`;
        }
        parts.push(head);
        const c = str(f.content);
        if (c) parts.push(c);
      }
      return parts.join('\n\n') + truncatedMark;
    }
    case 'modify':
    case 'edit': {
      const changes = arr(d.changes);
      if (changes.length === 0) return text;
      let out = `${str(d.file)}\n修改 ${changes.length} 处`;
      for (const c of changes) {
        for (const l of str(c.find).split('\n')) if (l.trim()) out += `\n- ${l}`;
        for (const l of str(c.replace).split('\n')) if (l.trim()) out += `\n+ ${l}`;
      }
      return out + truncatedMark;
    }
    case 'run':
    case 'verify': {
      if (typeof d.exit_code !== 'number' && !str(d.command)) return text;
      let out = `${str(d.command)}\nexit ${num(d.exit_code, -1)}`;
      if (str(d.stdout)) out += `\n${str(d.stdout)}`;
      if (str(d.stderr)) out += `\n[stderr]\n${str(d.stderr)}`;
      const err = v.error;
      if (err && typeof err === 'object') {
        const e = err as Record<string, unknown>;
        const code = str(e.code);
        const msg = str(e.message);
        if (code || msg) out += `\n[错误] ${code}${code && msg ? ': ' : ''}${msg}`;
      }
      const sug = str((v.error as Record<string, unknown> | undefined)?.suggestion) || str(d.suggestion);
      if (sug) out += `\n[建议] ${sug}`;
      return out + truncatedMark;
    }
    case 'write': {
      const path = str(d.path);
      const bytes = num(d.bytes_written, num(d.written, -1));
      if (!path && bytes < 0) return text;
      let out = path;
      if (bytes >= 0) out += `\n写入 ${bytes} 字节`;
      const bak = str(d.backup_path);
      if (bak) out += `\n备份: ${bak}`;
      return out + truncatedMark;
    }
    case 'list':
    case 'find_files':
    case 'audit': {
      const entries = arr(d.entries);
      const files = arr(d.files);
      if (entries.length > 0) {
        let out = `${num(d.dirs)} 目录 · ${num(d.files)} 文件`;
        for (const e of entries) out += `\n- ${str(e.path) || '?'}`;
        return out + truncatedMark;
      }
      if (files.length > 0) {
        let out = `${num(d.total, files.length)} 个文件`;
        for (const f of files) out += `\n- ${str(f.path) || '?'} · ${num(f.size)}B`;
        return out + truncatedMark;
      }
      return text;
    }
    default:
      return text;
  }
}
