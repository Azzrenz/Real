import { useEffect, useMemo } from 'react';
import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';

/**
 * 进度数据的**唯一一份**：索引 + 快照 + 轮询。
 *
 * 为什么从 ProgressPanel 里搬出来：tab 上的徽标要跟面板看同一份数据。
 * 各轮各的，就是两份计时器、两份状态，迟早出现"tab 说有活、点进去说没有"。
 *
 * 它读的仍是**约定好的快照文件**（`<工作区>/.real/progress.json`），不是去翻日志、也不是去猜进程。
 * 长活是别的进程在跑（渲染、转码、批处理），跟 Real 服务没关系，也不该为了报进度去连 Real；
 * 一份 JSON 落在盘上，谁写都行、谁读都行。找哪份快照靠 progress.py 维护的全局索引。
 */

/* ─────────────────────────── 快照的数据形状 ─────────────────────────── */

/** 主任务（一个 tag 一条）：字段名与 progress.py 的 aggregate() 输出逐字对应。 */
export interface SnapItem {
  tag: string;
  title: string;
  state: string;
  n_done?: number | null;
  n_total?: number | null;
  clips?: number;
  segs?: number;
  size_mb?: number;
  pct?: number | null;
  plan_s?: number | null;
  pid?: number | null;
  what?: string;
  alive?: boolean;
  started?: number;
  beat?: number;
  /** 后端 aggregate() 的滞停判定：状态还挂 running、但心跳过期或进程已不在。 */
  stuck?: boolean;
  /** 那条判定的中文理由（“进程已不在” / “心跳已停 N 秒”），直接显示，不再由前端拼。 */
  state_note?: string;
}

/** 子条目（一个视频一条）：asr_batch / framing 逐条上报。 */
export interface SnapSub {
  id: string;
  title?: string;
  desc?: string;
  state?: string;
  secs?: number | null;
  note?: number | null;
  started?: number;
  beat?: number;
  ended?: number;
}

export interface SnapProc {
  name: string;
  pid: string | number;
  mem_mb: number;
  cpu_s: number;
  what: string;
}

export interface Snapshot {
  time: number;
  items?: SnapItem[];
  /** 兼容旧键名 episodes（watch_render.py 早期输出）。 */
  episodes?: SnapItem[];
  subitems?: SnapSub[];
  procs?: SnapProc[];
  sub_done?: number;
  done: number;
  total: number;
  total_mem_mb: number;
}

/** 索引里的一条：一份快照落在哪儿。 */
export interface IndexRow {
  dir: string;
  snapshot: string;
  mtime: number;
}

/** 一条活 = 一份快照 + 它在盘上的位置。 */
export interface Live {
  dir: string;
  snapshot: string;
  mtime: number;
  snap: Snapshot | null;
  err?: string;
}

/* ───────────────────────────── 常量与纯函数 ───────────────────────────── */

/** 工作区下的约定路径。写成常量，免得脚本和面板哪天各改一半。 */
export const REL_PATH = '.real/progress.json';

/** 路径可改：长活不一定落在 Real 的工作区里；改过的路径存本地，下次还认。 */
export const LS_KEY = 'dock.progress.path';

/** 一次最多同时读几份快照：机器上跑过几百个活时，别一次开几百个文件。 */
const MAX_LIVES = 8;

/** 活儿停了之后，面板还留多久才自己收起来。 */
export const FOLD_AFTER_MS = 10_000;

export function joinPath(root: string, rel: string): string {
  const s = root.replace(/[\\/]+$/, '');
  const sep = s.includes('\\') ? '\\' : '/';
  return `${s}${sep}${rel.replace(/[\\/]+/g, sep)}`;
}

/** 主任务完成的百分比：有 n_total 就用它，否则退回脚本给的 pct。 */
export function itemPct(it: SnapItem): number | null {
  if (it.state === 'done' || it.state === 'failed') return 100;
  const tot = it.n_total ?? 0;
  if (tot > 0) return (100 * (it.n_done ?? 0)) / tot;
  return it.pct ?? null;
}

