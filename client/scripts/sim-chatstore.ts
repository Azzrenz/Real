// chatStore 事件归约仿真：重放真实会话形态（F5 回放 → 发送 → 执行中事件流 → 完成/出错）
// 验证：live 推进、lastSeq 游标、busy(=live 在场) 标志、多轮 seq 门卫不吞新事件、
//       轮次归属（H/I/J：异源终态事件不得落到别人那一轮）。
// 跑法：node scripts/run-sim.mjs（esbuild 打包后交给 node）
import { useChatStore } from '../src/features/chat/store/chatStore';

let pass = 0, fail = 0;
function assert(cond: boolean, msg: string) {
  if (cond) { pass++; console.log('  ok -', msg); }
  else { fail++; console.error('  FAIL -', msg); }
}
const S = 'sess-1';
const ev = (seq: number, kind: string, payload: Record<string, unknown>) =>
  ({ seq, kind, payload, ts: new Date(2026, 7, 28, 10, 0, seq).toISOString() });
const st = () => useChatStore.getState().sessions[S];
const st2 = () => useChatStore.getState().sessions[S2];
const cancelledCount = (sid: string) =>
  (useChatStore.getState().sessions[sid]?.rounds ?? []).filter((r) => r.status === 'cancelled').length;

// ---------- 场景 A：F5 后回放一段已完成的轮次（含 complete，terminalSeq 落位） ----------
console.log('A. F5 replay: completed round');
useChatStore.getState().replayEvents(S, [
  ev(1, 'thinking', { status: 'start', label: '深度思考' }),
  ev(2, 'reasoning', { text: '分析问题' }),
  ev(3, 'thinking', { status: 'end' }),
  ev(4, 'tool', { tools: [{ step_id: 'c1', name: 'fs_read', status: 'running' }] }),
  ev(5, 'tool', { tools: [{ step_id: 'c1', name: 'fs_read', status: 'success', duration_ms: 3 }] }),
  ev(6, 'message', { text: '第一轮结论。' }),
  ev(7, 'complete', { answer: '第一轮结论。', tool_count: 1, llm_calls: 1, cost_yuan: 0.01 }),
]);
assert(st()?.rounds.length === 1, 'A1 回放产出 1 个已完成轮次');
assert(st()?.live == null, 'A2 回放后无 live（busy=false）');
assert(useChatStore.getState().lastSeq[S] === 7, 'A3 lastSeq=7');
assert(useChatStore.getState().terminalSeq[S] === 7, 'A4 complete 落 terminalSeq=7');

// ---------- 场景 B：用户发送新消息（submitChat 路径）→ 执行中事件流 ----------
console.log('B. send -> live stream (auto-update path)');
useChatStore.getState().beginRun(S); // submitChat: await post 后调用
assert(useChatStore.getState().terminalSeq[S] === undefined, 'B1 beginRun 抬起 terminalSeq');
// SSE 逐事件到达（模拟 onmessage -> applyEvent）
useChatStore.getState().applyEvent(S, ev(8, 'thinking', { status: 'start', label: '深度思考' }));
assert(!!st()?.live, 'B2 首个事件创建 live');
assert(!!st()?.live, 'B3 首个事件建 live（busy=true）');
useChatStore.getState().applyEvent(S, ev(9, 'message', { text: '我先看一下配置文件：' }));
useChatStore.getState().applyEvent(S, ev(10, 'tool', { tools: [{ step_id: 'c2', name: 'fs_read', status: 'running' }] }));
assert(st()?.live?.nodes.length === 3, 'B4 live 节点推进（思考+旁白+工具卡）');
useChatStore.getState().applyEvent(S, ev(11, 'tool', { tools: [{ step_id: 'c2', name: 'fs_read', status: 'success', duration_ms: 5 }] }));
useChatStore.getState().applyEvent(S, ev(12, 'message', { text: '我先看一下配置文件：读取完成，结论如下。' }));
assert(
  (st()?.live?.nodes ?? []).filter((n) => n.type === 'narration').length === 2,
  'B5 累计全文只落增量旁白（不重复建节点）',
);
assert(useChatStore.getState().lastSeq[S] === 12, 'B6 lastSeq 游标推进到 12');
assert(!!st()?.live, 'B7 执行中 busy 保持 true（停止按钮/看门狗依赖）');

