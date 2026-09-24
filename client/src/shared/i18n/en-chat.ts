/** Chat-facing English: the reply surface, the thinking rows, the right dock, and the skill dialog.
 *
 *  Same split reasoning as en-settings.ts and en-tools.ts. A few keys here are passed through a
 *  variable rather than written as t('...') at the call site -- the time formatters, the skill
 *  categories, the top-up provider names -- so they cannot be found by grepping for literals. */

export const enChat: Record<string, string> = {
  // ── Relative time (used by the dock's log list and the artifact groups) ───
  '刚刚': 'Just now',
  '{n} 分钟前': '{n} min ago',
  '{n} 小时前': '{n}h ago',
  '{n} 天前': '{n}d ago',
  '今天': 'Today',
  '昨天': 'Yesterday',
  '{m} 月 {d} 日': '{m}/{d}',

  // ── Durations and counts ─────────────────────────────────────────────────
  '{n} 行': '{n} lines',
  '{n} 字': '{n} chars',
  '{lines} 行 · {chars} 字': '{lines} lines · {chars} chars',
  '{n} 分钟': '{n} min',
  '{h} 时 {m} 分': '{h}h {m}m',
  '{m}分': '{m}m',
  '{m}分{r}秒': '{m}m{r}s',
  '还有 {blocks} 段 · {chars} 字未显示':
    '{blocks} more sections · {chars} chars hidden',
  '内容较长，已显示前 {n} 字符': 'Long content — showing the first {n} characters',
  '显示全部（{n} 字）': 'Show all ({n} chars)',
  '显示全部（{lines} 行 / {chars} 字符）': 'Show all ({lines} lines / {chars} chars)',
  '（约 {n} 字）': ' (~{n} chars)',
  '正在加载思考全文…{extra}': 'Loading the full reasoning…{extra}',

  // ── Cost line ────────────────────────────────────────────────────────────
  'API {n} 次': '{n} API calls',
  '输入 {i} · 输出 {o} · 输出花费 ¥{oc} · 未命中 {mi}': 'in {i} · out {o} · out cost ¥{oc} · miss {mi}',
  '花费构成：未命中 {a}% · 缓存 {b}% · 输出 {c}%': 'cost split: miss {a}% · cache {b}% · output {c}%',
  '缓存命中 {n}%': '{n}% cache hit',
  '共消耗 ¥{n}': 'spent ¥{n}',
  '高峰价 ×{n}': 'peak pricing ×{n}',
  '平峰价': 'off-peak pricing',
  '约 ¥{n}': '≈ ¥{n}',
  '已消耗 {tokens} tokens（{calls} 次 API 调用）':
    'Used {tokens} tokens ({calls} API calls)',
  '当前模型 {name}': 'Current model {name}',

  // ── Reply surface ────────────────────────────────────────────────────────
  '↑ 更早记录（{n} 条）· 点击或滚到顶部加载':
    '↑ {n} earlier messages · click or scroll to top to load',
  '✓ 已复制': '✓ Copied',
  '复制代码': 'Copy code',
  '没有可重新生成的消息': 'Nothing to regenerate',
  '任务运行中：文字插话才能入队（附件请等本轮结束再发）':
    'A task is running: only text can be queued (send attachments once this turn ends)',
  '插话已入队：下一轮边界生效，气泡将出现在插入点':
    'Interjection queued — it takes effect at the next turn boundary and appears at the insertion point',
  '1. 先看 hub.log 尾部 50 行…\n2. 探测 9560 /api/health…\n判完成：…':
    '1. tail the last 50 lines of hub.log…\n2. probe 9560 /api/health…\ndone when:…',

  // ── Attachments ──────────────────────────────────────────────────────────
  '{name} 读取失败': 'Could not read {name}',
  '{name} 超过 {mb}MB 上限': '{name} is over the {mb}MB limit',
  '最多只能带 {n} 个附件': 'At most {n} attachments',

  // ── Balance / top-up card ────────────────────────────────────────────────
  '账户余额不足': 'Account out of credit',
  '检测到该家欠费': 'This provider looks out of credit',
  '打开{name}充值页面': 'Open the {name} top-up page',
  'DeepSeek 开放平台': 'DeepSeek open platform',
  '智谱 GLM 开放平台': 'Zhipu GLM open platform',

  // ── Skill absorption card and the skill dialog ───────────────────────────
  '技能沉淀 · {n} 条': 'Skills to absorb · {n}',
  '预览': 'Preview',
  '写入后并入「{name}」技能': 'Will merge into the “{name}” skill',
  '写入后新建独立技能': 'Will create a new skill',
  '证据：{v}': 'Evidence: {v}',
  '每条独立写入；未点写入前不进技能库（防模型自评注水）':
    'Each one is written separately; nothing enters the skill library until you click write (guards against a model over-rating its own work)',
  '未分类': 'Uncategorised',
  // Category values are DATA -- they are stored and sent to the backend -- so the keys stay Chinese
  // and only the label shown to the user is translated.
  '架构': 'Architecture',
  '工程': 'Engineering',
  '排障': 'Troubleshooting',
  '应用': 'Applications',
  '技能名（字母/数字/中文/-/_）': 'Skill name (letters / digits / Chinese / - / _)',
  '分类（它属于哪一块，列表里显示在标题前）':
    'Category (which part it belongs to; shown before the title in the list)',
  '一句话说明（什么时候用它）': 'One-line description (when to use it)',
  '工作流正文（确定性步骤，模型照此执行）':
    'Workflow body (the deterministic steps the model follows)',
  '或直接输入工作区绝对路径，如 D:\\projects\\my-app':
    'Or type an absolute workspace path, e.g. D:\\projects\\my-app',

  // ── Settings store messages (surfaced above the save button) ─────────────
  '切换失败：{err}': 'Switch failed: {err}',
  '已切换到 {name}（即时生效）': 'Switched to {name} (takes effect immediately)',
  '清除失败：{err}': 'Clear failed: {err}',
  '删除失败': 'Delete failed',

  // ── Transport ────────────────────────────────────────────────────────────
  '请求超时（{s}s）——后端可能正在重启，请稍候，将自动重试':
    'Request timed out ({s}s) — the backend may be restarting; it will retry automatically',
};
