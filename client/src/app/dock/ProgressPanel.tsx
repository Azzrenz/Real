import { useEffect, useMemo, useState } from 'react';
import { Icon } from '../../shared/ui/icons';
import { useT } from '../../shared/i18n';
import { revealInFolder } from '../../shared/lib/openExternal';
import {
  FOLD_AFTER_MS,
  joinPath,
  REL_PATH,
  itemPct,
  overallOf,
  useProgress,
  useProgressPolling,
} from './progressStore';

/**
 * 右侧栏的「进度」页。
 *
 * 它读的是**约定好的快照文件**，不是去翻日志、也不是去猜进程：`<工作区>/.real/progress.json`
 * （由 `progress.py` 每次上报时覆盖写）。前端只负责把它画成人看得懂的样子。
 *
 * 为什么是文件、不是接口：长活是**别的进程**在跑（渲染、转码、批处理），它跟 Real 服务
 * 没有关系，也不该为了报进度去连 Real。一份 JSON 落在盘上，谁写都行、谁读都行；
 * Real 关了它照跑，Real 开着就能看。
 *
 * 找哪份快照：读 `progress.py` 维护的**全局索引**（`%APPDATA%/real-agent/progress_index.json`）
 * ——上报进度时长活自己登记，所以这里**不用让用户手填路径**，机器上有几个活在跑就列几个。
 * 索引拿不到（还没跑过长活）时，才退回"工作区默认路径 + 手填"。
 *
 * 为什么轮询 2 秒而不是 SSE：这是个看一眼就走的面板，读的是本地小文件，
 * 2 秒一次的开销远小于为它维护一条推送链路的复杂度。
 */

/** 状态词 -> 中文。表外的原样显示，不吞。 */
const STATE_ZH: Record<string, string> = {
  pending: '待开始',
  wait: '排队',
  running: '进行中',
  starting: '起手',
  cutting: '剪段',
  burning: '合成',
  done: '完成',
  failed: '失败',
};

/** 状态的色档：绿=完成、红=失败、蓝=在跑、灰=还等着。 */
function stateTone(s?: string): 'ok' | 'bad' | 'live' | 'wait' {
  if (s === 'done') return 'ok';
  if (s === 'failed') return 'bad';
  if (s === 'running' || s === 'cutting' || s === 'burning' || s === 'starting') return 'live';
  return 'wait';
}

function ago(sec: number, t: ReturnType<typeof useT>): string {
  if (!sec) return '—';
  const d = Math.max(0, Date.now() / 1000 - sec);
  if (d < 5) return t('刚刚');
  if (d < 60) return t('{n} 秒前', { n: Math.floor(d) });
  if (d < 3600) return t('{n} 分钟前', { n: Math.floor(d / 60) });
  return t('{n} 小时前', { n: Math.floor(d / 3600) });
}

function fmtSecs(s?: number | null): string {
  if (!s && s !== 0) return '';
  if (s < 60) return `${Math.round(s)} 秒`;
  const m = Math.floor(s / 60);
  return m < 60 ? `${m} 分 ${Math.round(s % 60)} 秒` : `${Math.floor(m / 60)} 时 ${m % 60} 分`;
}

type Filter = 'all' | 'running' | 'wait' | 'done' | 'failed';

