
import { t } from '../shared/i18n';

/** 后端地址与设备 token 的持久化键。
 *
 *  为什么地址要能在**运行期**决定：桌面壳里后端永远在 `127.0.0.1:8943`，写死没问题；
 *  但手机是由浏览器/PWA 打开的，后端在另一台机器上（走 Tailscale），地址只能在打开时
 *  现场指定。所以解析顺序是：`?api=` 参数 → localStorage → 构建期默认值。
 *
 *  `?api=` 排在最前是有意的：它是用户**当面敲进去**的地址，也是一次性的「换服务器」入口，
 *  比上一次存下的值更新；读到即落盘，之后不带参数打开也还连得上。
 *
 *  解析只在模块加载时做一次，`BASE_URL` 因此仍是常量 —— `SSE_BASE_URL`
 *  （`services/domains/events.ts`）是 import 期快照，保持它不变，SSE 那侧一行都不用改。
 */
const DEFAULT_BASE_URL = 'http://127.0.0.1:8943';
const BASE_KEY = 'real.api-base';
const TOKEN_KEY = 'real.device-token';

/** 归一化后端地址：补 `http://` 前缀、去尾部斜杠。空串表示「不是个有效地址」。 */
export function normalizeBase(raw: string): string {
  let v = (raw ?? '').trim();
  if (!v) return '';
  if (!/^https?:\/\//i.test(v)) v = `http://${v}`;
  return v.replace(/\/+$/, '');
}

/** 当前存下的后端地址（未归一化前的原样），供配对页回填。 */
export function storedBase(): string {
  try {
    return localStorage.getItem(BASE_KEY) ?? '';
  } catch {
    return '';
  }
}

/** 存下后端地址（配对成功后调用）。 */
export function saveBase(raw: string): void {
  const v = normalizeBase(raw);
  if (!v) return;
  try {
    localStorage.setItem(BASE_KEY, v);
  } catch {
    /* 存储不可用：本次会话内仍能用，重载后失效 */
  }
}

/** 设备 token（配对后由后端下发）。桌面上永远是空串 ⇒ 请求头零变化。 */
export function getDeviceToken(): string {
  try {
    return localStorage.getItem(TOKEN_KEY) ?? '';
  } catch {
    return '';
  }
}

export function setDeviceToken(token: string): void {
  try {
    localStorage.setItem(TOKEN_KEY, token);
  } catch {
    /* 同上 */
  }
}

export function clearDeviceToken(): void {
  try {
    localStorage.removeItem(TOKEN_KEY);
  } catch {
    /* 同上 */
  }
}

function resolveBaseUrl(): string {
  try {
    const raw = new URLSearchParams(window.location.search).get('api');
    const v = normalizeBase(raw ?? '');
    if (v) {
      saveBase(v);
      return v;
    }
  } catch {
    /* 无 window/location（构建期、测试）：落到下一档 */
  }
  const saved = normalizeBase(storedBase());
  if (saved) return saved;
  return import.meta.env.VITE_API_BASE ?? DEFAULT_BASE_URL;
}

const BASE_URL = resolveBaseUrl();

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
  // 有 token 才带 Authorization：桌面（回环）永远没有 token，请求头与改动前逐字节一致。
  const token = getDeviceToken();
  let res: Response;
  try {
    res = await fetch(`${BASE_URL}${path}`, {
      ...options,
      signal: ctrl.signal,
      headers: {
        'Content-Type': 'application/json',
        ...(token ? { Authorization: `Bearer ${token}` } : {}),
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
