// 真实链路诊断：真实后端 + 真实 SSE + 真实 chatStore（复刻 ChatPanel 挂载时序）
import { useChatStore } from '../src/features/chat/store/chatStore';

const BASE = 'http://127.0.0.1:8943';
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function main() {
  // 1) 建会话
  const created = await (await fetch(`${BASE}/api/sessions`, {
    method: 'POST', headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ title: 'freeze-diag' }),
  })).json();
  const sid = created.session?.id;
  console.log('session:', sid);

  // 2) 复刻挂载时序：先 replay 对齐 lastSeq
  const detail = await (await fetch(`${BASE}/api/sessions/${sid}`)).json();
  if (detail.events?.length) useChatStore.getState().replayEvents(sid, detail.events);
  console.log('replay done, lastSeq =', useChatStore.getState().lastSeq[sid] ?? 0);

  // 3) 发消息（后台 spawn agent）
  await fetch(`${BASE}/api/sessions/${sid}/chat`, {
    method: 'POST', headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ content: '只回复两个字：收到' }),
  });
  console.log('chat posted');

  // 4) 开真实 EventSource（after=lastSeq）
  const after = useChatStore.getState().lastSeq[sid] ?? 0;
  const es = new EventSource(`${BASE}/api/sessions/${sid}/events?after=${after}`);
  es.onmessage = (e) => {
    try {
      const ev = JSON.parse(e.data);
      useChatStore.getState().applyEvent(sid, ev);
    } catch (err) { console.error('BAD FRAME:', err); }
  };
  es.onerror = () => console.log('[es] error, readyState=', es.readyState);
  es.onopen = () => console.log('[es] OPEN after=', after);

  // 5) 每秒盯 lastSeq 与 live 状态
  const t0 = Date.now();
  while (Date.now() - t0 < 75_000) {
    await sleep(1000);
    const s = useChatStore.getState();
    const st = s.sessions[sid];
    const el = st?.live?.nodes?.length ?? 0;
    const al = st?.live?.answer?.length ?? 0;
    console.log(`t+${((Date.now() - t0) / 1000).toFixed(0)}s lastSeq=${s.lastSeq[sid] ?? 0} running=${st?.running} liveNodes=${el} rounds=${st?.rounds.length} answerLen=${al}`);
    if (!st?.running && st?.rounds.length > 0) break;
  }
  es.close();
  console.log('DONE');
  process.exit(0);
}
main().catch((e) => { console.error('FATAL', e); process.exit(1); });
