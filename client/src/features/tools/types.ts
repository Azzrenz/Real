
export interface ToolCard {
  type: 'tool';
  stepId?: string;
  name: string;
  path?: string;
  args?: unknown;
  status: 'running' | 'success' | 'warning' | 'error' | 'cancelled';
  durationMs?: number;
  summary?: string;
  reason?: string;
  exitCode?: number;
  startedAt?: string;
  elapsedMs?: number;
}
