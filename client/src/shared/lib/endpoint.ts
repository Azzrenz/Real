/** 手机端要回答「我连的是哪台电脑」。
 *
 *  遥控器场景下后端在另一台机器上（走 Tailscale），用户需要随时能确认连的是哪一台 ——
 *  所以这个标签出现在标题栏第二行和抽屉底部。桌面走回环，后端固定在 127.0.0.1，
 *  没有这个必要，返回空串即可（调用方据此整段不渲染）。
 */
import { http } from '../../services/http';
import { inTauri } from './tauri';

/** 返回 `host:port`；桌面或解析失败时返回空串。 */
export function remoteHostLabel(): string {
  if (inTauri()) return '';
  try {
    return new URL(http.baseUrl).host;
  } catch {
    return '';
  }
}
