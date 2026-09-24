
function ws(workspace: string): string {
  return `?workspace=${encodeURIComponent(workspace)}`;
}

export const EP = {
  health: () => '/health',
  ready: () => '/ready',

  sessions: {
    list: () => '/api/sessions',
    create: () => '/api/sessions',
    detail: (id: string, withEvents = false) =>
      `/api/sessions/${id}?skeleton=true&events=${withEvents ? 1 : 0}`,
    patch: (id: string) => `/api/sessions/${id}`,
    remove: (id: string) => `/api/sessions/${id}`,
    thinking: (id: string, from: number, to: number) =>
      `/api/sessions/${id}/thinking?from=${from}&to=${to}`,
    events: (id: string, before: number) =>
      `/api/sessions/${id}/events/before?before=${before}`,
  },

  chat: {
    send: (id: string) => `/api/sessions/${id}/chat`,
    interject: (id: string) => `/api/sessions/${id}/interject`,
    cancel: (id: string) => `/api/sessions/${id}/cancel`,
    confirm: (id: string) => `/api/sessions/${id}/confirm`,
  },

  events: {
    stream: (id: string, after: number) => `/api/sessions/${id}/events?after=${after}`,
    poll: (id: string, after: number) => `/api/sessions/${id}/events?after=${after}&poll=true`,
  },

  /** Reveal a path in the OS file manager (clicking a filename on a tool card). */
  reveal: () => '/api/reveal',

  settings: {
    get: () => '/api/settings',
    put: () => '/api/settings',
    test: () => '/api/settings/test',
  },

  memory: {
    persona: () => '/api/memory/persona',
    preferences: (workspace = '') => `/api/memory/preferences${ws(workspace)}`,
    addPreference: () => '/api/memory/preferences',
    removePreference: (workspace: string, key: string) =>
      `/api/memory/preferences?workspace=${encodeURIComponent(workspace)}&key=${encodeURIComponent(key)}`,
    themes: (workspace = '') => `/api/memory/themes${ws(workspace)}`,
    pinTheme: () => '/api/memory/themes/pin',
    removeTheme: (id: string | number) => `/api/memory/themes/${encodeURIComponent(String(id))}`,
    digests: (workspace = '') => `/api/memory/digests${ws(workspace)}`,
    stats: (workspace = '') => `/api/memory/stats${ws(workspace)}`,
  },

  mcp: {
    list: () => '/api/mcp/servers',
    save: () => '/api/mcp/servers',
    apply: () => '/api/mcp/servers/apply',
  },

  schedule: {
    list: () => '/api/schedules',
    create: () => '/api/schedules',
    remove: (id: string) => `/api/schedules/${id}`,
    toggle: (id: string) => `/api/schedules/${id}/toggle`,
    run: (id: string) => `/api/schedules/${id}/run`,
  },
} as const;