// ---------- 场景 C：执行中 complete 收尾 ----------
console.log('C. complete archives live');
useChatStore.getState().applyEvent(S, ev(13, 'complete', { answer: '结论如下。', tool_count: 1, llm_calls: 1 }));
assert(st()?.rounds.length === 2, 'C1 live 归档为第 2 轮');
assert(st()?.live === null, 'C2 live 清空（busy=false）');

// ---------- 场景 D：真实事故形态——多轮长会话（round1 complete -> round2 执行中 -> error） ----------
console.log('D. long session replay with mid-stream complete');
useChatStore.getState().replayEvents(S, [
  ev(20, 'message', { text: '第二轮开始。' }),
  ev(21, 'tool', { tools: [{ step_id: 'c3', name: 'db_query', status: 'success' }] }),
  ev(22, 'complete', { answer: '第二轮完成。' }),
  ev(23, 'message', { text: '第三轮进行中' }),
]);
assert(st()?.rounds.length === 1 && !!st()?.live, 'D1 complete 后的新轮事件建立 live 轮（不被 terminalSeq 吞掉）');
assert(useChatStore.getState().lastSeq[S] === 23, 'D2 lastSeq=23');

// ---------- 场景 E：cancel 后迟到事件 + 新发送恢复 ----------
console.log('E. cancel -> late events swallowed -> new run recovers');
useChatStore.getState().applyEvent(S, ev(24, 'message', { text: '第四轮执行中' })); // live 建立
useChatStore.getState().archiveLive(S, 'cancelled');
assert(useChatStore.getState().archived[S] === true, 'E1 archiveLive 置 archived');
useChatStore.getState().applyEvent(S, ev(25, 'complete', { answer: '迟到的完成' })); // 迟到
assert(st()?.rounds.length === 2 && !st()?.live, 'E2 迟到 complete 不产生幻影轮');
useChatStore.getState().beginRun(S);
useChatStore.getState().applyEvent(S, ev(26, 'message', { text: '新一轮正常推进' }));
assert(!!st()?.live, 'E3 新一轮事件恢复流动');

// ---------- 场景 F：busy 标志污染检测（stale st spread） ----------
console.log('F. busy flag integrity during streaming');
const runningSnapshots: boolean[] = [];
for (let i = 27; i <= 40; i++) {
  useChatStore.getState().applyEvent(S, ev(i, 'message', { text: `累计文本 ${i}` }));
  runningSnapshots.push(!!st()?.live);
}
assert(runningSnapshots.every(Boolean), 'F1 流式全程 busy 不被污染为 false');
assert((st()?.live?.nodes.length ?? 0) >= 1, 'F2 message 节点持续推进');

// ---------- 场景 G：多轮收敛——complete 后【不重建连接】下一轮事件自动续流 ----------
console.log('G. multi-round convergence: complete -> next round flows on same stream');
useChatStore.getState().applyEvent(S, ev(50, 'thinking', { status: 'start', label: '深度思考' }));
useChatStore.getState().applyEvent(S, ev(51, 'message', { text: '第二轮收敛。' }));
useChatStore.getState().applyEvent(S, ev(52, 'complete', { answer: '第二轮收敛。' }));
assert(st()?.live === null, 'G1 首轮 complete 后 busy=false');
// 不调用 beginRun（后端循环自动进入下一轮），直接流来下一轮事件
useChatStore.getState().applyEvent(S, ev(53, 'thinking', { status: 'start', label: '深度思考' }));
useChatStore.getState().applyEvent(S, ev(54, 'message', { text: '第三轮。' }));
assert(!!st()?.live, 'G2 下一轮事件在【同一流】上自动重建 live（前端不得在 complete 时关流）');
assert(useChatStore.getState().lastSeq[S] === 54, 'G3 lastSeq 续推到 54');