/**
 * 一份快照的总条：按**主条**算，每个活等权。
 *
 * 曾经用子条（snap.subitems）算总条，结果是灾难性的：子条是同一目录下所有活混在一起的，
 * 转写（asr）的 253 条子条一跑完，取景跟焦（framing）还在 40% 就被总条盖成 100%。
 * 子条只该在下面列表里展示进度，不该决定总条。
 */
export function overallOf(snap: Snapshot): { pct: number | null; done: number; total: number } {
  const items = snap.items ?? snap.episodes ?? [];
  if (items.length > 0) {
    const ps = items.map(itemPct).filter((p): p is number => p !== null);
    const done = items.filter((i) => i.state === 'done' || i.state === 'failed').length;
    return {
      pct: ps.length ? ps.reduce((a, b) => a + b, 0) / ps.length : null,
      done,
      total: items.length,
    };
  }
  const subs = snap.subitems ?? [];
  if (subs.length > 0) {
    const done = snap.sub_done ?? subs.filter((s) => s.state === 'done' || s.state === 'failed').length;
    return { pct: (100 * done) / subs.length, done, total: subs.length };
  }
  const total = snap.total ?? 0;
  return { pct: total > 0 ? (100 * (snap.done ?? 0)) / total : null, done: snap.done ?? 0, total };
}

/**
 * 还有活在跑吗 —— 面板要不要收、tab 上要不要挂徽标，都问这一句。
 *
 * 主条和子条都算：有的脚本只报子条（asr_batch），只看主条会把它当成"没活"。
 */
export function hasLiveWork(lives: Live[]): boolean {
  for (const l of lives) {
    const s = l.snap;
    if (!s) continue;
    if ((s.items ?? s.episodes ?? []).some((i) => i.state === 'running')) return true;
    if ((s.subitems ?? []).some((i) => i.state === 'running')) return true;
  }
  return false;
}

/* ──────────────────────────────── 状态 ──────────────────────────────── */

/**
 * 本次会话里见过 running 吗。只在模块里放一份、不进 state：
 * 它决定「✓ 已完成」要不要出现 —— 一开机看见昨天留下的旧快照就说"已完成"是撒谎。
 */
let everRunning = false;

/** 防重入：一次 refresh 没回来之前，下一次直接跳过（本地小文件，跳过不丢事）。 */
let inflight = false;

function readCustom(): string {
  try {
    return window.localStorage.getItem(LS_KEY) ?? '';
  } catch {
    return '';
  }
}

interface ProgressState {
  root: string;
  custom: string;
  lives: Live[];
  cur: string;
  missing: boolean;
  /** null=没出错；'unreadable'=文件不在或不是文本；其余=异常字符串。 */
  err: string | null;
  /** 最后一次「从有活变成没活」的时刻（ms）。有活在跑时为 null。 */
  idleSince: number | null;
  setCur: (p: string) => void;
  setCustom: (p: string) => void;
  refresh: () => Promise<void>;
}

