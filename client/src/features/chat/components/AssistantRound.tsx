
import { memo, useState, useEffect} from 'react';
import { Icon } from '../../../shared/ui/icons';
import { DeepThinkRow } from '../../thinking/DeepThinkRow';
import { ToolCardView } from '../../tools/components/ToolCard';import { useSettings } from '../../settings/store/settingsStore';
import type { ChatNode, Round, ThinkRow } from '../store/chatStore';
import { fmtTime, fmtDuration, prettyModel } from '../../../shared/lib/format';
import { useT } from '../../../shared/i18n';
import {
  cleanNarration, cleanSoft, stripFirstPerson, shapeNarration, narrationBudget,
  hasPlanStructure, stripRedundantTail, stablePart, endAtColon, needsMarkdown,
  splitInlineListItems,
} from '../narration/present';
import { hasBlockMarkdown } from '../narration/pipeline';
import { pickTitle } from '../../tools/lib/purpose';
import { enTermify, handleProseClick, renderMarkdown } from '../../../shared/lib/markdown';
import { USE_DEEPTHINK_V2 } from '../config';
import { ThinkRowView } from './ThinkRowLegacy';
import { NarrationLine } from './NarrationLine';
import { CostLine } from './CostLine';
import { isBalanceError, PayErrorCard } from './PayErrorCard';
import { EvolutionCardView } from './EvolutionCardView';
import { UserBubbleText } from './UserBubbleText';

