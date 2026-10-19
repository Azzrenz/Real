/** English for the mobile (窄屏) surfaces: pairing page, mobile status bar, drawer entry points.
 *
 *  Split into its own dictionary for the same reason as en-settings.ts / en-tools.ts /
 *  en-chat.ts: en.ts sits close to its length limit, and a new department should be editable
 *  on its own. Keys that already exist elsewhere (已完成 / 已取消 / 已中断 / 收起执行过程 …)
 *  are deliberately NOT repeated here -- a second definition would silently shadow the first.
 */
export const enMobile: Record<string, string> = {
  // ── Pairing page ─────────────────────────────────────────────────────
  连接你的电脑: 'Connect your computer',
  '在电脑上打开 Real，进入「设置 → 设备」生成一个 6 位配对码，然后在这里输入。':
    'On your computer, open Real and go to Settings → Devices to generate a 6-digit code, then enter it here.',
  '6 位配对码': '6-digit code',
  '服务器地址：{url}（点此修改）': 'Server address: {url} (tap to change)',
  未设置: 'Not set',
  电脑端服务器地址: 'Computer server address',
  '手机和电脑要在同一个网络里（常用 Tailscale）。地址就是电脑上 Real 服务的地址，含端口。':
    'The phone and the computer must be on the same network (Tailscale is common). Use the address of the Real service on the computer, including the port.',
  '连接中…': 'Connecting…',
  连接: 'Connect',
  '配对码 5 分钟内有效、只能成功一次；如果提示不对，请到电脑上重新生成一个。':
    'The code is valid for 5 minutes and works only once. If it is rejected, generate a new one on the computer.',

  // ── Pairing failures ─────────────────────────────────────────────────
  '配对码不对或已过期（配对码 5 分钟内有效，且只能成功一次）。请在电脑上重新生成一个再试。':
    'The code is wrong or has expired (it is valid for 5 minutes and works only once). Generate a new one on the computer and try again.',
  '尝试次数过多，已被暂时限速。稍等一会儿，或到电脑上重新生成配对码。':
    'Too many attempts, temporarily rate-limited. Wait a moment, or generate a new code on the computer.',
  '这个地址上没有找到 Real 服务——请确认服务器地址填对了，且电脑上的 Real 正在运行。':
    'No Real service at this address -- check that the server address is correct and that Real is running on the computer.',
  '配对失败（HTTP {s}）。请稍后重试。': 'Pairing failed (HTTP {s}). Please try again later.',
  '连不上这个服务器地址。请确认它和电脑在同一个网络（如 Tailscale）里，且地址、端口没写错。':
    'Cannot reach this server address. Make sure it is on the same network as the computer (e.g. Tailscale) and that the address and port are correct.',
  '请先填写电脑端的服务器地址（例如 http://100.x.x.x:8943）。':
    'Enter the computer\u2019s server address first (e.g. http://100.x.x.x:8943).',
  '配对码是 6 位数字，请照电脑上显示的完整输入。':
    'The pairing code is 6 digits -- enter all of them exactly as shown on the computer.',
  '后端没有下发设备令牌，配对未完成。请更新电脑端再试。':
    'The backend did not issue a device token, so pairing is incomplete. Update the desktop app and try again.',
  '配对请求超时——地址可能不对或服务没在跑。':
    'Pairing request timed out -- the address may be wrong or the service is not running.',

  // ── Mobile status bar ────────────────────────────────────────────────
  待命: 'Idle',
  等你审批: 'Waiting for you',
  执行中: 'Running',
  执行失败: 'Failed',
  正在推理: 'Reasoning',
  '¥{n}': '¥{n}',

  // ── SSE reconnect (non-loopback needs a one-time ticket) ─────────────
  没换来连接票据: 'Could not obtain a stream ticket',

  // ── Mobile chrome ────────────────────────────────────────────────────
  打开任务列表: 'Open task list',
  关闭任务列表: 'Close task list',
  '打开任务列表（有一步等你审批）': 'Open task list (one step is waiting for your approval)',
  '已连接 · {host}': 'Connected · {host}',

  // ── Mobile composer placeholders (shorter: the 390px box wraps the long ones) ──
  '描述任务，或随便聊聊…': 'Describe a task, or just chat…',
  '打字后按 Enter 插话': 'Type and press Enter to interject',

  // ── Mobile start page: quick-start pills ─────────────────────────────
  '这个项目现在是什么状态？': 'What state is this project in right now?',
  '跑一遍测试，有失败就修': 'Run the tests and fix any failures',
  '把最近的改动总结一下': 'Summarize the recent changes',
  '解释一下刚才那个报错': 'Explain that error from just now',
};
