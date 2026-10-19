/** 手机端顶部状态条 —— 「看进度」的第一落点。
 *
 *  桌面大窗口里，过程本身就是界面：思考、工具、旁白三张卡并排铺开，信息是密集的。
 *  390px 上那套排版会变成一堵卡片墙，「看进度」退化成「往下滚很多屏」。手机的形态
 *  应该是**对话产品**：顶部一条常驻状态，告诉你「现在在干嘛 / 花了多少 / 最后一句」，
 *  细节收在下面，展开才看。
 *
 *  这个组件只读 store、不发请求、不碰事件流，因此不会影响 `lastSeq` / 回放的任何语义。
 */
import { useMemo, type ReactNode } from 'react';
import { useChatStore } from '../store/chatStore';
import { useT } from '../../../shared/i18n';
import type { Round } from '../types';

const EMPTY: never[] = [];

/** 取一段文字的第一行，压到一行以内。 */
function firstLine(text: string, max = 64): string {
  const line = (text ?? '').split('\n').map((s) => s.trim()).find((s) => s.length > 0) ?? '';
  return line.length > max ? `${line.slice(0, max - 1)}…` : line;
}

/** 当前这一步在做什么：从 live 轮的节点里倒着找最近的一个有意义的动作。 */
function currentStep(round: Round | null, tt: (zh: string) => string): string {
  if (!round) return '';
  const nodes = round.nodes ?? [];
  for (let i = nodes.length - 1; i >= 0; i -= 1) {
    const n = nodes[i];
    if (n.type === 'tool') {
      return (n.reason || '').trim() || firstLine(n.summary ?? '') || n.name;
    }
    if (n.type === 'think' && n.status === 'thinking') return tt('正在推理');
    if (n.type === 'narration') {
      const s = firstLine(n.text);
      if (s) return s;
    }
  }
  return round.notice ? firstLine(round.notice) : '';
}

/** 累计花费（元）：live 轮 + 所有已完成轮。
 *  以 `totals` 为准（它是后端对整轮的结算值）；只有 totals 缺失或被取消的轮才退回去加
 *  `usage`，否则同一笔钱会被算两遍。 */
function totalCost(rounds: Round[], live: Round | null): number {
  let cost = 0;
  const acc = (r: Round) => {
    const tc = r.totals?.cost_yuan;
    if (typeof tc === 'number' && tc > 0) {
      cost += tc;
      return;
    }
    for (const u of r.usage ?? []) if (typeof u.cost === 'number') cost += u.cost;
  };
  rounds.forEach(acc);
  if (live) acc(live);
  return cost;
}

export function MobileStatusBar({
  sessionId,
  dock,
}: {
  sessionId: string;
  /** 右侧插槽：手机上把 HeadDock（任务列表 / 历史提问）搬到这里，省掉一整条 .chat-head。 */
  dock?: ReactNode;
}) {
  const t = useT();
  const sess = useChatStore((s) => s.sessions[sessionId]);
  const confirm = useChatStore((s) => s.confirm);
  const rounds = sess?.rounds ?? EMPTY;
  const live = sess?.live ?? null;

  const waitingApproval = !!confirm && confirm.sessionId === sessionId;
  const lastRound = live ?? (rounds.length > 0 ? rounds[rounds.length - 1] : null);

  const step = useMemo(
    () => currentStep(live ?? (rounds.length > 0 ? rounds[rounds.length - 1] : null), t),
    [live, rounds, t],
  );
  const cost = useMemo(() => totalCost(rounds, live), [rounds, live]);

  let tone: 'approval' | 'running' | 'done' | 'idle' = 'idle';
  let label = t('待命');
  if (waitingApproval) {
    tone = 'approval';
    label = t('等你审批');
  } else if (live) {
    tone = 'running';
    label = t('执行中');
  } else if (lastRound) {
    const s = lastRound.status;
    if (s === 'cancelled') { tone = 'done'; label = t('已取消'); }
    else if (s === 'interrupted') { tone = 'done'; label = t('已中断'); }
    else if (s === 'error') { tone = 'approval'; label = t('执行失败'); }
    else { tone = 'done'; label = t('已完成'); }
  }

  const showCost = cost > 0;

  return (
    <div className={`m-status tone-${tone}`} role="status">
      <span className="m-status-dot" aria-hidden />
      <span className="m-status-label">{label}</span>
      {step && <span className="m-status-step" title={step}>{step}</span>}
      {showCost && <span className="m-status-cost">{t('¥{n}', { n: cost.toFixed(cost < 1 ? 4 : 2) })}</span>}
      {dock && <span className="m-status-dock">{dock}</span>}
    </div>
  );
}
