// 回归：会话「静音」事故 —— 缓存续点被归 0 ⇒ SSE 从 seq 0 全量重放 ⇒ 历史里的 error
// 把整个会话标成 archived ⇒ applyEvent 入口吞掉其后全部事件 ⇒ AI 正文整段消失，
// 面板只剩用户气泡（用户气泡来自 messages 表，独立渲染，所以"半截对话"看起来像半截）。
//
// 实况：会话 87485b26（Colipt），首条 error 是 #612040（全流 2.4% 处），
// 其后 145791 条事件（97.2%）全被入口闸门吞掉。
//
// 本仿真锁死三件事：
//   P  deriveResumePoints —— 续点（SSE 从哪连）不得由 terminalSeq 单源派生，且绝不倒退；
//   S  error 归档的是「一轮」，不得归档【整个会话】；并附对照，证明闸门确实存在；
//   E  端到端形态：缓存命中 + lastSeq=0 + 历史含早期 error ⇒ 正文必须仍然可见。
//
// 跑法：node scripts/run-sim.mjs scripts/sim-resume-gate.ts

import { useChatStore } from '../src/features/chat/store/chatStore';
import { deriveResumePoints } from '../src/features/chat/store/persist';

let pass = 0;
let fail = 0;
function assert(cond: boolean, msg: string) {
  if (cond) {
    pass++;
    console.log('  ok -', msg);
  } else {
    fail++;
    console.error('  FAIL -', msg);
  }
}
const ev = (seq: number, kind: string, payload: Record<string, unknown>) => ({
  seq,
  kind,
  payload,
  ts: new Date(2026, 8, 16, 18, 0, seq % 60).toISOString(),
});
const S = (id: string) => useChatStore.getState().sessions[id];
const seqOf = (id: string) => useChatStore.getState().lastSeq[id] ?? 0;

// ---------- P：缓存 → 续点（SSE 起点）的推导 ----------
console.log('P. deriveResumePoints: 续点不得从 terminalSeq 单源派生');
assert(deriveResumePoints({ terminalSeq: { s1: 42 } }).s1 === 42, 'P1 只有 terminalSeq → 采用它');
assert(
  deriveResumePoints({ lastSeq: { s1: 900 } }).s1 === 900,
  'P2 terminalSeq 缺失（beginRun 已清、流未起的事故窗口）→ 仍保留 lastSeq，绝不归 0 ★',
);
assert(
  deriveResumePoints({ lastSeq: { s1: 900 }, terminalSeq: { s1: 42 } }).s1 === 900,
  'P3 两源都有 → 取较大值（lastSeq 领先时不得被 terminalSeq 拉回去）',
);
assert(
  deriveResumePoints({ lastSeq: { s1: 7 }, terminalSeq: { s1: 42 } }).s1 === 42,
  'P4 两源都有 → 取较大值（terminalSeq 领先时采用它）',
);
assert(Object.keys(deriveResumePoints({})).length === 0, 'P5 两源皆缺 → 不造续点键（由上层判定走 replay）');
const mixed = deriveResumePoints({ lastSeq: { s1: 5 }, terminalSeq: { s2: 9 } });
assert(mixed.s1 === 5 && mixed.s2 === 9, 'P6 不同会话的键都不丢');

