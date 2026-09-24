// English strings for the settings panel, the scheduled-task panel and the right dock.

export const enSettings: Record<string, string> = {
  // ── Settings: model and credentials ──────────────────────────────────
  模型: 'Model',
  模型名: 'Model name',
  '自定义…': 'Custom…',
  '当前模型不在列表（历史自定义）；输入已注册模型名即可回到下拉选择':
    'This model is not in the list (a custom entry from earlier); type a registered model name to get the dropdown back',
  '凭据与端点': 'Credentials and endpoint',
  '智谱 GLM': 'Zhipu GLM',
  可达: 'Reachable',
  '连接失败：{err}': 'Connection failed: {err}',
  '已连接（real）{extra}': 'Connected (real){extra}',
  '当前状态：{mode}。填写 API Key 保存后自动启用真实调用（real）；清空 Key 自动回 Mock 演示模式。':
    'Current state: {mode}. Saving an API key turns on live calls automatically; clearing it falls back to mock demo mode.',

  // ── Settings: conversation behaviour ─────────────────────────────────
  '思考模式': 'Thinking',
  模式: 'Mode',
  '简单问答（短输入/无代码意图）自动关闭思考省 token；修复/重构类任务自动开启。':
    'Short questions with no code intent skip thinking to save tokens; fix and refactor work switches it on.',
  '思考强度（开启时）': 'Thinking effort (when on)',
  '强度越高思考越深入、响应越慢越贵。DeepSeek 官方档位 low/high/max（medium 会被映射为 high）；GLM 官方推荐 max。日常建议「低」，复杂重构临时「最大」。':
    'Higher effort means deeper thinking and slower, pricier replies. DeepSeek offers low/high/max officially (medium maps to high); GLM recommends max. Use low day to day, and max for a hard refactor.',
  '操作确认规则': 'Confirmation rules',
  '危险操作自动放行': 'Auto-approve dangerous operations',
  '总开关，与输入框底部那个小按钮是同一个状态。开启后删除 / 敏感写入不再弹窗，直接批准；单类规则里设成「自动拒绝」的优先级更高，仍会拦住。':
    'Master switch — the same state as the small button under the composer. When on, deletes and sensitive writes are approved without prompting; a per-action “auto-deny” rule still wins and blocks them.',
  '本会话（{id}）': 'This session ({id})',
  '清除{action}的本会话规则': 'Clear this session rule for {action}',
  '默认系统提示词': 'Default system prompt',
  '新建会话时自动带入；留空沿用内置执行伙伴规则。':
    'Carried into every new session; leave it empty to keep the built-in execution-partner rules.',
  '长期记忆（主题 / 偏好 / 画像）': 'Long-term memory (topics / preferences / profile)',

  // ── Settings: service status and data directory ──────────────────────
  '后端地址': 'Backend address',
  未连接: 'Not connected',
  '运行模式': 'Run mode',
  'real（真实调用）': 'real (live calls)',
  'mock（演示）': 'mock (demo)',
  '当前模型': 'Current model',
  '可选模型': 'Available models',
  '{n} 个': '{n}',
  '已配置（{masked}）': 'Configured ({masked})',
  说明: 'Notes',
  '清除失败': 'Clear failed',
  '（后端未连接）': '(backend not connected)',
  '已保存——重启 Real 后生效，数据将迁移到新目录':
    'Saved — takes effect once Real restarts, and the data is migrated to the new directory',
  '保存失败：{err}': 'Save failed: {err}',
  '数据目录': 'Data directory',
  '日志 / spill / 数据库存放根': 'Root for logs / spill / database',
  '留空 = 默认（{dir}）': 'Empty = default ({dir})',

  // ── Settings: tuning ─────────────────────────────────────────────────
  '调优档位': 'Presets',
  '高级参数': 'Advanced parameters',
  '我清楚这些参数的作用，允许修改': 'I understand what these do — allow editing',
  '轮次与预算': 'Rounds and budget',
  '一个回合最多跑几轮；到顶由系统终止并按未完成归档。范围 1–200；默认 {n}。':
    'How many rounds one turn may run. At the cap the system stops it and archives it as unfinished. Range 1–200; default {n}.',
  '单次调用的上下文硬上限，超过就压缩历史腾地方。归档启动线是本值的 33%。最低 10000；默认 {n}。':
    'Hard context cap per call — past it the history is compacted to make room. The archive trigger sits at 33% of this value. Minimum 10000; default {n}.',
  '输出单价最贵，收小是有效的省钱手段。范围 1024–1000000；默认 {n}。':
    'Output is the priciest line item, so trimming it really does save money. Range 1024–1000000; default {n}.',
  '上下文压缩': 'Context compaction',
  '（一发 0.008 元变 0.15 元）。所以判断标准不是省下多少字节，而是':
    ' (￥0.008 becomes ￥0.15 per call). So the test is not how many bytes you save, but whether',
  '「省下的字节」值不值「作废一次缓存」。': 'the bytes saved are worth voiding a cache.',
  '本回合内保留最近这几条工具输出原文，更早的压成头部摘要。给多 = 少动历史。范围 1–50；默认 {n}。':
    'Keep this many recent tool outputs verbatim within the turn; earlier ones collapse to their head. More = less history churn. Range 1–50; default {n}.',
  '想几乎不压就填上限 4000000。范围 128–4000000；默认 {n}。':
    'Use the 4000000 cap if you would rather almost never compact. Range 128–4000000; default {n}.',
  '除最近这几个回合，更早的回合正文整段换成一条摘要。给多 = 少动历史。范围 2–200；默认 {n}。':
    'Turns older than the last few have their body text replaced by a single summary. More = less history churn. Range 2–200; default {n}.',
  '它要大于窗口正常装满的量，否则每轮都在归档、每轮都赔。最小 1000；默认 {n}。':
    'It has to exceed what the window normally fills, or every round archives and every round pays. Minimum 1000; default {n}.',
  '记忆压缩': 'Memory compaction',
  '本工作区攒到这么多条记忆才开始考虑压缩。至少 2；默认 {n}。':
    'Compaction is only considered once the workspace holds this many memories. At least 2; default {n}.',
  '压缩时最近这几条原文保留不动，更早的压成摘要。至少 1；默认 {n}。':
    'During compaction the most recent entries stay verbatim and older ones become summaries. At least 1; default {n}.',
  '记忆累计字符数低于它就不压。至少 1000；默认 {n}。':
    'Below this many accumulated characters nothing is compacted. At least 1000; default {n}.',
  '生效方式': 'How changes apply',

  '首次使用 {p}：粘贴一次该厂商的 API Key（{url} 获取，仅存本机后端，之后一键互切）':
    'First time with {p}: paste that vendor’s API key once ({url}) — it stays in the local backend, and after that you can switch between them in one click.',

  // ── Settings: scheduled tasks ────────────────────────────────────────
  '新建定时任务': 'New scheduled task',
  '任务名称': 'Task name',
  '提示词（触发时当作一条消息发出）': 'Prompt (sent as one message when it fires)',
  '调度时间（cron 或自然语言）': 'Schedule (cron or plain language)',
  '支持 5 字段 cron（': 'Five-field cron (',
  '）或自然语言（': ') or plain language (',
  '每天早上 9 点': 'every day at 9am',
  '），由后端解析。': '), parsed by the backend.',
  '已有任务': 'Existing tasks',
  '（{n} 个）': ' ({n})',
  '还没有定时任务——建一个，让 Real 按点干活。':
    'No scheduled tasks yet — create one and let Real work to the clock.',
  ' · 下次 {time}': ' · next {time}',
  ' · 上次 {time}': ' · last {time}',
  '立即运行 {name}': 'Run {name} now',
  '删除 {name}': 'Delete {name}',
  '名称、提示词、调度时间三项都要填': 'Name, prompt and schedule are all required',
  '创建失败：{err}': 'Could not create: {err}',
  '定时任务已创建': 'Scheduled task created',
  '触发失败': 'Could not trigger it',
  '已触发「{name}」，正在新建会话': 'Triggered “{name}” — starting a new session',

  // ── Right dock ───────────────────────────────────────────────────────
  '打开右侧栏': 'Open side panel',
  '无法在右侧栏打开：{err}': 'Could not open it in the side panel: {err}',
  侧栏: 'Side panel',
  '与主窗口分离': 'Detach from the main window',
  '吸附到主窗口': 'Snap to the main window',
  '收起列表': 'Collapse list',
  '展开列表': 'Expand list',
  返回: 'Back',
  '当前任务': 'Current task',
  任务: 'Task',
  未命名: 'Untitled',
  '已进行': 'Elapsed',
  改动: 'Changes',
  '还没有改动': 'nothing yet',
  '{n} 个文件': '{n} files',
  '当前没有打开的任务': 'No task is open',
  '改动过的文件': 'Files changed',
  '记忆与工作日志': 'Memory and work logs',
  '长期记忆': 'Long-term memory',
  '还没有日志': 'No logs yet',
  '读取中…': 'Loading…',
  文件: 'Files',
  目录: 'Folders',
  占用: 'Size',
  '文件类型': 'File types',
  '扫描中…': 'Scanning…',
  '还没有产出': 'Nothing produced yet',
  '该文件不是文本，暂不支持预览': 'This file is not text, so it cannot be previewed',
  ' · 第 {n} 次': ' · attempt {n}',
  '还没有技能。技能 = 一份工作流话术（skills/<名>/SKILL.md），':
    'No skills yet. A skill is a workflow script (skills/<name>/SKILL.md),',
  'API Key（{p}）': 'API key ({p})',
  'GLM Key': 'GLM key',
  '{note} · HTTP {status} · {ms}ms': '{note} · HTTP {status} · {ms}ms',
};
