/**
 * Skill detail card placement health check: does it ever land on top of the popover?
 *
 * Why it exists: two fixed-position panels have to share one viewport. The old rule
 * clamped the card into the window edge, which parked it over the very list the
 * pointer was still moving through. Reading the code does not tell you at which
 * window widths that happens; sweeping every plausible geometry does.
 *
 * Usage: node scripts/run-sim.mjs scripts/sim-skill-card.ts
 */
import { placeCard } from '../src/features/chat/components/skillCardLayout';

// Mirrors SkillPicker.tsx: .skill-pop width and its clamping of the anchor.
const POP_W = 400;
const EDGE = 8;
const SIDEBAR = 261;

const WIDTHS = [900, 960, 1024, 1120, 1190, 1280, 1440, 1680, 1920];

let cases = 0;
let overlaps = 0;
let narrow = 0;

console.log('窗口宽  锚点左  弹窗区间        详情区间        宽   判定');
for (const w of WIDTHS) {
  for (let a = SIDEBAR; a <= w - POP_W - 20; a += 40) {
    const popLeft = Math.max(EDGE, Math.min(a, w - (POP_W + EDGE)));
    const pop = { left: popLeft, right: popLeft + POP_W };
    const { left, width } = placeCard(pop, w);
    const right = left + width;
    cases += 1;
    // Overlap: neither fully left of the popover nor fully right of it.
    const bad = !(right <= pop.left || left >= pop.right);
    const clipped = left < 0 || right > w;
    if (bad) overlaps += 1;
    if (width < 360) narrow += 1;
    const tag = bad
      ? '压住弹窗 ✗'
      : clipped
        ? '溢出 ✗'
        : right <= pop.left
          ? width < 360
            ? '左翻·收窄'
            : '左翻'
          : width < 360
            ? '收窄'
            : '右侧';
    console.log(
      `${String(w).padStart(5)}  ${String(popLeft).padStart(5)}  ` +
        `[${String(pop.left).padStart(5)},${String(pop.right).padStart(5)}]  ` +
        `[${String(left).padStart(5)},${String(right).padStart(5)}]  ` +
        `${String(width).padStart(4)}  ${tag}`,
    );
  }
}

console.log(`\n合计 ${cases} 组几何 · 压住弹窗 ${overlaps} · 收窄 ${narrow}`);
console.log(overlaps === 0 ? '判定：任何窗宽下详情卡都不遮挡弹窗 ✅' : '判定：仍有遮挡 ✗');
process.exit(overlaps === 0 ? 0 : 1);