// ---------- S：error 只归档「一轮」，不归档「整个会话」 ----------
console.log('S. error 归档的是一轮，不是整个会话');
const A = 'sess-error-gate'; // 事故形态：SSE 从 0 全量重放（replaying=false），中段一条 error
useChatStore.getState().applyEvent(A, ev(1, 'message', { text: '第一轮执行中', run_id: 'R1' }));
useChatStore.getState().applyEvent(A, ev(2, 'tool', { tools: [{ step_id: 't1', name: 'fs_read', status: 'running' }] }));
assert(!!S(A)?.live, 'S1 首条事件建立 live');
useChatStore.getState().applyEvent(A, ev(3, 'error', { error: '任务失败：模型超时', run_id: 'R1' }));
assert(useChatStore.getState().archived[A] !== true, 'S2 历史 error 不再把【会话】标为 archived ★');
assert(
  (S(A)?.rounds ?? []).some((r) => r.status === 'error'),
  'S3 error 那一轮仍正常落库（status=error，该轮的失败不被吞掉）',
);
assert(S(A)?.live == null, 'S4 error 归档该轮的 live（busy 收尾）');
// error 之后的事件——实况里占 97.2%，AI 正文全在里面
// 先来一条【迟到的 complete】（无 live 时）：必须被 lastRound.status='error' 挡住，不建幽灵轮
useChatStore.getState().applyEvent(A, ev(4, 'complete', { answer: '迟到的完成' }));
assert(
  (S(A)?.rounds ?? []).length === 1 && !S(A)?.live,
  'S5 error 后迟到的 complete 不建幽灵轮（不靠 archived，靠 cost.ts 的 lastRound.status 判据）',
);
useChatStore.getState().applyEvent(A, ev(5, 'thinking', { status: 'start', label: '深度思考' }));
useChatStore.getState().applyEvent(A, ev(6, 'message', { text: 'error 之后的正文。' }));
assert(!!S(A)?.live, 'S6 error 之后的正文仍被处理（新 live 重建）★');
assert(seqOf(A) === 6, 'S7 lastSeq 推进到 6');

// ---------- S*：对照——archived 为真时入口确实吞事件（证明上面不是恒真断言） ----------
console.log('S*. 对照：archived 为真 ⇒ 后续事件被入口吞掉');
const B = 'sess-archived-control';
useChatStore.getState().applyEvent(B, ev(1, 'message', { text: '开始', run_id: 'RB' }));
useChatStore.getState().archiveLive(B, 'cancelled'); // 真实取消路径仍然置 archived（未被本次修复波及）
assert(useChatStore.getState().archived[B] === true, 'S8 取消路径仍置 archived（修复没有误伤取消语义）');
useChatStore.getState().applyEvent(B, ev(2, 'message', { text: '取消之后迟到的事件' }));
assert(seqOf(B) !== 2, 'S9 archived 为真 ⇒ 事件被入口吞掉（闸门确实存在，故 S2/S6 才有区分度）');

// ---------- E：端到端形态（缓存命中 + lastSeq=0 + 历史早期 error） ----------
console.log('E. 端到端：早期 error 之后的正文必须可见');
const C = 'sess-87485b26-like';
const flow: ReturnType<typeof ev>[] = [
  ev(1, 'message', { text: '第一轮执行中', run_id: 'R1' }),
  ev(2, 'tool', { tools: [{ step_id: 't1', name: 'fs_read', status: 'running' }] }),
  ev(3, 'error', { error: '任务失败：模型超时', run_id: 'R1' }), // ← 对应实况 #612040
];
for (let i = 4; i <= 30; i++) flow.push(ev(i, 'message', { text: `error 之后的正文增量 ${i}` }));
flow.push(ev(31, 'complete', { answer: '重放结束后的答复正文。', tool_count: 1, llm_calls: 1, cost_yuan: 0.01 }));
for (const e of flow) useChatStore.getState().applyEvent(C, e);
const roundsC = S(C)?.rounds ?? [];
const tailNodes = roundsC[roundsC.length - 1]?.nodes ?? [];
const tailText = tailNodes.reduce((n, x) => n + String((x as { text?: string }).text ?? '').length, 0);
const answerLen = roundsC.reduce((n, r) => n + (r.answer?.length ?? 0), 0);
assert(useChatStore.getState().archived[C] !== true, 'E1 整段重放后会话未被静音 ★');
assert(roundsC.length === 2, 'E2 error 轮 + 收尾轮都在（不丢轮）');
assert(tailText > 0, `E3 error 之后那一轮携带正文（节点文本 ${tailText} 字，不再被入口吞掉）★`);
assert(answerLen > 0, 'E4 收尾轮的 AI 正文非空（不再只剩用户气泡）★');
assert(seqOf(C) === 31, 'E5 lastSeq 推到 31');

console.log(`\n结果：${pass} 通过 / ${fail} 失败`);
process.exit(fail > 0 ? 1 : 0);
