/** 设备配对状态：token 有没有、够不够用，以及配对本身怎么走。
 *
 *  后端把鉴权边界划得很干净（见 `server/src/routes/auth_guard.rs`）：
 *  - 回环来源（桌面壳打 127.0.0.1）**免鉴权** ⇒ 桌面永远「已配对」，这一层什么都不做；
 *  - 非回环来源必须持有效设备 token，否则 `/api/*` **统一返回 404**（不暴露服务存在）。
 *  所以「404」在前端读作「我还没配对 / token 失效了」，绝不能读作「接口不存在」。
 *
 *  ⚠️ 配对码过期与配对码写错，后端都回 401（同一个 `Unauthorized`），前端**无法区分**
 *  —— 提示语必须如实说「可能已过期，请在电脑上重新生成」，不能假装知道是哪一种。
 */
import { create } from 'zustand';
import {
  ApiError,
  clearDeviceToken,
  getDeviceToken,
  http,
  normalizeBase,
  saveBase,
  setDeviceToken,
} from '../../../services/http';
import { EP } from '../../../services/endpoints';
import { inTauri } from '../../../shared/lib/tauri';
import { t } from '../../../shared/i18n';

export type AuthStatus = 'unknown' | 'checking' | 'paired' | 'unpaired';

/** 配对请求的超时：比普通请求短一点，地址填错时别让用户干等。 */
const PAIR_TIMEOUT_MS = 8_000;

interface AuthState {
  status: AuthStatus;
  /** 配对页上正在提交。 */
  pairing: boolean;
  /** 配对失败的原因（已翻译成一句人话），成功或重开配对时清空。 */
  error: string | null;

  /** 启动时探一次：够不够用。桌面直接算已配对，不发请求。 */
  probe: () => Promise<void>;
  /** 任何一次业务请求撞上 404（= 未鉴权）时调用，把界面切回配对页。 */
  noteApiError: (e: unknown) => void;
  /** 用 6 位配对码换 token；成功即重载让 BASE_URL 生效。 */
  pair: (server: string, code: string, name?: string) => Promise<boolean>;
  /** 清掉本机 token（换设备 / 排查用）。 */
  unpair: () => void;
}

/** 把 fetch/网络异常翻成一句用户看得懂的话。 */
function describePairFailure(e: unknown): string {
  if (e instanceof ApiError) {
    if (e.status === 401) {
      return t('配对码不对或已过期（配对码 5 分钟内有效，且只能成功一次）。请在电脑上重新生成一个再试。');
    }
    if (e.status === 429) {
      return t('尝试次数过多，已被暂时限速。稍等一会儿，或到电脑上重新生成配对码。');
    }
    if (e.status === 404) {
      return t('这个地址上没有找到 Real 服务——请确认服务器地址填对了，且电脑上的 Real 正在运行。');
    }
    return t('配对失败（HTTP {s}）。请稍后重试。', { s: e.status });
  }
  return t('连不上这个服务器地址。请确认它和电脑在同一个网络（如 Tailscale）里，且地址、端口没写错。');
}

export const useAuthStore = create<AuthState>((set) => ({
  status: 'unknown',
  pairing: false,
  error: null,

  probe: async () => {
    // 回环免鉴权：壳里不存在「未配对」这个状态，一次请求都不必发。
    if (inTauri()) {
      set({ status: 'paired', error: null });
      return;
    }
    set({ status: 'checking' });
    try {
      await http.get(EP.auth.devices());
      set({ status: 'paired', error: null });
    } catch (e) {
      if (e instanceof ApiError && e.status === 404) {
        set({ status: 'unpaired', error: null });
      } else if (getDeviceToken()) {
        // 有 token 但连不上（网络/地址问题）：不当作未配对，交给业务层报超时。
        set({ status: 'paired', error: null });
      } else {
        set({ status: 'unpaired', error: null });
      }
    }
  },

  noteApiError: (e) => {
    if (inTauri()) return;
    // 业务接口回 404 ⇒ 网关判定未鉴权（token 缺失或已失效）⇒ 回配对页。
    if (e instanceof ApiError && e.status === 404) {
      clearDeviceToken();
      set({ status: 'unpaired' });
    }
  },

  pair: async (server, code, name) => {
    const base = normalizeBase(server);
    const trimmed = (code ?? '').trim();
    if (!base) {
      set({ error: t('请先填写电脑端的服务器地址（例如 http://100.x.x.x:8943）。') });
      return false;
    }
    if (!/^\d{6}$/.test(trimmed)) {
      set({ error: t('配对码是 6 位数字，请照电脑上显示的完整输入。') });
      return false;
    }
    set({ pairing: true, error: null });
    const ctrl = new AbortController();
    const timer = window.setTimeout(() => ctrl.abort(), PAIR_TIMEOUT_MS);
    try {
      const res = await fetch(`${base}${EP.auth.pair()}`, {
        method: 'POST',
        signal: ctrl.signal,
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ code: trimmed, name: name?.trim() || undefined }),
      });
      if (!res.ok) {
        const body = await res.json().catch(() => null);
        throw new ApiError(res.status, body);
      }
      const data = (await res.json()) as { token?: string };
      const token = typeof data.token === 'string' ? data.token : '';
      if (!token) {
        set({ pairing: false, error: t('后端没有下发设备令牌，配对未完成。请更新电脑端再试。') });
        return false;
      }
      setDeviceToken(token);
      // 地址落盘后再重载：BASE_URL 只在模块加载时解析一次，重载才会生效。
      saveBase(base);
      set({ pairing: false, status: 'paired', error: null });
      window.location.reload();
      return true;
    } catch (e) {
      const msg =
        ctrl.signal.aborted
          ? t('配对请求超时——地址可能不对或服务没在跑。')
          : describePairFailure(e);
      set({ pairing: false, error: msg });
      return false;
    } finally {
      window.clearTimeout(timer);
    }
  },

  unpair: () => {
    clearDeviceToken();
    set({ status: 'unpaired', error: null });
  },
}));
