
export const TOOL_LABEL_ZH: Record<string, string> = {
  read: '读取',
  write: '写入',
  edit: '编辑',
  modify: '编辑',
  run: '执行',
  search: '搜索',
  find_files: '查找',
  list: '列出',
  audit: '审计项目',
  verify: '验证',
  ask: '请你定夺',
  env: '检查环境',
  db_query: '查询数据',
  web_fetch: '抓取网页',
  web_search: '联网搜索',
  self_heal: '诊断',
  done: '收尾',
  proc_status: '查询进程活度',
  process_status: '查询进程活度',

  video_ingest: '收录视频',
  video_transcribe: '转写视频',
  video_bg_ingest: '收录背景图',
  video_new_task: '创建视频任务',
  video_run: '跑视频生产线',
  video_auto_subtitle: '自动配字幕',
  video_status: '查视频队列',
  video_templates: '列视频模板',
  video_render: '按模板出片',
};

export function toolActionZh(name: string): string {
  const known = TOOL_LABEL_ZH[name];
  if (known) return known;
  const lower = name.toLowerCase();
  if (lower.includes('proc') && lower.includes('status')) return '查询进程活度';
  if (lower.includes('status')) return '查询';
  if (lower.includes('proc')) return '查询进程活度';
  if (lower.includes('install')) return '安装';
  if (lower.includes('search')) return '搜索';
  if (lower.includes('fetch') || lower.includes('download')) return '抓取';
  if (lower.includes('list') || lower.includes('ls')) return '列出';
  return name;
}

export function parseArgs(args: unknown): Record<string, unknown> | null {
  if (!args) return null;
  const v = typeof args === 'string'
    ? (() => { try { return JSON.parse(args); } catch { return null; } })()
    : args;
  return v && typeof v === 'object' ? (v as Record<string, unknown>) : null;
}