export const useProgress = create<ProgressState>()((set, get) => ({
  root: '',
  custom: readCustom(),
  lives: [],
  cur: '',
  missing: true,
  err: null,
  idleSince: null,

  setCur: (p) => set({ cur: p }),

  // 路径跟着存本地：改过的路径下次还认。
  setCustom: (p) => {
    try {
      window.localStorage.setItem(LS_KEY, p);
    } catch {
      void 0;
    }
    set({ custom: p });
  },

  refresh: async () => {
    if (inflight) return;
    inflight = true;
    try {
      const { custom, cur, root: cachedRoot } = get();

      // 工作区根只取一次。取不到也不致命：索引里本来就带绝对路径。
      let root = cachedRoot;
      if (!root) {
        try {
          root = await invoke<string>('workspace_root');
          set({ root });
        } catch {
          void 0;
        }
      }

      // 1) 索引：机器上有哪些活在跑。索引读不到就退回"工作区默认路径"这一条。
      let rows: IndexRow[] = [];
      try {
        const raw = await invoke<string | null>('progress_index');
        if (raw) {
          const obj = JSON.parse(raw) as Record<string, IndexRow>;
          rows = Object.values(obj).filter((r) => r && r.snapshot);
        }
      } catch {
        rows = [];
      }
      if (custom && !rows.some((r) => r.snapshot.toLowerCase() === custom.toLowerCase())) {
        rows.unshift({ dir: '', snapshot: custom, mtime: 0 });
      }
      if (root && rows.length === 0) {
        rows = [{ dir: root, snapshot: joinPath(root, REL_PATH), mtime: 0 }];
      }
      rows.sort((a, b) => (b.mtime || 0) - (a.mtime || 0));

      // 2) 每份快照都读一遍：挑活跃的要有依据，光看 mtime 不够（快照本身也在被写）。
      const read = await Promise.all(
        rows.slice(0, MAX_LIVES).map(async (r): Promise<Live> => {
          try {
            const txt = await invoke<string | null>('read_preview_text', { path: r.snapshot });
            // 文案不在这里翻译 —— store 拿不到语言环境。哨兵留给面板去译。
            if (txt === null) return { ...r, snap: null, err: 'unreadable' };
            return { ...r, snap: JSON.parse(txt) as Snapshot };
          } catch (e) {
            return { ...r, snap: null, err: String(e) };
          }
        }),
      );

      // 3) 选中项：用户选过就认它，否则挑"在跑优先、其次最新"的那条。
      const keep = read.find((l) => l.snapshot === cur);
      const nextCur = keep
        ? cur
        : (read.find((l) => (l.snap?.items ?? []).some((i) => i.state === 'running')) ??
            read.find((l) => l.snap) ??
            read[0])?.snapshot ?? '';

      // 4) 「还有没有活在跑」的沿变：只在"见过 running → 现在没了"的那一刻记时间戳，
      //    这样面板收起来的时机是"活干完"而不是"第一次打开面板"。
      const busy = hasLiveWork(read);
      let idleSince = get().idleSince;
      if (busy) {
        everRunning = true;
        idleSince = null;
      } else if (everRunning && idleSince === null) {
        idleSince = Date.now();
      }

      set({
        lives: read,
        cur: nextCur,
        missing: read.length === 0 || !read.some((l) => l.snap),
        err: null,
        idleSince,
      });
    } finally {
      inflight = false;
    }
  },
}));

/* ──────────────────────────────── hooks ──────────────────────────────── */

/** 同一页面里挂几次都只跑一条计时器：最后一个卸载才停。 */
let mounts = 0;
let timer: number | null = null;

/**
 * 轮询 2 秒。为什么不是 SSE：这是个看一眼就走的面板，读的是本地小文件，
 * 2 秒一次的开销远小于为它维护一条推送链路的复杂度。
 */
export function useProgressPolling(): void {
  useEffect(() => {
    mounts += 1;
    if (timer === null) {
      void useProgress.getState().refresh();
      timer = window.setInterval(() => void useProgress.getState().refresh(), 2000);
    }
    return () => {
      mounts -= 1;
      if (mounts === 0 && timer !== null) {
        window.clearInterval(timer);
        timer = null;
      }
    };
  }, []);
}

export interface ProgressBadge {
  kind: 'live' | 'done';
  /** 有活在跑时是总百分比；总步数不可知（agent 自己）时为 null，徽标只画一个点。 */
  pct: number | null;
}

/**
 * 侧栏 tab 上那个小徽标 —— 它替面板回答**「现在要不要看一眼」**。
 *
 * 有活在跑 → 报百分比；全干完了 → 一个勾（停在那儿，直到下次有新活）。
 * 没有活、本次会话也没跑过 → 返回 null，tab 上干干净净。这是"进度不该占页面"的落点：
 * 常态只占一个字符，真的要细节才点进去。
 */
export function useProgressBadge(): ProgressBadge | null {
  const lives = useProgress((s) => s.lives);
  const cur = useProgress((s) => s.cur);
  const idleSince = useProgress((s) => s.idleSince);

  return useMemo(() => {
    if (hasLiveWork(lives)) {
      const snap =
        (lives.find((l) => l.snapshot === cur) ?? lives.find((l) => l.snap))?.snap ?? null;
      return { kind: 'live' as const, pct: snap ? overallOf(snap).pct : null };
    }
    if (idleSince !== null) return { kind: 'done' as const, pct: null };
    return null;
  }, [lives, cur, idleSince]);
}
