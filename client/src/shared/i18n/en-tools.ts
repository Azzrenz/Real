/** Tool-facing English: the tool-name table, status phrasing, and the backend error-code hints.
 *
 *  Split out from en.ts for the same reason as en-settings.ts -- one dictionary file per department
 *  keeps each under its length limit and lets a department be edited without touching the others.
 *
 *  These keys are the Chinese source text verbatim. Several reach t() through a variable rather
 *  than a literal -- t(toolActionZh(name)), t(STATUS_ZH[status]), t(zh) inside the error-hint table
 *  -- so grepping the codebase for t('...') will NOT find them. Add them here by hand. */

export const enTools: Record<string, string> = {
  // ── Tool names: TOOL_LABEL_ZH, plus the fallbacks in toolActionZh ──────────
  '读取': 'Read',
  '写入': 'Write',
  '编辑': 'Edit',
  '执行': 'Run',
  '搜索': 'Search',
  '查找': 'Find',
  '列出': 'List',
  '审计项目': 'Audit project',
  '验证': 'Verify',
  '请你定夺': 'Your call',
  '检查环境': 'Check environment',
  '查询数据': 'Query data',
  '抓取网页': 'Fetch page',
  '联网搜索': 'Web search',
  '诊断': 'Diagnose',
  '收尾': 'Wrap up',
  '查询进程活度': 'Check process',
  '查询': 'Query',
  '安装': 'Install',
  '抓取': 'Fetch',
  '收录视频': 'Ingest video',
  '转写视频': 'Transcribe video',
  '收录背景图': 'Ingest background image',
  '创建视频任务': 'New video task',
  '跑视频生产线': 'Run video pipeline',
  '自动配字幕': 'Auto subtitles',
  '查视频队列': 'Video queue',
  '列视频模板': 'List video templates',
  '按模板出片': 'Render from template',

  // ── Tool card status: the state word, then the four phrase templates ──────
  '进行中': 'In progress',
  '已完成': 'Done',
  '完成但有异常': 'Done with warnings',
  '失败': 'Failed',
  '已取消': 'Cancelled',
  // The action name is interpolated, so each template carries the whole sentence rather than being
  // assembled from a verb -- English cannot follow the Chinese word order for these.
  '正在{action}{suffix}': '{action} — in progress{suffix}',
  '已{action}{suffix}': '{action} — done{suffix}',
  '{action}已取消{suffix}': '{action} — cancelled{suffix}',
  '{action}失败{suffix}': '{action} — failed{suffix}',
  '{status}：{title}': '{status}: {title}',

  // ── Tool detail: summary lines the salvage layer emits in Chinese and ToolDetail re-renders ──
  '{dirs} 目录 · {files} 文件': '{dirs} dirs · {files} files',
  '修改 {n} 处': '{n} changes',
  ' · 本次读 {s}–{e} 行': ' · read {s}–{e}',

  // ── Backend error codes: what went wrong AND what to do about it ──────────
  '要改的位置没找到——原文可能已被前面的修改改变，换行号模式重试':
    'The text to change was not found — an earlier edit probably changed it; retry with line numbers',
  '匹配到多处相同内容——带上更长的上下文重新定位':
    'Multiple identical matches — add more surrounding context to pin down the right one',
  '行号超出文件范围——文件比预期的短，先重新读一遍再改':
    'Line number past the end of the file — re-read it first, then edit',
  '给出的原文对不上——文件当前内容与预期不同，重新读取后再改':
    'The quoted text does not match — the file differs from what was expected; re-read it before editing',
  '照给出的位置改完没有变化——old 与 new 可能写成了同一个内容':
    'Nothing changed at that position — old and new may hold the same text',
  '这个文件还没读过就改——先读一遍拿到最新内容再发编辑':
    'This file was never read before editing — read it first to get the current content',
  '文件在编辑期间又被改动过——基于最新内容重新发一次':
    'The file changed while it was being edited — re-read and send the edit again',
  '必填内容缺失——把要查找/替换的内容补全再发':
    'Required content missing — fill in what to find and what to replace',
  '参数取值不对——按工具要求调整后重试':
    'Invalid argument value — adjust it to what the tool expects and retry',
  '需要绝对路径——把路径补全成完整路径再发':
    'An absolute path is required — expand it to the full path and resend',
  '目标是个目录不是文件——换成具体文件路径':
    'The target is a directory, not a file — point at a specific file',
  '目标不是目录——检查路径写法':
    'The target is not a directory — check how the path is written',
  '目标路径不存在——检查目录和文件名':
    'That path does not exist — check the directory and file name',
  '没有写入权限——检查文件只读属性或访问权限':
    'No write permission — check the read-only flag and the file permissions',
  '被安全护栏拦下——这是受保护的操作，确认合规后换方式进行':
    'Blocked by the safety guardrail — this is a protected operation; pick another approach',
  '等待确认——在弹出的确认卡上点同意后继续':
    'Waiting for confirmation — approve it on the card and the task continues',
  '改完的代码括号不平衡——语法护栏拦下了这次写入，检查配对后重试':
    'Unbalanced brackets after the edit — the syntax guardrail blocked the write; fix the pairing and retry',
  '写入后自检没过——改完的内容与预期不符，已回退，重新发一次编辑':
    'Post-write self-check failed — the result did not match; it was rolled back, send the edit again',
  '写入成功但读回不一致——文件已落盘且未回滚，先 read 确认实际内容再决定':
    'Write succeeded but the read-back differs — the file IS on disk and was NOT rolled back; read it before deciding',
  '内容带了包装壳——去掉外层说明文字，只发纯文件内容':
    'The content arrived wrapped in commentary — send the bare file content only',
  '操作意图与工具不匹配——换合适的工具或调整参数':
    'The intent does not match the tool — use a different tool or adjust the arguments',
  '多行内容要用多行模式替换——调整参数再试':
    'Multi-line content needs the multi-line replace mode — adjust the arguments and retry',
  '整个文件刚读过——直接基于已读内容修改，不必重复读':
    'The whole file was just read — edit from what you already have instead of re-reading',
  '执行超时': 'Timed out',
  '命令执行失败——看输出里的报错定位原因':
    'The command failed — read the error in its output to find the cause',
  '文件读写失败——检查文件是否被占用或路径是否正确':
    'File read/write failed — check whether the file is locked and whether the path is right',
  '网络或服务连接失败——稍后重试':
    'Network or service connection failed — retry shortly',

  // ── Right dock: shell, file tabs and the workspace tree ────────────────────
  '文件树': 'File tree',
  '空目录': 'Empty folder',
  '未保存': 'Unsaved',
  '已保存': 'Saved',
  '加载编辑器…': 'Loading editor…',
};
