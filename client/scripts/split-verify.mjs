#!/usr/bin/env node
// 拆分等价性校验 v3（postcss 真解析版）
//
// 为什么重写：v2 用手写解析器，规则起始行只认"含 { 的那行"——
// 多行选择器组（A,\nB,\nC { ... }）的**前几个选择器所在行不属于规则**，
// 于是拆分时会静默丢掉它们，而 v2 的原文件与拆分后文件用的是同一套（错误的）解析，
// 错误互相抵消 → 校验通过、界面缺样式。v3 用 postcss，选择器逐个比对。
//
// 用法: node scripts/split-verify.mjs <原文件> <拆分后文件...>
// 检查：① 选择器零丢失（逐个）② 每个选择器的"生效声明表"逐字一致
//      ③ @media/@keyframes 等 @ 块零丢失 ④ 有意偏离走 split-verify.ignore
import { readFileSync, existsSync } from 'node:fs';
import postcss from 'postcss';

const [, , srcFile, ...outFiles] = process.argv;
if (!srcFile || !outFiles.length) {
  console.log('用法: node scripts/split-verify.mjs <原文件> <拆分后文件...>');
  process.exit(0);
}

const norm = (s) => s.replace(/\s+/g, ' ').trim();

function parse(text) {
  const effective = new Map();   // selector -> Map(prop -> value)   后写覆盖
  const atBlocks = new Set();    // 归一化 @ 块
  const filesSel = new Set();
  const root = postcss.parse(text);
  root.walkRules((r) => {
    if (r.parent && r.parent.type === 'atrule' && /keyframes/.test(r.parent.name)) return;
    const cond = [];
    for (let p = r.parent; p && p.type !== 'root'; p = p.parent) {
      if (p.type === 'atrule') cond.unshift('@' + p.name + ' ' + p.params);
    }
    const prefix = cond.length ? cond.join(' && ') + ' >> ' : '';
    r.selectors.forEach((sel) => {
      const s = prefix + norm(sel);
      filesSel.add(s);
      if (!effective.has(s)) effective.set(s, new Map());
      const d = effective.get(s);
      r.walkDecls((x) => d.set(x.prop, x.value));
    });
  });
  root.walkAtRules((a) => {
    if (a.name === 'keyframes' || a.name === 'font-face') atBlocks.add(norm(a.toString()));
  });
  return { effective, atBlocks, filesSel };
}

const orig = parse(readFileSync(srcFile, 'utf8'));
const outs = outFiles.filter((f) => existsSync(f));
let merged = { effective: new Map(), atBlocks: new Set() };
for (const f of outs) {
  const p = parse(readFileSync(f, 'utf8'));
  for (const [sel, m] of p.effective) {
    if (!merged.effective.has(sel)) merged.effective.set(sel, new Map());
    const d = merged.effective.get(sel);
    for (const [k, v] of m) d.set(k, v);
  }
  p.atBlocks.forEach((b) => merged.atBlocks.add(b));
}

const ignoreFile = new URL('split-verify.ignore', import.meta.url);
const ALLOW = new Set(
  existsSync(ignoreFile)
    ? readFileSync(ignoreFile, 'utf8').split('\n').map((l) => l.split('#')[0].trim()).filter(Boolean)
    : [],
);

const missingSel = [];
const valueDiff = [];
for (const [sel, m] of orig.effective) {
  const cur = merged.effective.get(sel);
  if (!cur) { missingSel.push(sel); continue; }
  const diff = [...m].filter(([k, v]) => cur.get(k) !== v);
  if (diff.length && !ALLOW.has(sel)) {
    valueDiff.push(sel + '\n      原: ' + diff.map(([k, v]) => k + ':' + String(v).slice(0, 30)).join('; ') +
      '\n      今: ' + diff.map(([k]) => k + ':' + String(cur.get(k)).slice(0, 30)).join('; '));
  }
}
const missingAt = [...orig.atBlocks].filter((b) => !merged.atBlocks.has(b));

const rep = (title, arr, hint) => {
  console.log('\n' + (arr.length ? 'FAIL' : 'OK  ') + ' ' + title + ' (' + arr.length + ')' + (arr.length ? '  ' + hint : ''));
  arr.slice(0, 12).forEach((x) => console.log('   ' + String(x).slice(0, 150)));
  if (arr.length > 12) console.log('   ... 另有 ' + (arr.length - 12) + ' 条');
};

console.log('拆分等价性校验 v3（postcss）');
console.log('  原文件: ' + srcFile + '  选择器 ' + orig.effective.size + ' / @块 ' + orig.atBlocks.size);
console.log('  拆分后: ' + outs.length + ' 个文件  选择器 ' + merged.effective.size + ' / @块 ' + merged.atBlocks.size);
rep('选择器零丢失', missingSel, '→ 整条规则没搬过去（样式整块失效）');
rep('生效属性逐字一致', valueDiff, '→ 属性被改/被覆盖（视觉会变）');
rep('@media / @keyframes 零丢失', missingAt, '→ 响应式/动效失效');
const ok = !missingSel.length && !valueDiff.length && !missingAt.length;
console.log('\n' + (ok ? '✅ 等价性成立' : '❌ 等价性不成立：先修上面问题再交付'));
