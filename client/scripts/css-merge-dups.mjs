#!/usr/bin/env node
// CSS 同选择器重复：真冲突清理 + 安全合并（零行为变化，自带等价性证明）
//
// 三类处理：
//   [真冲突]  同选择器多次出现，同一字段被后面的值覆盖 → 前面的声明是死代码，删除（合并时天然消失）
//   [可合并]  同选择器多次出现但字段互补，且两处之间没有"同字段规则" → 合并到最后一处
//   [跳过]    中间存在特异性更高的同字段规则（生效结果可能变）→ 不动，仅报告
//
// 安全性论证：
//   ① 同选择器、同特异性 → 后写必胜 → 前面的同字段声明恒为死代码
//   ② 若最后的出现处也声明了该字段，且中间规则的特异性均不高于本选择器 → 删除前面声明不影响任何元素
//   ③ 合并后重新计算"每个选择器的最终生效声明表"，必须与合并前逐字相同，否则整文件回滚
//
// 用法: node scripts/css-merge-dups.mjs [--write]
import { readFileSync, writeFileSync } from 'node:fs';
import postcss from 'postcss';

const WRITE = process.argv.includes('--write');
// 自动发现 src 下所有 CSS（避免遗漏新文件）
import { readdirSync, statSync } from 'node:fs';
import { join, extname } from 'node:path';
function walkCss(dir, out = []) {
  for (const f of readdirSync(dir)) {
    if (f === 'node_modules' || f === 'dist') continue;
    const p = join(dir, f);
    if (statSync(p).isDirectory()) walkCss(p, out);
    else if (extname(f) === '.css') out.push(p.split('\\').join('/'));
  }
  return out;
}
const FILES = walkCss('src');

const ctxOf = (rule) => {
  const parts = [];
  let p = rule.parent;
  while (p && p.type !== 'root') { if (p.type === 'atrule') parts.unshift('@' + p.name + ' ' + p.params); p = p.parent; }
  return parts.join(' && ');
};
const keyOf = (rule) => ctxOf(rule) + ' || ' + rule.selector.split(',').map((s) => s.trim()).join(',');

