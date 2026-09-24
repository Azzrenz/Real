#!/usr/bin/env node
// UI 卫生自检：把"叠加漂移"变成可检测红线。用法：node scripts/ui-guard.mjs
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, extname, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..', 'src');
const CJK = /[\u4e00-\u9fa5]/;
const LIMITS = { css: 800, ts: 600, tsx: 600 };
const TOKEN_FILES = ['tokens.css'];

function walk(dir, out = []) {
  for (const f of readdirSync(dir)) {
    if (f === 'node_modules' || f === 'dist') continue;
    const p = join(dir, f);
    if (statSync(p).isDirectory()) walk(p, out);
    else if (['.css', '.ts', '.tsx'].includes(extname(f))) out.push(p);
  }
  return out;
}

const files = walk(ROOT);
const issues = { dup: [], dupThemed: [], big: [], zh: [], color: [], prompt: [] };

for (const p of files) {
  const rel = p.slice(ROOT.length + 1).split('\\').join('/');
  const lines = readFileSync(p, 'utf8').split('\n');
  const ext = extname(p).slice(1);
  if (lines.length > LIMITS[ext]) issues.big.push(rel + '  ' + lines.length + ' 行 > ' + LIMITS[ext]);

  if (ext === 'css' && !TOKEN_FILES.includes(rel.split('/').pop())) {
    lines.forEach((l, i) => {
      if (/#[0-9a-fA-F]{6}\b/.test(l) && !l.trim().startsWith('*'))
        issues.color.push(rel + ':' + (i + 1) + '  ' + l.trim().slice(0, 60));
    });
  }

  // Chinese comments: line AND block. Checking only `//` left the whole /* */ class
  // invisible, which is exactly where the stale prose had accumulated (2026-09-13 audit).
  // Both delimiters must anchor to a real comment position — a bare `/*` inside a string
  // (e.g. a glob like '**/*.ts') must not open a block and swallow the rest of the file.
  let inBlock = false;
  lines.forEach((l, i) => {
    const s = l.trim();
    const blockStart = /^\/\*/.test(s) || /\{\s*\/\*/.test(s);
    const blockEnd = /\*\/$/.test(s) || /\*\/\s*\}/.test(s);
    const isComment = inBlock || blockStart || s.startsWith('//');
    if (isComment && CJK.test(s)) issues.zh.push(rel + ':' + (i + 1) + '  ' + s.slice(0, 56));
    if (blockStart && !blockEnd) inBlock = true;
    else if (blockEnd) inBlock = false;
  });

  if (ext === 'ts' || ext === 'tsx') {
    lines.forEach((l, i) => {
      if (/(systemPrompt|defaultSystemPrompt)\s*[:=]\s*['"`][^'"`]{10,}/.test(l))
        issues.prompt.push(rel + ':' + (i + 1) + '  ' + l.trim().slice(0, 70));
    });
  }
}

function show(name, arr, hint) {
  console.log('\n' + (arr.length ? 'WARN' : 'OK  ') + ' ' + name + ' (' + arr.length + ')  ' + (arr.length ? hint : ''));
  arr.slice(0, 12).forEach((x) => console.log('   ' + x));
  if (arr.length > 12) console.log('   ... 另有 ' + (arr.length - 12) + ' 条');
}

console.log('UI 卫生自检 - ' + files.length + ' 个文件');
show('同名重复·真冗余', issues.dup, '-> 同前缀重复=漂移源：改旧的、删旧的');
show('同名重复·主题覆盖', issues.dupThemed, '-> 带 [data-theme] 前缀的后段覆盖，合法');
show('文件超行数红线', issues.big, '-> 超限即归位（职能拆到对应部门）');
show('残留中文注释', issues.zh, '-> 注释只写约束不写流水；改动说明进 commit/技能');
show('硬编码色值(非 tokens.css)', issues.color, '-> 应走 token，一处改全 UI 跟随');
show('前端 prompt 话术硬编码', issues.prompt, '-> 话术归 server/prompts/ 资产');
const total = Object.values(issues).reduce((a, b) => a + b.length, 0);
console.log('\n合计 ' + total + ' 项。理想态：重复=0，超限=0，中文注释=0。');


// ── API 闭环检查（前端路径模板 vs 后端路由表）────────────────────────────
import { existsSync } from 'node:fs';
const EP = join(ROOT, 'services', 'endpoints.ts');
const ROUTES = join(ROOT, '..', '..', 'server', 'src', 'routes', 'mod.rs');
if (existsSync(EP) && existsSync(ROUTES)) {
  const epSrc = readFileSync(EP, 'utf8');
  const SSE = join(ROOT, '..', '..', 'server', 'src', 'sse', 'mod.rs');
  const rtSrc = readFileSync(ROUTES, 'utf8') + (existsSync(SSE) ? readFileSync(SSE, 'utf8') : '');
  const norm = (s) => s.replace(/\$\{[^}]*\}/g, '{p}').replace(/\{[^}]*\}/g, '{p}').replace(/:[A-Za-z_]\w*/g, '{p}').split('?')[0].replace(/\/$/, '').replace(/\{p\}$/, '');
  const front = new Set();
  for (const m of epSrc.matchAll(/[`'"](\/api\/[^`'"\n]*)/g)) front.add(norm(m[1]));
  const back = new Set();
  for (const m of rtSrc.matchAll(/\.route\(\s*"([^"]+)"/g)) back.add(norm(m[1]));
  const ghost = [...front].filter((p) => !back.has(p));
  console.log('\n' + (ghost.length ? 'WARN' : 'OK  ') + ' API 闭环（前端 ' + front.size + ' / 后端 ' + back.size + '）');
  ghost.forEach((g) => console.log('   后端无此路由: ' + g));
  if (ghost.length === 0) console.log('   前端调用的每个路径后端都存在');
}


// ── 相对 import 可解析性（tsc 能过但 Vite 会 500 的那类错：路径层级/文件不存在）─────
import { existsSync as _exists } from 'node:fs';
import { dirname as _dir, resolve as _res } from 'node:path';
{
  const missing = [];
  const cand = (base) => [base, base + '.ts', base + '.tsx', base + '.css', base + '/index.ts', base + '/index.tsx'];
  for (const file of files) {
    if (!file.endsWith('.ts') && !file.endsWith('.tsx')) continue;
    const dir = _dir(file);
    const src = readFileSync(file, 'utf8');
    const specs = [...src.matchAll(/(?:from |import\()\s*'((?:\.\.?\/)+[^']+)'/g)].map((m) => m[1]);
    for (const spec of specs) {
      const target = _res(dir, spec);
      if (!cand(target).some((c) => _exists(c))) {
        missing.push(file.slice(ROOT.length + 1).split('\\').join('/') + '  ->  ' + spec);
      }
    }
  }
  console.log('\n' + (missing.length ? 'WARN' : 'OK  ') + ' 相对 import 可解析');
  missing.slice(0, 12).forEach((m) => console.log('   ' + m));
  if (missing.length > 12) console.log('   ... 另有 ' + (missing.length - 12) + ' 条');
}


// ── CSS/TS 注释配平 + CSS 花括号配平（静默杀手：未闭合 /* 会把后面所有规则吞掉，
//   tsc 与浏览器都不报错，只表现为"样式整段失效"——2026-09-10 composer 塌陷事故根因）──
{
  const broken = [];
  for (const file of files) {
    const src = readFileSync(file, 'utf8');
    const rel = file.slice(ROOT.length + 1).split('\\').join('/');
    const open = (src.match(/\/\*/g) || []).length;
    const close = (src.match(/\*\//g) || []).length;
    // TS/TSX 里正则字面量可能含 */，只在 CSS 与 TS 注释块内严格判；这里统一提示、人工确认
    if (file.endsWith('.css') && open !== close) broken.push(rel + '  注释不配平 /* ' + open + ' vs */ ' + close);
    if (file.endsWith('.css')) {
      const ob = (src.match(/\{/g) || []).length, cb = (src.match(/\}/g) || []).length;
      if (ob !== cb) broken.push(rel + '  花括号不配平 { ' + ob + ' vs } ' + cb);
    }
  }
  console.log('\\n' + (broken.length ? 'WARN' : 'OK  ') + ' CSS 注释/花括号配平');
  broken.slice(0, 10).forEach((b) => console.log('   ' + b));
}


// ── 同名选择器重复（postcss 精确解析：区分"真冲突=死声明"与"互补叠加=无害"）──
{
  let postcss = null;
  try { postcss = (await import('postcss')).default; } catch { postcss = null; }
  if (!postcss) {
    console.log('\n(跳过同名重复检查：postcss 不可用)');
  } else {
    const conflicts = [];
    const complements = [];
    const parseFail = [];
    for (const file of files.filter((f) => f.endsWith('.css'))) {
      const rel = file.slice(ROOT.length + 1).split('\\').join('/');
      let root;
      try { root = postcss.parse(readFileSync(file, 'utf8')); }
      catch (e) { parseFail.push(rel + '  ' + String(e.reason || e.message).slice(0, 90) + '  @L' + e.line); continue; }
      const ctxOf = (r) => { const a = []; let q = r.parent; while (q && q.type !== 'root') { if (q.type === 'atrule') a.unshift('@' + q.name + ' ' + q.params); q = q.parent; } return a.join(' && '); };
      const groups = new Map();
      root.walkRules((r) => {
        const k = ctxOf(r) + ' || ' + r.selector.split(',').map((x) => x.trim()).join(',');
        if (!groups.has(k)) groups.set(k, []);
        groups.get(k).push(r);
      });
      for (const [k, occ] of groups) {
        if (occ.length < 2) continue;
        const per = occ.map((r) => { const m = new Map(); r.walkDecls((d) => m.set(d.prop, d.value)); return m; });
        const all = new Set(); per.forEach((m) => m.forEach((_, pr) => all.add(pr)));
        const bad = [];
        for (const pr of all) {
          const vals = per.filter((m) => m.has(pr)).map((m) => m.get(pr));
          if (vals.length > 1 && new Set(vals).size > 1) bad.push(pr + '(' + vals[0].slice(0, 18) + '→' + vals[vals.length - 1].slice(0, 18) + ')');
        }
        const sel = k.split(' || ')[1];
        const where = occ.map((r) => r.source.start.line).join(',');
        if (bad.length) conflicts.push(rel + ':' + where + '  ' + sel.slice(0, 50) + '  ' + bad.join(' '));
        else complements.push(rel + ':' + where + '  ' + sel.slice(0, 50));
      }
    }
    console.log('\n' + (parseFail.length ? 'WARN' : 'OK  ') + ' CSS 可解析性(' + parseFail.length + ')');
    parseFail.forEach((x) => console.log('   ' + x));
    console.log((conflicts.length ? 'WARN' : 'OK  ') + ' 同名选择器·真冲突（死声明）(' + conflicts.length + ')');
    conflicts.slice(0, 10).forEach((x) => console.log('   ' + x));
    console.log((complements.length ? 'INFO' : 'OK  ') + ' 同名选择器·互补叠加（无害）(' + complements.length + ')');
    complements.slice(0, 5).forEach((x) => console.log('   ' + x));
  }
}