// ---------- 场景 H：轮次归属——取消事件迟到，不得落到新消息那一轮（2026-09-10 事故） ----------
// 实况时序：点停止 → 立刻发新消息（新一轮起跑）→ 旧 run 到下一个边界才收手，
// 它的 cancelled 事件几秒后才落库。此前靠"事件时间戳比 live 早"猜归属，迟到事件
// 比新轮的 startedAt 更新 → 判不出异源 → 新消息那一轮被盖上"已取消"。
// （用户原话：一发新消息，取消下面又回复一个已取消，等于有两个竞争的。）
console.log('H. late cancel of a foreign run must not settle the new round');
const S2 = 'sess-2';
const soon = (ms: number) => new Date(Date.now() + ms).toISOString();
const e2 = (seq: number, kind: string, payload: Record<string, unknown>, ts?: string) =>
  ({ seq, kind, payload, ts });

useChatStore.getState().applyEvent(S2, e2(1, 'message', { text: '第一轮执行中', run_id: 'A' }));
assert(st2()?.live?.runId === 'A', 'H1 首条事件把 live 绑到 run A');
useChatStore.getState().archiveLive(S2, 'cancelled'); // 用户点停止：乐观归档
assert(cancelledCount(S2) === 1, 'H2 此刻只有 1 张已取消轮');
useChatStore.getState().beginRun(S2, 'B'); // 新消息：send 回执给 run B
assert(st2()?.live?.runId === 'B', 'H3 新一轮出生即绑 run B');
// 旧 run A 的取消事件迟到：时间戳比新轮 startedAt 更新（时间戳判据必然判错）
useChatStore.getState().applyEvent(
  S2,
  e2(2, 'cancelled', { message: '任务已被取消', run_id: 'A' }, soon(3000)),
);
assert(st2()?.live?.runId === 'B', 'H4 迟到的取消没杀掉新一轮（live 仍在、仍属 run B）');
assert(cancelledCount(S2) === 1, 'H5 已取消仍只有 1 张（没有两个竞争的提示）');
useChatStore.getState().applyEvent(S2, e2(3, 'message', { text: '新一轮推进', run_id: 'B' }));
assert((st2()?.live?.nodes.length ?? 0) > 0, 'H6 run B 的事件照常落在自己那一轮');

// ---------- 场景 I：异源 complete 不得把旧轮答案盖到新轮 ----------
console.log('I. late complete of a foreign run must not settle the new round');
useChatStore.getState().applyEvent(
  S2,
  e2(4, 'complete', { answer: '旧轮的答案', run_id: 'A' }, soon(4000)),
);
assert(!!st2()?.live, 'I1 异源 complete 不归档新轮');
assert(
  (st2()?.rounds ?? []).every((r) => r.answer !== '旧轮的答案'),
  'I2 旧轮答案没有串到新轮',
);

// ---------- 场景 J：取消轮不伪造状态文案当回复 ----------
console.log('J. cancelled round keeps answer empty (no fake reply under the head)');
useChatStore.getState().applyEvent(S2, e2(5, 'cancelled', { message: '任务已被取消', run_id: 'B' }, soon(5000)));
const bRound = (st2()?.rounds ?? [])[(st2()?.rounds ?? []).length - 1];
assert(bRound?.status === 'cancelled', 'J1 本轮自己的取消事件正常收场');
assert(!!bRound?.errorText, 'J2 状态文案进 errorText 留痕');
// 新一轮没有任何模型输出就被取消 → answer 必须为空（不出现"已取消"这种假回复）
useChatStore.getState().beginRun(S2, 'C');
useChatStore.getState().applyEvent(S2, e2(6, 'thinking', { status: 'start', label: '深度思考', run_id: 'C' }));
useChatStore.getState().applyEvent(S2, e2(7, 'cancelled', { message: '任务已被取消', run_id: 'C' }, soon(6000)));
const cRound = (st2()?.rounds ?? [])[(st2()?.rounds ?? []).length - 1];
assert(cRound?.status === 'cancelled' && !cRound?.answer, 'J3 空轮取消 answer 为空（不伪造回复）');
assert(cancelledCount(S2) === 3, 'J4 三次取消各留一张轮（不多不少）');

console.log(`\n==== ${pass} passed, ${fail} failed ====`);
process.exit(fail > 0 ? 1 : 0);
