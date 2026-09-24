
import { t } from '../shared/i18n';

const BASE_URL = import.meta.env.VITE_API_BASE ?? 'http://127.0.0.1:8943';

export class ApiError extends Error {
  constructor(
    public status: number,
    public body: Record<string, unknown> | null,
  ) {
    super((body?.detail as string) ?? `API error ${status}`);
  }
}

const REQUEST_TIMEOUT_MS = 8_000;

async function request<T>(path: string, options: RequestInit = {}): Promise<T> {
  const ctrl = new AbortController();
  const timer = window.setTimeout(() => ctrl.abort(), REQUEST_TIMEOUT_MS);
  let res: Response;
  try {
    res = await fetch(`${BASE_URL}${path}`, {
      ...options,
      signal: ctrl.signal,
      headers: {
        'Content-Type': 'application/json',
        ...options.headers,
      },
    });
  } catch (e) {
    const aborted = ctrl.signal.aborted;
    const err = aborted
      ? new Error(
          t('请求超时（{s}s）——后端可能正在重启，请稍候，将自动重试', {
            s: REQUEST_TIMEOUT_MS / 1000,
          }),
        )
      : (e as Error);
    throw err;
  } finally {
    window.clearTimeout(timer);
  }
  if (!res.ok) {
    const body = await res.json().catch(() => null);
    throw new ApiError(res.status, body);
  }
  if (res.status === 204) return undefined as T;
  return res.json() as Promise<T>;
}

export const http = {
  baseUrl: BASE_URL,
  get: <T>(path: string) => request<T>(path),
  post: <T>(path: string, data?: unknown) =>
    request<T>(path, { method: 'POST', body: data === undefined ? undefined : JSON.stringify(data) }),
  put: <T>(path: string, data?: unknown) =>
    request<T>(path, { method: 'PUT', body: data === undefined ? undefined : JSON.stringify(data) }),
  patch: <T>(path: string, data?: unknown) =>
    request<T>(path, { method: 'PATCH', body: data === undefined ? undefined : JSON.stringify(data) }),
  delete: <T>(path: string) => request<T>(path, { method: 'DELETE' }),
};