export function ProgressPanel() {
  const t = useT();
  // 数据不再由本组件自己轮询：索引 / 快照挪进了 progressStore —— 侧栏 tab 上的徽标
  // 也要看同一份数据。各读一遍就是两条计时器、两份状态，迟早出现「tab 说有活、点进去说没有」。
  const root = useProgress((s) => s.root);
  const custom = useProgress((s) => s.custom);
  const lives = useProgress((s) => s.lives);
  const cur = useProgress((s) => s.cur);
  const missing = useProgress((s) => s.missing);
  const err = useProgress((s) => s.err);
  const idleSince = useProgress((s) => s.idleSince);
  const setCustom = useProgress((s) => s.setCustom);
  const setCur = useProgress((s) => s.setCur);
  useProgressPolling();
  const [filter, setFilter] = useState<Filter>('all');
  const [q, setQ] = useState('');
  const [showPath, setShowPath] = useState(false);
  // 明细 / 进程默认折起来：面板第一眼只该回答"到哪了"，细节按需展开。
  // 上百条明细铺满一屏，正是它此前看着不简洁的主因。
  const [subsOpen, setSubsOpen] = useState(false);
  const [procsOpen, setProcsOpen] = useState(false);
  const [, setTick] = useState(0);

  // 「多久之前」要跟着秒走，否则快照停了它也不动，看着像卡住。
  useEffect(() => {
    const id = window.setInterval(() => setTick((n) => n + 1), 1000);
    return () => window.clearInterval(id);
  }, []);

  const live = useMemo(() => lives.find((l) => l.snapshot === cur) ?? lives[0] ?? null, [lives, cur]);
  const snap = live?.snap ?? null;

  const subs = useMemo(() => snap?.subitems ?? [], [snap]);
  const items = useMemo(() => snap?.items ?? snap?.episodes ?? [], [snap]);

  const filtered = useMemo(() => {
    const k = q.trim().toLowerCase();
    return subs.filter((s) => {
      const st = s.state ?? 'wait';
      if (filter === 'running' && st !== 'running') return false;
      if (filter === 'wait' && st !== 'wait' && st !== 'pending') return false;
      if (filter === 'done' && st !== 'done') return false;
      if (filter === 'failed' && st !== 'failed') return false;
      if (k && !(`${s.title ?? ''} ${s.desc ?? ''} ${s.id}`.toLowerCase().includes(k))) return false;
      return true;
    });
  }, [subs, filter, q]);

  const counts = useMemo(() => {
    const c = { all: subs.length, running: 0, wait: 0, done: 0, failed: 0 };
    for (const s of subs) {
      const tone = stateTone(s.state);
      if (tone === 'live') c.running += 1;
      else if (tone === 'ok') c.done += 1;
      else if (tone === 'bad') c.failed += 1;
      else c.wait += 1;
    }
    return c;
  }, [subs]);

  if (err) return <p className="dock-msg">{err}</p>;

  if (missing || !snap) {
    return (
      <div className="panel-pad">
        <div className="panel-head">
          <Icon name="target" size={13} />
          <span className="panel-head-title">{t('进度')}</span>
        </div>
        <p className="dock-msg">
          {t('当前没有在跟踪的活儿。长活（渲染 / 转写 / 批处理）一开跑，这里就会自己列出它。')}
        </p>
        <code className="prog-hint">{joinPath(root || '<工作区>', REL_PATH)}</code>
        <p className="dock-msg">
          {t('长活用 progress.py 上报进度，它会自己登记到索引里——不用在这里填路径。')}
        </p>
        {lives.length > 0 && (
          <p className="dock-msg">
            {t('索引里有 {n} 个进度目录，但快照都读不到：', { n: lives.length })}
            {lives.map((l) => (
              <span key={l.snapshot} className="prog2-errline">
                {l.snapshot} ·{' '}
                {l.err === 'unreadable' ? t('读不到（文件不在，或不是文本）') : (l.err ?? '—')}
              </span>
            ))}
          </p>
        )}
        <button className="prog2-link" onClick={() => setShowPath((v) => !v)}>
          {showPath ? t('收起手填路径') : t('手填路径（一般用不上）')}
        </button>
        {showPath && (
          <input
            className="prog-path"
            value={custom}
            placeholder={joinPath(root || '<工作区>', REL_PATH)}
            spellCheck={false}
            title={t('进度快照文件路径（留空走索引 / 默认）')}
            onChange={(e) => {
              setCustom(e.target.value);
            }}
          />
        )}
      </div>
    );
  }

  // ── 退场 ────────────────────────────────────────────────────────────────
  // 活干完了，面板自己收成一行 —— 这是「进度不该常占页面」的落点。
  // 只在**本次会话真的见过活跑**才收：一开机对着昨天留下的旧快照说「已完成」是撒谎，
  // progressStore 里的 idleSince 因此只在「从有活变成没活」的那一刻才置位。
  // 上面那个每秒 tick 会让这里的 Date.now() 跟着走，到点自然折。
  const [reopened, setReopened] = useState(false);
  useEffect(() => setReopened(false), [idleSince]);
  const folded = idleSince !== null && Date.now() - idleSince >= FOLD_AFTER_MS && !reopened;

  if (folded && idleSince !== null) {
    return (
      <div className="panel-pad prog2">
        <button
          type="button"
          className="prog2-done"
          title={t('展开')}
          onClick={() => setReopened(true)}
        >
          <span className="prog2-done-mark">✓</span>
          <span className="prog2-done-t">{t('已完成')}</span>
          <span className="prog2-done-when">{ago(Math.floor(idleSince / 1000), t)}</span>
          <Icon name="chevron" size={11} />
        </button>
      </div>
    );
  }

  const ov = overallOf(snap);
  // 总步数不可知（agent 自己就是）⇒ 走不定态，不画一个假的 0% 骗人。
  const unknown = ov.pct === null;
  const pct = unknown ? 0 : Math.min(100, Math.max(0, ov.pct as number));
  const stale = Date.now() / 1000 - (snap.time || 0) > 30;
  const running = items.filter((i) => i.state === 'running');
  // 「状态还挂着 running，但活已经不在了」——这时候再把条画成蓝的、会动的，就是撒谎。
  // 取两条里更硬的一条：进程表说 alive=false，或心跳超过 2 分钟没动。
  const lastBeat = Math.max(
    snap.time || 0,
    ...items.map((i) => i.beat || 0),
    ...subs.map((s) => s.beat || 0),
  );
  const beatIdle = Math.max(0, Date.now() / 1000 - lastBeat);
  const anyAlive = items.some((i) => i.state === 'running' && i.alive !== false);
  const interrupted = running.length > 0 && (!anyAlive || beatIdle > 120);

  return (
    <div className="panel-pad prog2">
      <div className="panel-head">
        <Icon name="target" size={13} />
        <span className="panel-head-title">{t('进度')}</span>
        <span
          className={`prog-stamp${stale ? ' stale' : ''}`}
          title={new Date(snap.time * 1000).toLocaleString()}
        >
          {stale ? t('{t}没动', { t: ago(snap.time, t) }) : ago(snap.time, t)}
        </span>
        {/* 快照落在盘上，用户多半想直接去看它旁边的产物 —— 就地开一次文件夹，
            比让人回去地址栏复制路径少三步。 */}
        <button
          className="prog2-act"
          title={t('在文件管理器里打开')}
          aria-label={t('在文件管理器里打开')}
          onClick={() => void revealInFolder(live?.dir || live?.snapshot || '')}
        >
          <Icon name="folder" size={12} />
        </button>
      </div>

      {lives.length > 1 && (
        <select className="prog2-select" value={cur} onChange={(e) => setCur(e.target.value)}>
          {lives.map((l) => (
            <option key={l.snapshot} value={l.snapshot}>
              {(l.snap ? (l.snap.items ?? []).map((i) => i.title).slice(0, 2).join(' / ') : '?') +
                ' · ' +
                l.snapshot}
            </option>
          ))}
        </select>
      )}

      {/* 总览：百分比、条、计数挤进同一行。原先是大数字 / 长条 / 0-50-100 刻度三段竖排，
          面板顶上就吃掉小半屏，而刻度本身不带信息、纯属噪音，一并去掉。 */}
      <div className="prog2-sum">
        <span className={`prog2-sum-pct${interrupted ? ' dead' : ''}`}>
          {unknown ? '--' : Math.round(pct)}
          <i>%</i>
        </span>
        <div className="prog2-sum-mid">
          <div
            className="prog2-bar"
            role="progressbar"
            aria-valuenow={unknown ? undefined : Math.round(pct)}
            aria-valuemin={0}
            aria-valuemax={100}
          >
            {unknown ? (
              <span className="prog2-bar-fill indet" />
            ) : (
              <span
                className={`prog2-bar-fill${interrupted ? ' dead' : ''}`}
                style={{ width: `${pct}%` }}
              />
            )}
          </div>
          <div className="prog2-sum-meta">
            {t('完成 {a}/{b}', { a: ov.done, b: ov.total })}
            {interrupted ? (
              <span className="prog2-hero-dead">
                {t('· {n} 项已中断', { n: running.length })}
              </span>
            ) : running.length > 0 ? (
              <span className="prog2-hero-run">{t('· 在跑 {n} 项', { n: running.length })}</span>
            ) : null}
          </div>
        </div>
      </div>

      {items.length > 0 && (
        <ul className="prog2-cards">
          {items.map((it) => {
            const p = itemPct(it);
            const tone = stateTone(it.state);
            // 卡片描述若是「第 25/101 段（本段 7.7 秒）」这种，前半行再写一遍 25/101
            // 就是同一句话说两遍 —— 让位给描述，别让一行里塞两个同样的分数。
            const dupNum =
              !!it.what && it.n_total != null && it.what.includes(`${it.n_done ?? 0}/${it.n_total}`);
            const bits: string[] = [];
            if (it.n_total) {
              if (!dupNum) bits.push(`${it.n_done ?? 0}/${it.n_total}`);
            } else if (it.n_done) {
              // 总步数不可知的任务（agent 自己就是）：不画没有分母的假分数，改报「第 N 轮」。
              bits.push(t('第 {n} 轮', { n: it.n_done }));
            }
            if (it.clips) bits.push(`${it.clips} 镜`);
            if (it.size_mb) bits.push(`${it.size_mb} MB`);
            if (it.plan_s) bits.push(t('计划 {s}', { s: fmtSecs(it.plan_s) }));
            return (
              <li className={`prog2-card tone-${tone}`} key={it.tag}>
                <div className="prog2-card-top">
                  <span
                    className={`prog2-dot ${tone}${it.state === 'running' && (it.alive === false || it.stuck) ? ' dead' : ''}`}
                  />
                  <span className="prog2-card-title" title={it.title}>
                    {it.title || it.tag}
                  </span>
                  <span className="prog2-card-pct">{p === null ? '--' : `${Math.round(p)}%`}</span>
                </div>
                <div className="prog2-mini">
                  {p === null ? (
                    <i className="indet" />
                  ) : (
                    <i style={{ width: `${Math.min(100, p)}%` }} />
                  )}
                </div>
                <div className="prog2-card-bot">
                  <span className="prog2-num">
                    
                    {bits.join(' · ')}
                  </span>
                  {it.what && (
                    <span className="prog2-what" title={it.what}>
                      {it.what}
                    </span>
                  )}
                  {it.stuck && it.state_note ? (
                    <span className="prog2-what" title={it.state_note}>
                      {it.state_note}
                    </span>
                  ) : null}
                  <span className={`prog-state s-${it.state}`}>{t(STATE_ZH[it.state] ?? it.state)}</span>
                </div>
              </li>
            );
          })}
        </ul>
      )}

      {subs.length > 0 && (
        <>
          {/* 明细默认折起：一屏上百条正是"不简洁"的主因 —— 先给结论，细节按需展开。 */}
          <button
            className={`prog2-fold${subsOpen ? ' on' : ''}`}
            onClick={() => setSubsOpen((v) => !v)}
          >
            <Icon name="chevron" size={11} />
            {t('明细 {n} 条', { n: subs.length })}
            {counts.running > 0 && <b>{t('· {n} 在跑', { n: counts.running })}</b>}
          </button>
          <div className={subsOpen ? 'prog2-fold-body' : 'prog2-hidden'}>
          <div className="prog2-chips">
            {(
              [
                ['all', t('全部'), counts.all],
                ['running', t('进行中'), counts.running],
                ['wait', t('待办'), counts.wait],
                ['done', t('已完成'), counts.done],
                ['failed', t('失败'), counts.failed],
              ] as [Filter, string, number][]
            ).map(([k, label, n]) => (
              <button
                key={k}
                className={`prog2-chip${filter === k ? ' on' : ''}`}
                onClick={() => setFilter(k)}
              >
                {label} <b>{n}</b>
              </button>
            ))}
          </div>
          {subs.length > 30 && (
            <input
              className="prog2-search"
              value={q}
              placeholder={t('搜文件名 / 描述 …')}
              spellCheck={false}
              onChange={(e) => setQ(e.target.value)}
            />
          )}
          <ul className="prog2-list">
            {filtered.slice(0, 400).map((s) => {
              const tone = stateTone(s.state);
              return (
                <li className={`prog2-row tone-${tone}`} key={s.id}>
                  <span className={`prog2-dot ${tone}`} />
                  <div className="prog2-body">
                    <div className="prog2-line1">
                      <span className="prog2-name" title={s.title ?? s.id}>
                        {s.title ?? s.id}
                      </span>
                      <span className="prog2-secs">{s.secs ? fmtSecs(s.secs) : ''}</span>
                    </div>
                    <div className="prog2-mini">
                      {tone === 'live' ? (
                        <i className="indet" />
                      ) : (
                        <i style={{ width: tone === 'ok' || tone === 'bad' ? '100%' : '0%' }} />
                      )}
                    </div>
                    <div className="prog2-line2">
                      <span className="prog2-desc" title={s.desc}>
                        {s.desc ?? ''}
                      </span>
                      {s.note ? <span className="prog2-note">{t('产出 {n}', { n: s.note })}</span> : null}
                    </div>
                  </div>
                </li>
              );
            })}
          </ul>
          {filtered.length > 400 && (
            <p className="dock-msg">{t('只画前 400 条（共 {n} 条）——筛一下再看', { n: filtered.length })}</p>
          )}
          {filtered.length === 0 && <p className="dock-msg">{t('这个筛选下没有条目。')}</p>}
          </div>
        </>
      )}

      {(snap.procs ?? []).length > 0 && (
        <>
          {/* 进程表是实现细节，与上面的任务卡说的是同一件事 —— 折起来，别抢结论的位置。 */}
          <button
            className={`prog2-fold${procsOpen ? ' on' : ''}`}
            onClick={() => setProcsOpen((v) => !v)}
          >
            <Icon name="chevron" size={11} />
            {t('进程 {n} 个 · 合计 {m} MB', {
              n: (snap.procs ?? []).length,
              m: snap.total_mem_mb ?? 0,
            })}
          </button>
          <ul className={procsOpen ? 'prog-procs' : 'prog2-hidden'}>
            {(snap.procs ?? []).slice(0, 10).map((p) => (
              <li className="prog-proc" key={`${p.pid}-${p.name}`}>
                <span className="prog-proc-name">{p.name}</span>
                <span className="prog-proc-num">{p.mem_mb} MB</span>
                <span className="prog-proc-num">{p.cpu_s}s</span>
                <span className="prog-proc-what" title={p.what}>
                  {p.what}
                </span>
              </li>
            ))}
          </ul>
        </>
      )}
    </div>
  );
}
