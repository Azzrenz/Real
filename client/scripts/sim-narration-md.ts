/**
 * 旁白 markdown 渲染链路体检：从模型原文走到 marked 产出的 HTML。
 *
 * 为什么需要它：旁白渲染分「清洗 → 判型 → 渲染」三段，分散在 present.ts /
 * markdown.ts 两处。只看代码判断不出某段语料在哪一步被毁掉，必须跑一遍看中间产物。
 *
 * 用法：node scripts/run-sim.mjs scripts/sim-narration-md.ts
 */
import { marked } from 'marked';
import { cleanSoft, cleanNarration, needsMarkdown, endAtColon, narrationPunct, splitInlineListItems } from '../src/features/chat/narration/present';

marked.setOptions({ gfm: true, breaks: true });

// 复刻 src/shared/lib/markdown.ts 的 normalizeBlockBoundaries（该函数未导出，
// 且 markdown.ts 依赖 DOMPurify，node 里 import 不了）。
function normalizeBlockBoundaries(text: string): string {
  return text
    .replace(/(^|\n)(#{1,6})([^\s#])/g, '$1$2 $3')
    .replace(/([^\n])\n(#{1,6}\s)/g, '$1\n\n$2')
    .replace(/([^\n])\n(\s*[-*+]\s)/g, '$1\n\n$2')
    .replace(/([^\n])\n(\s*\d+[.)、]\s)/g, '$1\n\n$2')
    .replace(/([\u4e00-\u9fa5])\s*(\d{1,2}[.、)）](?!\d))/g, '$1\n$2');
}

const CASES: Array<{ name: string; raw: string }> = [
  {
    name: 'A 正常分行表格',
    raw: '已定位三处带毒版本，逐一核对来源：\n| 题 | 位置 | 带毒状态 |\n|----|------|---------|\n| django-16595 | exam/+task/ | 天然带毒 |\n| E1 | exam/selfmade | 未植入 |',
  },
  {
    name: 'B 压成一行的表格',
    raw: '关键判定：| 题 | 位置 | 带毒 | |----|------|------| | django-16595 | exam/ | 天然带毒 |',
  },
  {
    name: 'C 单元格含括号说明',
    raw: '三处核对如下：\n| 题 | 环境 | 状态 |\n|----|------|------|\n| E1 | Python 3.14（已装） | 通过 |',
  },
  {
    name: 'D 单元格中文紧接数字加点',
    raw: '执行概览：\n| 阶段 | 结果 |\n|------|------|\n| 第1步 摸底 | 完成 |\n| 第2步 植入 | 进行中 |',
  },
  {
    name: 'E 代码块',
    raw: '用这条命令验证：\n```bash\ncargo test --all\n```',
  },
  {
    name: 'F 有序列表',
    raw: '接下来三步：\n1. 读题面\n2. 核对毒版\n3. 跑测试',
  },
];

const inTable = (s: string) => /(^|\n)\s*\|.*\|/.test(s);

for (const c of CASES) {
  const soft = cleanSoft(c.raw);
  const needs = needsMarkdown(soft);
  const mdIn = !/[|`]\s*$/.test(soft) ? endAtColon(soft) : soft;
  const norm = normalizeBlockBoundaries(mdIn);
  const html = marked.parse(norm, { async: false }) as string;
  const hasTable = html.includes('<table>');
  const hasPre = html.includes('<pre>');
  const hasOl = html.includes('<ol>');

  console.log('\n================ ' + c.name + ' ================');
  console.log('[原文换行数] ' + (c.raw.match(/\n/g) || []).length + '   原文含完整表格行: ' + inTable(c.raw));
  console.log('[needsMarkdown] ' + needs);
  console.log('[cleanSoft 后换行数] ' + (soft.match(/\n/g) || []).length + '   与原文换行数一致: ' + ((soft.match(/\n/g) || []).length === (c.raw.match(/\n/g) || []).length));
  if (soft !== c.raw) console.log('[cleanSoft 改动] ' + JSON.stringify(soft));
  console.log('[normalize 后换行数] ' + (norm.match(/\n/g) || []).length);
  if (norm !== mdIn) console.log('[normalize 改动] ' + JSON.stringify(norm));
  console.log('[渲染] <table>=' + hasTable + '  <pre>=' + hasPre + '  <ol>=' + hasOl);
  if (!hasTable && (c.name.startsWith('A') || c.name.startsWith('C') || c.name.startsWith('D'))) {
    console.log('[!! 未成表] ' + html.slice(0, 300).replace(/\n/g, '\\n'));
  }
  if (c.name.startsWith('C')) console.log('[C 表格内容] ' + (html.match(/<td>.*?<\/td>/g) || []).join(' '));
}

// 纯文本路径同样跑一遍，看它会不会把结构吃掉。
console.log('\n================ 纯文本路径（cleanNarration + 截断） ================');
const listCase = CASES[5].raw;
console.log('[原文]\n' + listCase);
console.log('[cleanNarration]\n' + cleanNarration(listCase));

console.log('\n================ 行内清单还原（splitInlineListItems） ================');
const INLINE: Array<{ name: string; raw: string }> = [
  {
    name: '用户实报的一行清单',
    raw: '未完成清单：-①编辑引用门禁✅已落地-②失败画像进Solver✅-③Replanner接失败画像✅-④guard_events死管道接活❌未做',
  },
  {
    name: '普通连字符（不得误伤）',
    raw: '路径在 D:/proj/server/src，组件叫 placeholder-rs，另有 a-b 与 3-2 这种写法。',
  },
  {
    name: '只有一个列表项（不触发）',
    raw: '先看第一处：-①编辑引用门禁，别的稍后。',
  },
];
for (const c of INLINE) {
  const out = splitInlineListItems(c.raw);
  console.log('\n---- ' + c.name + ' ----');
  console.log('[换行] ' + (c.raw.match(/\n/g) || []).length + ' → ' + (out.match(/\n/g) || []).length);
  console.log('[needsMarkdown 前→后] ' + needsMarkdown(c.raw) + ' → ' + needsMarkdown(out));
  if (out !== c.raw) console.log('[归一化后]\n' + out);
}

console.log('\n================ 二次判型：narrationPunct 是否吃掉换行 ================');
const SECOND: Array<{ name: string; raw: string }> = [
  { name: '多行有序列表', raw: '接下来三步：\n1. 读题面\n2. 核对毒版\n3. 跑测试' },
  { name: '行尾带逗号的多行列表', raw: '接下来三步：\n1. 读题面，\n2. 核对毒版，\n3. 跑测试' },
  { name: '表格 + 行尾句号', raw: '核对如下：\n| 题 | 状态 |\n|----|------|\n| E1 | 通过。|' },
  { name: '代码块', raw: '验证命令：\n```bash\ncargo test\n```' },
  { name: '两段之间空行', raw: '第一段结论。\n\n第二段补充。' },
];
for (const c of SECOND) {
  const punct = narrationPunct(c.raw);
  const settled = endAtColon(punct);
  const nlBefore = (c.raw.match(/\n/g) || []).length;
  const nlAfter = (settled.match(/\n/g) || []).length;
  console.log('\n---- ' + c.name + ' ----');
  console.log('[换行] ' + nlBefore + ' → ' + nlAfter + (nlAfter < nlBefore ? '   <<< 换行被吃掉' : ''));
  console.log('[needsMarkdown 原文] ' + needsMarkdown(c.raw) + '   [二次判型 settled] ' + needsMarkdown(settled));
  if (settled !== c.raw) console.log('[settled] ' + JSON.stringify(settled));
}
