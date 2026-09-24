
const KEY = 'real.confirm-trust';

export const ACTION_LABEL: Record<string, string> = {
  system_command: '危险命令',
  sensitive_write: '敏感写入',
  delete: '删除操作',
  registry_write: '注册表写入',
  select_workspace: '选择工作区',
  ask: '需要你定一下',
};
export const ACTION_DESC: Record<string, string> = {
  system_command: 'Agent 想执行一条可能产生破坏的命令',
  sensitive_write: 'Agent 想写入一个受保护的位置',
  delete: 'Agent 想删除文件或目录',
  registry_write: 'Agent 想修改系统注册表',
};

export function actionLabel(action: string): string {
  return ACTION_LABEL[action] ?? action;
}

export type TrustDecision = 'allow' | 'deny';
export type TrustScope = 'session' | 'global';

interface ConfirmTrust {
  sessions: Record<string, Record<string, TrustDecision>>; // sessionId -> action -> decision
  global: Record<string, TrustDecision>; // action -> decision
}

const EMPTY: ConfirmTrust = { sessions: {}, global: {} };

function load(): ConfirmTrust {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return EMPTY;
    const o = JSON.parse(raw) as ConfirmTrust;
    return {
      sessions: o && typeof o.sessions === 'object' ? o.sessions : {},
      global: o && typeof o.global === 'object' ? o.global : {},
    };
  } catch {
    return EMPTY;
  }
}

function save(t: ConfirmTrust) {
  try {
    localStorage.setItem(KEY, JSON.stringify(t));
  } catch {
  }
}

export function getTrustRule(sessionId: string, action: string): TrustDecision | undefined {
  const t = load();
  return t.sessions[sessionId]?.[action] ?? t.global[action];
}

export function setTrustRule(
  sessionId: string,
  action: string,
  decision: TrustDecision,
  scope: TrustScope,
) {
  const t = load();
  if (scope === 'global') {
    t.global[action] = decision;
  } else {
    t.sessions[sessionId] = { ...(t.sessions[sessionId] ?? {}), [action]: decision };
  }
  save(t);
}

export function listTrustRules(sessionId: string): {
  sessionRules: Array<{ action: string; decision: TrustDecision }>;
  globalRules: Array<{ action: string; decision: TrustDecision }>;
} {
  const t = load();
  const sessionRules = Object.entries(t.sessions[sessionId] ?? {}).map(([action, decision]) => ({
    action,
    decision,
  }));
  const globalRules = Object.entries(t.global).map(([action, decision]) => ({ action, decision }));
  return { sessionRules, globalRules };
}

export function clearSessionTrust(sessionId: string) {
  const t = load();
  delete t.sessions[sessionId];
  save(t);
}

export function clearGlobalTrust() {
  const t = load();
  t.global = {};
  save(t);
}

export function clearTrustRule(sessionId: string, action: string, scope: TrustScope) {
  const t = load();
  if (scope === 'global') {
    delete t.global[action];
  } else {
    delete t.sessions[sessionId]?.[action];
  }
  save(t);
}
