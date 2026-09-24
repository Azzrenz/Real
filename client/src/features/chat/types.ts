
import type {
  ConfirmRequestInfo,
  LLMCall,
  Message,
  SseEvent,
  ToolCall,
} from '../../services/contracts';
import type { ToolCard } from '../tools/types';

export interface ThinkRow {
  type: 'think';
  id: number;
  label: string;
  note: string;
  body: string;
  status: 'thinking' | 'done';
  lazy?: { from: number; to: number; len?: number };
  loading?: boolean;
}

export interface NarrationRow {
  type: 'narration';
  id: number;
  text: string;
}

export type ChatNode = ThinkRow | ToolCard | NarrationRow | UserBubbleRow | EvolutionCard;

export interface EvoItem {
  type: 'skill' | 'memory';
  name: string;
  summary: string;
  evidence: string;
  category?: string;
  body?: string;
  merge_into?: string;
}
export interface EvolutionCard {
  type: 'evo';
  id: number;
  items: EvoItem[];
  toolCalls: number;
}

export interface UserBubbleRow {
  type: 'user_bubble';
  id: number;
  text: string;
  cancelled?: boolean;
}

export interface Round {
  seq: number;
/** the run that produced this round (backend stamps run_id on every event). */
  runId?: string;
  startedAt: string;
  endedAt?: string;
  status: 'running' | 'completed' | 'error' | 'cancelled' | 'interrupted';
  nodes: ChatNode[];
  answer: string;
  errorText?: string;

  notice?: string;
  metrics: { runMs?: number };
  usage: LLMCall[];
  totals: {
    tool_count?: number;
    llm_calls?: number;
    cost_yuan?: number;
    peak_tag?: string;
    cache_hit_ratio?: number;
    model?: string;
  };
}

export interface SessionChat {
  rounds: Round[];
  live: Round | null;
}

export interface ChatStoreState {
  sessions: Record<string, SessionChat>;
  lastSeq: Record<string, number>;
 /** per-session Set (O(1) dedup; was number[] with O(n) includes -- long sessions stalled) */
  seenSeq: Record<string, Set<number>>;
 /** terminal boundary: events with seq <= this are ignored (cancel/late arrivals) */
  terminalSeq: Record<string, number>;
 /** true after archiveLive: late complete/error with no live must not spawn phantom rounds */
  archived: Record<string, boolean>;
  replaying: Record<string, boolean>;
 /** pending danger-op confirmation (ConfirmGate, seq=0 events, bypasses seq gates) */
  confirm: (ConfirmRequestInfo & { sessionId: string }) | null;

  applyEvent: (sessionId: string, event: SseEvent) => void;
  replayEvents: (sessionId: string, events: SseEvent[]) => void;
  replayFromMessages: (sessionId: string, messages: Message[], toolCalls?: ToolCall[]) => void;
 /** archive live as terminal round (cancel path), blocks late events */
  archiveLive: (sessionId: string, status?: Round['status']) => void;
  reset: (sessionId: string) => void;
  removeSession: (sessionId: string) => void;
/** user starts a new run: lift cancel boundary + archived flag so fresh events flow again. */
  beginRun: (sessionId: string, runId?: string) => void;
  fetchIncremental: (sessionId: string) => Promise<void>;
  loadThinking: (sessionId: string, thinkId: number) => Promise<void>;
 /** answer ConfirmGate; posts to backend and clears the pending modal */
  respondConfirm: (
    approved: boolean,
    trustSession: boolean,
    /** The picked value: a workspace path, or the chosen option's label. */
    selected?: string,
    /** ask only: the sentence the user typed in their own words. */
    note?: string,
  ) => Promise<void>;
}
