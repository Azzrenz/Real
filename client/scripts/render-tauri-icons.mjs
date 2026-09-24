// 一次性脚本：按 icons.tsx 的 flame 路径，渲染 Tauri 全套图标（替换实心双层 PNG）
// 用 @resvg/resvg-js（纯 Rust 实现，无 native 依赖）
import { Resvg } from '@resvg/resvg-js';
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';

const ICON_DIR = 'src-tauri/icons';

// 与 icons.tsx 完全同构的 flame path（24 viewBox，线条风格）
const FLAME_PATH = 'M8.5 14.5A2.5 2.5 0 0 0 11 12c0-1.38-.5-2-1-3-1.072-2.143-.224-4.054 2-6 .5 2.5 2 4.9 4 6.5 2 1.6 3 3.5 3 5.5a7 7 0 1 1-14 0c0-1.153.433-2.294 1-3a2.5 2.5 0 0 0 2.5 2.5z';

// Real 主题色（与 styles.css --real 一致，暖橙）
const REAL = '#E8893F';

// 根据目标尺寸自适应 stroke（桌面图标需要更粗的线条才显眼）
function makeSvg(size) {
  const strokeW = size <= 32 ? 1.6 : size <= 128 ? 1.9 : 2.2;
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="${REAL}" stroke-width="${strokeW}" stroke-linecap="round" stroke-linejoin="round">
    <path d="${FLAME_PATH}"/>
  </svg>`;
}

function renderPng(name, size) {
  const svg = makeSvg(size);
  const resvg = new Resvg(svg, { fitTo: { mode: 'width', value: size }, background: 'transparent' });
  const png = resvg.render().asPng();
  const out = join(ICON_DIR, name);
  writeFileSync(out, png);
  console.log(`[OK] ${out} (${size}x${size}, ${png.length} B)`);
}

renderPng('icon.png', 512);          // 主源图标
renderPng('128x128@2x.png', 256);    // @2x = 实际 256
renderPng('128x128.png', 128);
renderPng('32x32.png', 32);

console.log('--- PNG done; .ico 由 tauri icon 从 icon.png 生成 ---');
