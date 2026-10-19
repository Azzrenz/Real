/** 给 SSE URL 换一次性票据（`?ticket=`）。
 *
 *  为什么需要它：`new EventSource()` **不能自定义请求头**，所以远程连接既没法用 Bearer，
 *  也不该把长期 token 塞进 URL 查询串（会进访问日志/历史/代理）。后端的做法是
 *  `POST /api/auth/sse-ticket` 发一张绑定会话、用后即弃的短票据，SSE 端点从 `?ticket=` 读它
 *  （见 `server/src/routes/auth_guard.rs`）。本模块只负责"换票并拼查询串"这一件事。
 *
 *  桌面（回环）没有设备 token ⇒ 直接返回空串，SSE URL 与改动前逐字节一致。
 */
import { http, getDeviceToken } from '../../../services/http';
import { EP } from '../../../services/endpoints';
import { t } from '../../../shared/i18n';

/** 非回环来源（有设备 token）才需要票据；桌面回环走同步直连。 */
export function needsSseTicket(): boolean {
  return !!getDeviceToken();
}

/** 返回要拼到 SSE URL 后面的查询串（`&ticket=xxx`）；桌面回环返回空串。失败则抛错。 */
export async function sseTicketQuery(sessionId: string): Promise<string> {
  if (!needsSseTicket()) return '';
  const r = await http.post<{ ticket?: string }>(EP.auth.sseTicket(), { session_id: sessionId });
  const tk = typeof r?.ticket === 'string' ? r.ticket : '';
  if (!tk) throw new Error(t('没换来连接票据'));
  return `&ticket=${encodeURIComponent(tk)}`;
}