// 粗略特异性：id*100 + (类/属性/伪类)*10 + 元素*1，取选择器列表中最大值
function specificity(selector) {
  const one = (s) => {
    const ids = (s.match(/#[\w-]+/g) || []).length;
    const cls = (s.match(/\.[\w-]+|\[[^\]]+\]|:(?!:)[\w-]+/g) || []).length;
    const els = (s.replace(/[#.][\w-]+|\[[^\]]+\]|::?[\w-]+(\([^)]*\))?/g, ' ').match(/[a-zA-Z][\w-]*/g) || []).length;
    return ids * 100 + cls * 10 + els;
  };
  return Math.max(...selector.split(',').map((x) => one(x.trim())));
}

function effective(root) {
  const out = new Map();
  root.walkRules((r) => {
    const k = keyOf(r);
    if (!out.has(k)) out.set(k, new Map());
    const m = out.get(k);
    r.walkDecls((d) => m.set(d.prop, d.value));
  });
  return out;
}
const mapsEqual = (a, b) => {
  if (a.size !== b.size) return false;
  for (const [k, v] of a) {
    const w = b.get(k);
    if (!w || w.size !== v.size) return false;
    for (const [p, val] of v) if (w.get(p) !== val) return false;
  }
  return true;
};

let mergedTotal = 0, deadTotal = 0, skipTotal = 0;
for (const file of FILES) {
  const src = readFileSync(file, 'utf8');
  const root = postcss.parse(src);
  const before = effective(root);

  const flat = [];
  root.walkRules((r) => flat.push(r));
  const groups = new Map();
  flat.forEach((r, idx) => {
    const k = keyOf(r);
    if (!groups.has(k)) groups.set(k, []);
    groups.get(k).push({ r, idx });
  });

  const lines = [];
  const actions = [];
  for (const [k, occ] of groups) {
    if (occ.length < 2) continue;
    const sel = k.split(' || ')[1];
    const propsAll = new Set();
    occ.forEach(({ r }) => r.walkDecls((d) => propsAll.add(d.prop)));
    const last = occ[occ.length - 1];
    const lastProps = new Map();
    last.r.walkDecls((d) => lastProps.set(d.prop, d.value));
    const mySpec = specificity(sel);

    // 真冲突：同字段多次出现且值不同
    const per = occ.map(({ r }) => { const m = new Map(); r.walkDecls((d) => m.set(d.prop, d.value)); return m; });
    const conflicts = [];
    for (const p of propsAll) {
      const vals = per.filter((m) => m.has(p)).map((m) => m.get(p));
      if (vals.length > 1 && new Set(vals).size > 1) conflicts.push({ p, from: vals[0], to: vals[vals.length - 1] });
    }

    // 中间是否存在"特异性更高的同字段规则"（可能改变生效结果）
    const risky = [];
    const sameIdx = new Set(occ.map((o) => o.idx));
    for (let i = occ[0].idx + 1; i < last.idx; i += 1) {
      if (sameIdx.has(i)) continue;
      const r = flat[i];
      let hit = null;
      r.walkDecls((d) => { if (conflicts.some((c) => c.p === d.prop) && !hit) hit = d.prop; });
      if (hit && specificity(r.selector) > mySpec) risky.push({ line: r.source.start.line, prop: hit, sel: r.selector.slice(0, 44), spec: specificity(r.selector) });
    }
    // 安全论证：同选择器、同特异性 → 后写必胜 → 前面的同字段声明对**任何**元素都不可能生效
    // （不论中间有什么规则：中间规则要么特异性更高、要么更低/相同，都比不过"同特异性且更靠后"的末次出现）
    const deadSafe = conflicts.length > 0 && conflicts.every((c) => lastProps.has(c.p));

    // 纯粹的互补叠加：不冲突，但合并仍可能改变与中间规则的先后 → 仅当中间无同字段规则才合并
    let complementSafe = false;
    if (conflicts.length === 0) {
      let interHit = null;
      for (let i = occ[0].idx + 1; i < last.idx && !interHit; i += 1) {
        if (sameIdx.has(i)) continue;
        flat[i].walkDecls((d) => { if (propsAll.has(d.prop) && !interHit) interHit = d.prop; });
      }
      complementSafe = !interHit;
    }

    const tag = conflicts.length ? '[真冲突]' : '[互补叠加]';
    if (conflicts.length && !deadSafe) {
      skipTotal += 1;
      lines.push('  – ' + tag + ' ' + sel.slice(0, 52) + ' x' + occ.length + ' → 跳过（末次出现未声明该字段，删前面会改变生效结果）');
      continue;
    }
    if (!conflicts.length && !complementSafe) {
      skipTotal += 1;
      lines.push('  – ' + tag + ' ' + sel.slice(0, 52) + ' x' + occ.length + ' → 无需动（真互补，中间有同字段规则）');
      continue;
    }
    deadTotal += conflicts.length;
    mergedTotal += occ.length - 1;
    lines.push('  ✓ ' + tag + ' ' + sel.slice(0, 52) + ' x' + occ.length + ' 行' + occ.map((o) => o.r.source.start.line).join(',') +
      (conflicts.length ? '  删死声明: ' + conflicts.map((c) => c.p).join(',') : '  （字段互补，合并为一处）'));
    actions.push({ occ });
  }

  console.log('\n=== ' + file + ' ===');
  lines.length ? lines.forEach((l) => console.log(l)) : console.log('  （无重复）');
  if (!WRITE || !actions.length) continue;

  for (const { occ } of actions) {
    const target = occ[occ.length - 1].r;
    const acc = [];
    occ.forEach(({ r }) => r.walkDecls((d) => acc.push({ prop: d.prop, value: d.value, important: d.important })));
    target.removeAll();
    const seen = new Map();
    for (const d of acc) { if (seen.has(d.prop)) acc[seen.get(d.prop)] = d; else seen.set(d.prop, acc.indexOf(d)); }
    const final = [];
    const pos = new Map();
    for (const d of acc) { if (pos.has(d.prop)) final[pos.get(d.prop)] = d; else { pos.set(d.prop, final.length); final.push(d); } }
    for (const d of final) target.append({ prop: d.prop, value: d.value, important: d.important });
    occ.slice(0, -1).forEach(({ r }) => r.remove());
  }

  const after = effective(root);
  if (!mapsEqual(before, after)) { console.log('  !! 等价性校验失败，未写盘'); continue; }
  writeFileSync(file, root.toString());
  console.log('  ✅ 已落盘（生效声明表逐字一致）');
}
// ── 规则体内重复声明：CSS 后写必胜 → 保留最后一条，删掉前面被覆盖的死声明 ──
for (const file of FILES) {
  const src = readFileSync(file, 'utf8');
  const root = postcss.parse(src);
  const before = effective(root);
  let removed = 0;
  root.walkRules((r) => {
    const seen = new Map();
    const dup = [];
    r.walkDecls((d) => {
      if (seen.has(d.prop)) dup.push(seen.get(d.prop));
      seen.set(d.prop, d);
    });
    dup.forEach((d) => { d.remove(); removed += 1; });
  });
  if (!removed) continue;
  const after = effective(root);
  if (!mapsEqual(before, after)) { console.log('  !! ' + file + ' 规则体内去重后等价性失败，未写盘'); continue; }
  if (WRITE) writeFileSync(file, root.toString());
  console.log('  ✓ ' + file + ' 规则体内去重 ' + removed + ' 条（保留最后一条）' + (WRITE ? ' 已落盘' : ''));
  deadTotal += removed;
}

console.log('\n合计：合并 ' + mergedTotal + ' 处 · 删死声明 ' + deadTotal + ' 条 · 跳过 ' + skipTotal + ' 组' + (WRITE ? '' : '（dry-run，加 --write 落盘）'));
