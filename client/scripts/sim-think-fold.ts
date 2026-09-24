/**
 * 思考内容的"折行"体检。
 *
 * 现象：思考窗口里出现「发。同时我应该看 main.rs 或 lib.rs 里的 `mod prompts;` 声明。
 * 嗯，先查目录。发。**真相查明**……」这样一整坨，读起来四句话粘在一起、语义接不上。
 *
 * 用真实库里捞出来的思考原文（run 9d828d7d）跑 foldPlainThinkingText，
 * 看它把「一句一段」揉成了什么。
 *
 * 运行：node scripts/run-sim.mjs scripts/sim-think-fold.ts
 */
import { foldPlainThinkingText } from '../src/features/thinking/parseThinking';

const RAW =
  '发。\n\n同时我应该看 main.rs 或 lib.rs 里的 `mod prompts;` 声明。\n\n嗯，先查目录。\n\n发。**真相查明**：`src/prompts/` 整个目录（6 个 .rs 文件）被删除了！';

const LONG =
  '先看引用面。\n\n只有两处引用。\n\n改动很小：删掉 evaluation 目录，把 solver 并进 orchestration。\n\n' +
  '开始前先确认工作区干净。\n\n' +
  '这样做的代价是 Replanner 那条路径要跟着改，先记下。\n\n' +
  '现在动手。\n\n' +
  '三处声明改完，顺手清掉 .bak 垃圾，然后编译验证。\n\n' +
  '编译报 E0583，排查中。';

for (const [tag, raw] of [['① 截图那段（4 段心里话）', RAW], ['② 更长的思考（8 段）', LONG]] as const) {
  console.log('='.repeat(70));
  console.log(tag);
  console.log('  原文换行结构 :', JSON.stringify(raw.match(/[^\n]*\n/g)?.slice(0, 20)));
  console.log('  原文行数     :', raw.split('\n').filter((l) => l.trim()).length, '行非空 /', raw.split('\n').length - 1, '个换行');
  const folded = foldPlainThinkingText(raw);
  console.log('  折叠后       :', JSON.stringify(folded));
  console.log('  折叠后行数   :', folded.split('\n').length);
  console.log('  ── 视觉 ──');
  console.log(folded);
  console.log();
}
