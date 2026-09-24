/**
 * 旁白与下一行之间的空白体检。
 *
 * 现象：界面上某些旁白下方多出一整行空白（实测 51px），另一些只有 22px，
 * 差值 29px 恰等于旁白一行的高度（15px * 1.95）。
 *
 * 假设：`.flow-narration` 带 `white-space: pre-line`（纯文本路径需要它来保留
 * 模型写的换行与清单）。而 markdown 路径的容器里是 marked 产出的 HTML ——
 * 那份 HTML 源码里的换行（`</p>\n`）会被 pre-line 当成"内容换行"渲染成空行。
 *
 * 这个脚本把两条路径的产物原样打印（JSON 转义），尾部有没有 \n 一眼可见。
 * 运行：node scripts/run-sim.mjs scripts/sim-narration-gap.ts
 */
import { marked } from 'marked';
// Same options as src/shared/lib/markdown.ts. renderMarkdown itself cannot run here:
// it sanitizes through DOMPurify, which needs a DOM.
marked.setOptions({ gfm: true, breaks: true });
const renderMarkdown = (raw: string) => marked.parse(raw, { async: false }) as string;
import {
  cleanSoft,
  cleanNarration,
  splitInlineListItems,
  stripFirstPerson,
  shapeNarration,
  needsMarkdown,
} from '../src/features/chat/narration/present';

const CASES: Array<{ tag: string; raw: string }> = [
  { tag: '① 行内加粗（界面有空行）', raw: '按方案执行：**截掉**evaluation部门。先摸清引用面与目录内容。' },
  { tag: '② 纯文本（界面有空行）', raw: '引用面极小：只有agent/mod.rs两行，其它都走兼容路径crate::agent::solver。移动很干净。先建立编辑资格并移文件。' },
  { tag: '③ 整句加粗（界面无空行）', raw: '**文件已移动。改模块声明三处。**' },
  { tag: '④ 带清单（markdown 路径）', raw: '待办三项：**\n- ① 补话术\n- ② 改判型\n- ③ 加回归台' },
];

for (const c of CASES) {
  console.log('='.repeat(64));
  console.log(c.tag);
  console.log('  原文      :', JSON.stringify(c.raw));

  const md = needsMarkdown(c.raw);
  console.log('  needsMarkdown :', md);

  if (md) {
    const html = renderMarkdown(c.raw);
    console.log('  marked 输出   :', JSON.stringify(html));
    console.log('  尾部是 \\n 吗 :', JSON.stringify(html.slice(-3)));
    console.log('  内部换行数    :', (html.match(/\n/g) ?? []).length);
  }

  const lead = shapeNarration(splitInlineListItems(cleanNarration(c.raw)), 300, false);
  console.log('  纯文本路径输出:', JSON.stringify(lead));
  const softened = splitInlineListItems(stripFirstPerson(cleanSoft(c.raw)));
  console.log('  markdown 路径输入:', JSON.stringify(softened));
}
