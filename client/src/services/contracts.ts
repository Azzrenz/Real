
export interface Session {
  id: string;
  title: string;
  status: SessionStatus;
  system_prompt: string;
  created_at: string;
  updated_at: string;
  /** Model chosen for this session; null or absent = follow the global default. */
  model?: string | null;
  /** Task area tag: the backend infers it from the round's changed paths, once, so it
   *  cannot drift. */
  area?: string | null;
}

export type SessionStatus =
  | 'idle'
  | 'planning'
  | 'executing'
  | 'solving'
  | 'reflecting'
  | 'done'
  | 'error'
  | 'cancelled';

/** One option in an ask prompt. `detail` is required — an unexplained option is a guess. */
export interface ConfirmOption {
  label: string;
  detail: string;
  recommended: boolean;
}

export interface ConfirmRequestInfo {
  requestId: string;
  action: string;
  target: string;
  impact: string;
  riskLabel: string;
  workspaceCandidates?: string[];
  /** ask: the options; guard confirmations (delete / sensitive write / pick workspace) have none. */
  options?: ConfirmOption[];
}

export interface Message {
  id: string;
  session_id: string;
  role: 'user' | 'assistant' | 'tool' | 'reasoning';
  content: string;
  reasoning?: string;
  item_json: string | null;
  created_at: string;
}

export type ToolStatus = 'success' | 'error' | 'timeout';

export interface ContentPart {
  type: string;
  text?: string;
  uri?: string;
  mime_type?: string;
}

export interface ToolEnvelope {
  tool_call_id: string;
  name: string;
  status: ToolStatus;
  is_error: boolean;
  duration_ms: number;
  content: ContentPart[];
  truncated: boolean;
  meta?: Record<string, unknown>;
  exit_code?: number;
}

export interface ToolCall {
  id: string;
  session_id: string;
  plan_id: string | null;
  step_id: string | null;
  name: string;
  arguments: string;
  result_json: string | null;
  status: string;
  duration_ms: number;
  created_at: string;
}

export interface PlanStep {
  step_id: string; // #E1
  description: string;
  tool_name: string | null;
  tool_args: Record<string, unknown> | null;
  depends_on: string[];
  result?: string | null;
}

export interface Plan {
  plan_id: string;
  objective: string;
  steps: PlanStep[];
  status: string;
  created_at?: string;
}

export interface PlanRow {
  id: string;
  session_id: string;
  objective: string;
  steps_json: string;
  status: string;
  attempt: number;
  created_at: string;
}

export type SseEventKind =
  | 'reasoning'
  | 'message'
  | 'tool'        // {tools:[{step_id,name,args,path,status,duration_ms,result_summary,reason,exit_code}]}
  | 'thinking'    // {status:'start'|'end', label, summary}
  | 'complete'    // {answer, tool_count, llm_calls, cost_yuan, peak_tag, cache_hit_ratio}
  | 'error'
  | 'cancelled'   // {message}
  | 'interrupted'
  | 'user_interjection'
  | 'llm.usage'   // {model,input,cached,output}
  // {items:[{type:'skill'|'memory',name,summary,evidence,category?,body?,merge_into?}], tool_calls}
  | 'evolution.candidates'
  | 'session.title'
  | 'confirm.request'   // {session_id,action,target,impact,risk_label,request_id}
  | 'confirm.cancelled' // {request_id}: the session was cancelled, nobody answered
  | 'confirm.resolved'  // {request_id, approved, trust_session}
  | 'progress'
  // {attempt,max,backoff_ms,reason,upstream} 上游繁忙·自动重试（瞬时态，后端只广播不落库）
  | 'llm.retry';

export interface SseEvent {
  seq: number;
  kind: SseEventKind;
  payload: Record<string, unknown>;
  ts?: string;
}

export interface LLMCall {
  label: string;
  input: number;
  cached: number;
  output: number;
  cost: number;
  peak: string;
  /** 费用分项（元）：未命中输入 / 缓存命中输入 / 输出 —— 让界面能回答“钱花在哪” */
  cost_miss?: number;
  cost_cached?: number;
  cost_output?: number;
}

export interface SessionDetail {
  session: Session;
  messages: Message[];
  tool_calls: ToolCall[];
  plans: PlanRow[];
  events?: SseEvent[];
  /** Whether older events exist; when false the "load earlier" entry is hidden. */
  events_has_earlier?: boolean;
}

export interface ModelLabel {
  id: string;
  label: string;
}

export interface ProviderInfo {
  id: string;
  name: string;
  base_url: string;
  models: ModelLabel[];
}

/**
 * Tuning snapshot (backend `config::settings::TuningSnapshot`).
 * Effective values and factory defaults share one shape; the panel's "default X" reads from
 * `tuning_defaults` rather than hard-coding, so backend changes show up in the UI.
 */
export interface TuningSnapshot {
  max_rounds: number;
  compact_trigger: number;
  compact_keep_raw: number;
  compact_pressure_chars: number;
  task_keep_outputs: number;
  archive_keep_recent: number;
  max_output_tokens: number;
  task_stub_min_bytes: number;
  /** read 全文落盘线（字符，信封口径）。 */
  read_spill_threshold_chars: number;
  /** read 精读（mode=lines）落盘线（字符）。 */
  read_lines_spill_threshold_chars: number;
  /** 落盘预览保留头部（字符）。 */
  spill_preview_head_chars: number;
  /** 落盘预览保留尾部（字符）。 */
  spill_preview_tail_chars: number;
  /** spill 单文件字节硬上限。 */
  spill_max_file_bytes: number;
}

