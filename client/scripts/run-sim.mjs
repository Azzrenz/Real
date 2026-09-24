/**
 * 运行 scripts/sim-*.ts：esbuild 打包成 ESM 后交给 node 执行。
 *
 * 为什么需要它：store 是真 TypeScript + 真 import 图（zustand/services/tokens），
 * node 跑不了 .ts，而 tsc 编译整个项目太重、会带上 React 组件。
 * 这里只打一个 entry，`import.meta.env` 空定义（node 里没有 vite 的环境变量）。
 *
 * 用法：node scripts/run-sim.mjs [scripts/sim-chatstore.ts]
 */
import { build } from 'esbuild';
import path from 'node:path';
import fs from 'node:fs';
import { spawnSync } from 'node:child_process';

const target = process.argv[2] ?? 'scripts/sim-chatstore.ts';
const out = path.resolve('.sim-out.mjs');

await build({
  entryPoints: [target],
  bundle: true,
  platform: 'node',
  format: 'esm',
  target: 'node20',
  outfile: out,
  define: { 'import.meta.env': '{}' },
  logLevel: 'warning',
});

const r = spawnSync(process.execPath, [out], { stdio: 'inherit' });
fs.rmSync(out, { force: true });
process.exit(r.status ?? 1);