export const AssistantRound = memo(function AssistantRound({
  round,
  running,
  live,
  phaseHint,
  onRegen: _onRegen,
}: {
  round: Round;
  running?: boolean;
  live?: boolean;
  phaseHint?: string;
  sessionId?: string;
  onRegen?: () => void;
}) {
  const nodes: ChatNode[] = round.nodes ?? [];
  const [actOk, setActOk] = useState<'like' | 'dislike' | 'copy' | null>(null);
  const flashOk = (which: 'like' | 'dislike' | 'copy') => {
    setActOk(which);
    window.setTimeout(() => setActOk((a) => (a === which ? null : a)), 1400);
  };
  const doCopy = () => {
    void navigator.clipboard.writeText(round.answer ?? '').then(() => flashOk('copy'));
  };
  const settingsModel = useSettings((s) => s.model);
  const t = useT();
  const modelLabels = useSettings((s) => s.modelLabels);
  const currentModel = round.totals?.model || settingsModel;
 const aborted = round.status === 'cancelled' || round.status === 'error' || round.status === 'interrupted';
 const [showProcess, setShowProcess] = useState(aborted);
  useEffect(() => {
    if (aborted) setShowProcess(true);
  }, [aborted]);
  const done = !live && !running;
  const toolCount = nodes.filter((n) => n.type === 'tool').length;
  const thinkCount = nodes.filter((n) => n.type === 'think').length;
  const hasAnswer = !!(round.answer && round.answer.trim());
  const doneLabel = round.status === 'cancelled' ? t('已取消')
    : round.status === 'interrupted' ? t('已中断')
    : round.status === 'error' ? t('执行失败') : t('已完成');
  // Duration is stamped once, when the round ends (metrics.runMs). Rounds rebuilt from
  // stored messages carry no such value — showing nothing beats showing a fake 0.
  const runMs = round.metrics?.runMs;
  const metaParts = [
    toolCount > 0 ? t('{n} 个工具', { n: toolCount }) : '',
    thinkCount > 0 ? t('{n} 次思考', { n: thinkCount }) : '',
    hasAnswer ? t('回复就绪') : '',
    runMs != null ? t('耗时 {d}', { d: fmtDuration(runMs) }) : '',
  ].filter(Boolean);
  const hasCost = round.status !== 'running'
    && (round.totals.cost_yuan != null || (round.totals.llm_calls ?? 0) > 0);
  const usageAgg = aborted && round.usage.length > 0
    ? round.usage.reduce(
        (a, c) => ({
          tokens: a.tokens + c.input + c.output,
          cost: a.cost + (typeof c.cost === 'number' ? c.cost : 0),
          calls: a.calls + 1,
        }),
        { tokens: 0, cost: 0, calls: 0 },
      )
    : null;
  // 执行中（live = 还在跑）不显示脚注：复制按钮 / 模型名 / 时间 / 消耗——
  // 干活时这几样一直杵在回合底部是纯噪音，等回合结束（live 落回 false）再出现。
  const showFoot = !live && (!!round.answer || hasCost || !!usageAgg);

  const runningTool = nodes.some((n) => n.type === 'tool' && n.status === 'running');
  const openThink = nodes.find((n) => n.type === 'think' && n.status === 'thinking') as ThinkRow | undefined;
  const activityText = (() => {
    if (!live) return null;
    if (runningTool) return null;
    if (openThink) return (openThink.body ?? '').length > 0 ? null : t('正在推理中');
    if (nodes.length === 0) return t(phaseHint ?? '正在准备…');
    return null;
  })();

  const planLines = (text: string): string =>
    text.replace(
      /[；;]\s*(?=[0-9０-９]+[.、．)）]|[①②③④⑤⑥⑦⑧⑨⑩]|[一二三四五六七八九十]+[、．．)）]|第[一二三四五六七八九十]+步)/g,
      '\n',
    );

  const renderNodes = () => {
    const baseBudget = narrationBudget({
      reasoningChars: nodes.reduce((n, x) => (x.type === 'think' ? n + (x.body?.length ?? 0) : n), 0),
      toolCount: nodes.reduce((n, x) => (x.type === 'tool' ? n + 1 : n), 0),
      thinkCount: nodes.reduce((n, x) => (x.type === 'think' ? n + 1 : n), 0),
    });
  const absorbed = new Set<string>();
  const purposeByToolIdx = new Map<number, string>();
  for (let k = 0; k < nodes.length - 1; k++) {
    const n = nodes[k];
    const next = nodes[k + 1];
    if (next.type !== 'tool') continue;
    if (next.name !== 'run' && next.name !== 'verify') continue;
    const t =
      n.type === 'narration'
        ? n.text.trim()
        : n.type === 'think'
          ? (n.body ?? '').slice(-300).trim()
          : '';
    // needsMarkdown, not hasBlockMarkdown: a narration carrying inline markers must not be
    // folded into a tool-row title either - the title renders as plain text, so ** would
    // show up literally.
    if (!t || needsMarkdown(t)) continue;
    if (/[。！？.!?\n]$/.test(t)) {
      const segs = t.split(/[，,；;：:。！？!?]|——/).map((s) => s.trim()).filter(Boolean);
      const pick = pickTitle(segs);
      if (pick) purposeByToolIdx.set(k + 1, pick);
      continue;
    }
    const parts = t.split(/[，,；;：:]/).map((s) => s.trim()).filter(Boolean);
    if (parts.length <= 1) {
      // 2026-09-23 修：**工具已自带 reason 时不得吸收旁白**。
      // 吸收来的文字只在 ToolCard 里当"兜底标题"（usableReason 优先，见 ToolCard.tsx:132-135）；
      // 若该工具 reason ≥ 8 字，这份文字一个字都进不了界面 —— 旁白被 absorbed 吞掉、
      // 文字又没派上用场，用户看到的就"这条旁白凭空消失"。
      // 判据必须与取用条件对齐：工具有可用 reason ⇒ 只设标题兜底，不吸收，旁白照常渲染。
      if (((next.reason ?? '').trim()).length >= 8) {
        purposeByToolIdx.set(k + 1, t);
        continue;
      }
      if (t.replace(/\s+/g, '').length > 32) continue;
      absorbed.add(String((n as { id?: string }).id ?? `i${k}`));
      purposeByToolIdx.set(k + 1, t);
    } else {
      const best = pickTitle(parts);
      if (best) purposeByToolIdx.set(k + 1, best);
    }
  }
  for (const [idx, pur] of [...purposeByToolIdx]) {
    const baseNode = nodes[idx];
    const base = baseNode && baseNode.type === 'tool' ? baseNode.name : '';
    if (!base) continue;
    for (let m = idx + 1; m < nodes.length; m++) {
      const nd = nodes[m];
      if (nd.type === 'tool' && nd.name === base && !purposeByToolIdx.has(m)) {
        purposeByToolIdx.set(m, pur);
      } else break;
    }
  }
    return (
      <>
        {nodes.map((n, i) => {
          if (n.type === 'tool') {
            let repeatNo = 1;
            for (let k = i - 1; k >= 0; k--) {
              const p = nodes[k];
              if (p.type === 'tool' && p.name === n.name) repeatNo += 1;
              else break;
            }
            return (
              <ToolCardView
                key={`t-${n.stepId ?? `i${i}`}`}
                tool={n}
                repeatNo={repeatNo}
                purposeOverride={purposeByToolIdx.get(i)}
              />
            );
          }
          if (n.type === 'narration') {
            if (absorbed.has(String(n.id ?? `i${i}`))) return null;
            // 2026-09-16 修复：删掉「思考块进行中 ⇒ 旁白不渲染」的 gate（原 `live && gateByThinkIdx >= 0`
            // 两处 return null）。它按「思考与旁白一一交错」的模型设计：思考还在流时先把旁白藏住，
            // 等思考结束再放出来，免得读的人被来回插的字打断。
            // 但同批发齐之后，一轮里思考块往往到最末才结束 —— gate 于是整轮生效，实时跑的时候
            // 一条旁白都看不见（落库有、回看也有，唯独正在跑时没有）。读起来正是「模型不写旁白了」。
            // 旁白既已到达 nodes，就照显；遮蔽留白交给排版，不交给丢弃。
            const nextNode = nodes[i + 1];
            const nextPurpose = nextNode && nextNode.type === 'tool'
              ? ((nextNode.reason ?? '').trim() || purposeByToolIdx.get(i + 1) || '')
              : '';
            const full = nextPurpose && !hasBlockMarkdown(n.text)
              ? stripRedundantTail(n.text, nextPurpose)
              : n.text;
            // One clean, one decision - both render paths consume the same cleaned input.
            // hasBlockMarkdown is deliberately NOT the gate here: it only recognises block
            // constructs, so narration carrying inline bold or inline code fell through to
            // the plain-text path, where the markers were stripped and the emphasis vanished.
            // needsMarkdown also keys on inline markers, and cleanSoft leaves them intact.
            // Person voice is stripped here too, matching the plain-text path (which does it
            // inside shapeNarration) - otherwise a leading first-person pronoun would survive
            // on markdown narration only.
            const softened = splitInlineListItems(stripFirstPerson(cleanSoft(full)));
            if (needsMarkdown(softened)) {
              const md = softened.replace(/(^|\n)(#{1,6})([^\s#])/g, '$1$2 $3');
              // Same closing rule as the plain-text path. endAtColon itself skips text that
              // ends in a table row or a code fence, so that guard lives in exactly one place.
              const out = live ? md : endAtColon(md);
              return (
                <div key={`n-${n.id ?? i}`} className="flow-narration">
                  <div className="narration-block prose" onClick={handleProseClick} dangerouslySetInnerHTML={{ __html: enTermify(renderMarkdown(out)) }} />
                </div>
              );
            }
            // live 期间只显"已完结的句子"是防抖 —— 但模型常写**不带句末标点**的短旁白，
            // 那种情况下 stablePart 返回空，整条旁白在跑的过程中一个字都不出现（回合结束才补上），
            // 实时读起来同样是"模型不写旁白"。没有完结句时退化为显示原文：宁可见半句，不留白。
            const raw = live ? (stablePart(full) || full) : full;
            if (!raw) return null;
            const budget = hasPlanStructure(raw) ? Math.max(baseBudget, 800) : baseBudget;
            const lead = shapeNarration(splitInlineListItems(cleanNarration(raw)), budget, !live);
            const listed = planLines(lead);
            if (!lead) return null;
            const narrKey = `n-${n.id ?? i}`;
            return (
              <div key={narrKey} className="flow-narration">
                <NarrationLine text={listed} live={live === true} />
              </div>
            );
          }
          if (n.type === 'think') {
            return USE_DEEPTHINK_V2 ? (
              <DeepThinkRow key={`k-${n.id}`} row={n} />
            ) : (
              <ThinkRowView key={`k-${n.id}`} row={n} running={running} />
            );
          }
          if (n.type === 'user_bubble') {
            const cancelledBubble = (n as { cancelled?: boolean }).cancelled === true;
            return (
              <div key={`ub-${n.id}`} className="flow-interjection">
                <div
                  className={`msg msg-user${n.text.length > 300 ? ' msg-user-long' : ''}${cancelledBubble ? ' msg-user-cancelled' : ''}`}
                  title={cancelledBubble ? t('任务已停止，这条插话未被执行') : undefined}
                >
                  {cancelledBubble && <span className="bubble-cancel-badge">{t('已取消')}</span>}
                  <UserBubbleText text={n.text} />
                </div>
              </div>
            );
          }
          if (n.type === 'evo') {
            return <EvolutionCardView key={`ev-${n.id}`} card={n} />;
          }
          return null;
        })}
      </>
    );
  };

  return (
    <div className="assistant-unit" data-seq={round.seq} data-live={live ? 'true' : undefined}>
      <div className="assistant-unit-head">
        <Icon name="flame" size={18} className="au-flame" />
        <span className="au-name">Real</span>
      </div>
      {done ? (
        <div className="done-process">
          <button
            className="done-process-head"
            onClick={() => setShowProcess((v) => !v)}
            aria-expanded={showProcess}
            title={showProcess ? t('收起执行过程') : t('展开执行过程')}
          >
            <Icon name="chevron" size={14} className={`flow-row-caret${showProcess ? ' open' : ''}`} />
            <span className="done-process-label">{doneLabel}</span>
            {metaParts.length > 0 && (
              <span className="done-process-meta">{metaParts.join(' · ')}</span>
            )}
          </button>
          {showProcess && (
            <div className="done-process-body">
              {renderNodes()}
            </div>
          )}
        </div>
      ) : (
        <>
          {renderNodes()}
          {round.notice && (
            <div className="live-activity" role="status" title={round.notice}>
              <span className="live-activity-dot" aria-hidden />
              <span className="live-activity-text">{round.notice}</span>
            </div>
          )}
          {activityText && (
            <div className="live-activity">
              <span className="live-activity-dot" aria-hidden />
              <span className="live-activity-text">{activityText}</span>
            </div>
          )}
        </>
      )}
      {!live && round.errorText && isBalanceError(round.errorText) ? (
        <PayErrorCard message={round.errorText} />
      ) : (
        !live && round.answer && (
          <div
            className="answer-block prose"
            onClick={handleProseClick}
            dangerouslySetInnerHTML={{ __html: enTermify(renderMarkdown(round.answer)) }}
          />
        )
      )}
      {showFoot && (
        <div className="unit-foot">
          {round.answer && (
            <div className="answer-actions">
              <button
                className={`answer-act${actOk === 'copy' ? ' ok' : ''}`}
                onClick={doCopy}
                title={t('复制')}
                aria-label={t('复制')}
              >
                <Icon name="copy" size={16} />
                {actOk === 'copy' && <span className="act-ok-badge">{t('✓ 已复制')}</span>}
              </button>
              <span
                className="unit-model"
                title={t('当前模型 {name}', { name: prettyModel(currentModel, modelLabels) })}
              >
                {prettyModel(currentModel, modelLabels)}
              </span>
              <span className="unit-time">{fmtTime(round.endedAt ?? round.startedAt)}</span>
            </div>
          )}
          {usageAgg && (
            <div className="cost-brief unit-foot-cost">
              {t('已消耗 {tokens} tokens（{calls} 次 API 调用）', {
                tokens: usageAgg.tokens.toLocaleString(),
                calls: usageAgg.calls,
              })}
              {usageAgg.cost > 0 && (
                <span className="cost-price">{t('约 ¥{n}', { n: usageAgg.cost.toFixed(4) })}</span>
              )}
            </div>
          )}
          {hasCost && <CostLine totals={round.totals} usage={round.usage} />}
        </div>
      )}
    </div>
  );
});