export interface Settings {
  llm_mode: 'mock' | 'real';
  model: string;
  model_label?: string;
  base_url: string;
  has_api_key: boolean;
  api_key_masked: string | null;
  has_glm_key?: boolean;
  default_system_prompt: string;
  thinking_mode: string;
  thinking_effort: string;
  /** Memory scope: shared = across sessions in one workspace; session = this task only. */
  memory_scope?: string;
  models: string[];
  model_labels?: ModelLabel[];
  providers?: ProviderInfo[];
  provider_has_key?: Record<string, boolean>;
  max_rounds?: number;
  compact_trigger?: number;
  compact_keep_raw?: number;
  compact_pressure_chars?: number;
  task_keep_outputs?: number;
  archive_keep_recent?: number;
  max_output_tokens?: number;
  /** In-turn compaction threshold in bytes: tool output is compressed only above it. */
  task_stub_min_bytes?: number;
  /** read 全文落盘线（字符）—— 超过才把全文落盘、上下文只留预览。 */
  read_spill_threshold_chars?: number;
  /** read 精读落盘线（字符）—— mode=lines 用这条。 */
  read_lines_spill_threshold_chars?: number;
  /** 落盘预览保留头部（字符）。 */
  spill_preview_head_chars?: number;
  /** 落盘预览保留尾部（字符）。 */
  spill_preview_tail_chars?: number;
  /** spill 单文件字节硬上限。 */
  spill_max_file_bytes?: number;
  /** Factory default snapshot used for the panel's "default X"; backend is the source. */
  tuning_defaults?: TuningSnapshot;
  data_dir?: string | null;
  data_root_effective?: string;
  server: { host: string; port: number };
}

export interface SettingsUpdate {
  llm_mode?: 'mock' | 'real';
  api_key?: string;
  base_url?: string;
  model?: string;
  /** When present the model is stored on that session; otherwise it changes the default. */
  session_id?: string;
  default_system_prompt?: string;
  thinking_mode?: string;
  thinking_effort?: string;
  /** Memory scope: shared (per workspace) or session (this task only). */
  memory_scope?: string;
  /**
   * Output language for model-produced content (thinking / narration / tool cards).
   * Mirrors the UI language switch (`useLang`); the backend reads it every turn.
   */
  output_lang?: 'zh' | 'en';
  max_rounds?: number;
  compact_trigger?: number;
  compact_keep_raw?: number;
  compact_pressure_chars?: number;
  task_keep_outputs?: number;
  archive_keep_recent?: number;
  /** Output token ceiling per call (a cap: exceeding it truncates). */
  max_output_tokens?: number;
  /** In-turn compaction threshold in bytes. */
  task_stub_min_bytes?: number;
  /** read 全文落盘线（字符）。 */
  read_spill_threshold_chars?: number;
  /** read 精读（mode=lines）落盘线（字符）。 */
  read_lines_spill_threshold_chars?: number;
  /** 落盘预览保留头部（字符）。 */
  spill_preview_head_chars?: number;
  /** 落盘预览保留尾部（字符）。 */
  spill_preview_tail_chars?: number;
  /** spill 单文件字节硬上限。 */
  spill_max_file_bytes?: number;
  data_dir?: string;
}

export interface TestResult {
  ok: boolean;
  url: string;
  status?: number;
  latency_ms?: number;
  note?: string;
  error?: string;
}

export interface Persona {
  identity: string;
  needs: string;
  principles: string;
}

export interface MemoryRow {
  id: string | number;
  value: string;
  priority: number;
  created_at: string;
  session_id?: string;
  permanent?: boolean;
}

export interface PreferenceRow {
  key: string;
  value: string;
  priority: number;
}

export interface MemoryStats {
  themes_total: number;
  themes_permanent: number;
  themes_regular: number;
  prefs_count: number;
  freshness: { today: number; week: number; old: number };
}

export interface ScheduledJob {
  id: string;
  name: string;
  prompt: string;
  cron_expr: string;
  natural_lang: string | null;
  workspace: string | null;
  enabled: boolean;
  last_run_at: string | null;
  last_status: 'running' | 'ok' | 'error' | null;
  last_result: string | null;
  next_run_at: string | null;
  created_at: string;
}

export interface ReadyResult {
  status: string;
  checks: {
    database: boolean;
    llm_mode: string;
    tools: string[];
  };
}

export type McpTransport = 'stdio' | 'http';

export interface McpServerConfig {
  name: string;
  transport: McpTransport;
  command?: string | null;
  args?: string[];
  url?: string | null;
  headers?: Record<string, string>;
}

export interface McpServerStatus {
  name: string;
  connected: boolean;
  tool_count: number;
  last_error?: string | null;
}

export interface McpListResult {
  servers: McpServerStatus[];
  saved: McpServerConfig[];
}

export interface McpApplyResult {
  ok: boolean;
  applied: string[];
  removed: string[];
  failed: Array<{ name: string; error: string }>;
  servers: McpServerStatus[];
}

