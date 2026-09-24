
import { memo, useState } from 'react';
import { createPortal } from 'react-dom';
import { Icon, type IconName } from '../../../shared/ui/icons';
import { DetailWindow } from '../../../shared/ui/DetailWindow';
import type { ToolCard } from '../types';
import { parseArgs, toolActionZh } from '../lib/labels';
import { durationText, titleCompact, toolEntity, tidyPurpose } from '../lib/purpose';
import { openExternal } from '../../../shared/lib/openExternal';
import { systemApi } from '../../../services/domains/system';
import { useToastStore } from '../../../shared/store/toastStore';
import { ToolDetail } from './ToolDetail';
import '../tools.css';
import { useT, type T } from '../../../shared/i18n';

function toolStatusText(tool: ToolCard, purpose: string, t: T): string {
  // The action is interpolated into the phrase templates below, so it has to be translated on its
  // own -- otherwise an English template would be filled with a Chinese action name.
  const action = t(toolActionZh(tool.name));
  const target = purpose || toolEntity(tool);
  const suffix = target ? ` · ${target}` : '';
  const running = tool.status === 'running';
  const ok = tool.status === 'success' || tool.status === 'warning';
  if (running) return t('正在{action}{suffix}', { action, suffix });
  if (ok) return t('已{action}{suffix}', { action, suffix });
  if (tool.status === 'cancelled') return t('{action}已取消{suffix}', { action, suffix });
  return t('{action}失败{suffix}', { action, suffix });
}

function toolIcon(name: string): IconName {
  const n = (name || '').toLowerCase();
  if (/(read|doc|cat|open)/.test(n)) return 'doc';
  if (/(search|grep|find|glob)/.test(n)) return 'search';
  if (/(write|edit|modify|patch|insert|replace)/.test(n)) return 'pencil';
  if (/(run|exec|verify|cmd|shell|bash|test)/.test(n)) return 'play';
  if (/(list|dir|tree|ls)/.test(n)) return 'folder';
  if (/(web|fetch|http|url|browse)/.test(n)) return 'external';
  if (/(db|sql|query)/.test(n)) return 'sliders';
  return 'wrench';
}

function statusIcon(tool: ToolCard): IconName {
  if (tool.status === 'running') return 'spinner';
  return toolIcon(tool.name);
}

/** Raw Chinese source, translated where it is rendered. Calling t() at module scope would freeze
 *  the language at import time and then ignore every later switch. */
const STATUS_ZH: Record<ToolCard['status'], string> = {
  running: '进行中',
  success: '已完成',
  warning: '完成但有异常',
  error: '失败',
  cancelled: '已取消',
};

function failReasonLine(summary: string, t: T): string {
  const s = (summary ?? '').trim();
  if (!s || s.startsWith('{')) return '';
  const lines = s.split('\n').map((l) => l.trim()).filter(Boolean);
  const hit = lines.find((l) =>
    /error|Error|失败|异常|错误|原因|because|failed|Traceback|not found|denied|invalid|timeout|拒绝/i.test(l),
  );
  let line = (hit ?? lines[0] ?? '').trim();
  line = line.replace(/\[TOOL_ERROR_WITH_PREVIEW\]\s*/g, '');
  const mp = line.match(/MISSING_PARAM\]?\s*[^:：]*[:：]\s*([^\s，。]+)/i);
  if (mp) {
    return t('缺少必填参数「{p}」——这次操作没带上它，已跳过本次执行，补上后重试即可', { p: mp[1] });
  }
  const ERR_ZH: Array<[RegExp, string]> = [
    [/FIND_NOT_FOUND/i, '要改的位置没找到——原文可能已被前面的修改改变，换行号模式重试'],
    [/FIND_AMBIGUOUS|OCCURRENCE/i, '匹配到多处相同内容——带上更长的上下文重新定位'],
    [/LINE_NOT_FOUND/i, '行号超出文件范围——文件比预期的短，先重新读一遍再改'],
    [/OLD_TEXT_MISMATCH/i, '给出的原文对不上——文件当前内容与预期不同，重新读取后再改'],
    [/MODIFY_NO_CHANGE|NO_MATCH/i, '照给出的位置改完没有变化——old 与 new 可能写成了同一个内容'],
    [/EDIT_NEEDS_READ/i, '这个文件还没读过就改——先读一遍拿到最新内容再发编辑'],
    [/EDIT_CONFLICT/i, '文件在编辑期间又被改动过——基于最新内容重新发一次'],
    [/MODIFY_FIND_EMPTY|EMPTY_CONTENT|REPLACEMENTS_REQUIRED|REPLACEMENT_REQUIRED|ARGS_REQUIRED/i, '必填内容缺失——把要查找/替换的内容补全再发'],
    [/INVALID_PARAM/i, '参数取值不对——按工具要求调整后重试'],
    [/RELATIVE_PATH/i, '需要绝对路径——把路径补全成完整路径再发'],
    [/IS_DIRECTORY/i, '目标是个目录不是文件——换成具体文件路径'],
    [/NOT_DIRECTORY/i, '目标不是目录——检查路径写法'],
    [/PARENT_NOT_FOUND|PATH_NOT_FOUND|NOT_FOUND|不存在|找不到/i, '目标路径不存在——检查目录和文件名'],
    [/PERMISSION_DENIED|拒绝/i, '没有写入权限——检查文件只读属性或访问权限'],
    [/SENSITIVE_FILE|SECURITY_PATTERN|COMMAND_NOT_ALLOWED/i, '被安全护栏拦下——这是受保护的操作，确认合规后换方式进行'],
    [/REQUIRES_CONFIRM/i, '等待确认——在弹出的确认卡上点同意后继续'],
    [/MODIFY_BRACE_UNBALANCED/i, '改完的代码括号不平衡——语法护栏拦下了这次写入，检查配对后重试'],
    [/VERIFY_FAILED|MODIFY_VERIFY_FAIL|WRITE_VERIFY_FAILED/i, '写入后自检没过——改完的内容与预期不符，已回退，重新发一次编辑'],
    // Split into its own entry (fixed 2026-09-15): the backend reports these two as "file
    // written, not rolled back" (POST_EDIT_INVALID / POST_EDIT_VERIFY_READ_FAILED in
    // fs_write.rs). Merged with the entry above they read as "rolled back" -- the opposite of
    // the real behaviour, which lets a model assume the file was restored and keep editing on a
    // false premise.
    [/POST_EDIT_INVALID|POST_EDIT_VERIFY_READ_FAILED/i, '写入成功但读回不一致——文件已落盘且未回滚，先 read 确认实际内容再决定'],
    [/CONTENT_IS_ENVELOPE/i, '内容带了包装壳——去掉外层说明文字，只发纯文件内容'],
    [/INTENT_MISMATCH/i, '操作意图与工具不匹配——换合适的工具或调整参数'],
    [/MULTI_LINE_REPLACE_SINGLE/i, '多行内容要用多行模式替换——调整参数再试'],
    [/DUPLICATE_FULL_READ/i, '整个文件刚读过——直接基于已读内容修改，不必重复读'],
    [/TIMEOUT/i, '执行超时'],
    [/RUN_FAILED|EXEC_FAILED|TEST_FAILED/i, '命令执行失败——看输出里的报错定位原因'],
    [/READ_FAILED|WRITE_FAILED/i, '文件读写失败——检查文件是否被占用或路径是否正确'],
    [/NETWORK_ERROR|DB_CONNECT_FAIL/i, '网络或服务连接失败——稍后重试'],
  ];
  let matched = false;
  for (const [re, zh] of ERR_ZH) {
    if (re.test(line)) { line = t(zh); matched = true; break; }
  }
  if (!matched) line = line.replace(/^\[[A-Z_]{3,}\]\s*/, '');
  if (!line) return '';
  return line.length > 48 ? `${line.slice(0, 48)}…` : line;
}


export const ToolCardView = memo(function ToolCardView({
  tool,
  repeatNo,
  purposeOverride,
}: {
  tool: ToolCard;
  repeatNo?: number;
  purposeOverride?: string;
}) {
  // Subscribing here matters: the component is memo()'d, so a language switch re-renders the parent
  // but hands down identical props -- without this the card would keep its original language.
  const t = useT();
  const failed = tool.status === 'error';
  const warning = tool.status === 'warning';
  const running = tool.status === 'running';
  const [open, setOpen] = useState(false);
  const [pathTip, setPathTip] = useState<{ left: number; top: number } | null>(null);
  const isCmd = tool.name === 'run' || tool.name === 'verify';
  const intentReason = (tool.reason ?? '').trim().replace(/\*+/g, '').replace(/`+/g, '').trim();
  const usableReason = intentReason.length >= 8 ? intentReason : '';
  const overrideText = tidyPurpose((purposeOverride ?? '').trim());
  const purpose = isCmd ? titleCompact(usableReason || (overrideText.length >= 6 ? overrideText : '')) : '';
  const title = toolStatusText(tool, purpose, t);
  const CODE_TOKEN = /\b[a-z][a-z0-9]*\.[a-z]{1,4}\b|[\w.-]+\/[\w./-]*|\b[a-z_][a-z0-9_]*\b/g;
  const parts: Array<{ t: string; green: boolean }> = [];
  let cursor = 0;
  for (const m of title.matchAll(CODE_TOKEN)) {
    const i = m.index ?? 0;
    if (i > cursor) parts.push({ t: title.slice(cursor, i), green: false });
    parts.push({ t: m[0], green: true });
    cursor = i + m[0].length;
  }
  if (cursor < title.length) parts.push({ t: title.slice(cursor), green: false });
  const repeatText = repeatNo && repeatNo > 1 ? t(' · 第 {n} 次', { n: repeatNo }) : '';
  const durText = durationText(running ? tool.elapsedMs : tool.durationMs, t);
  const argsObj = parseArgs(tool.args);
  // The title carries the basename only -- hover it to see where the file actually is.
  // Commands are excluded: their green tokens are words from the sentence, not a path.
  const filePath =
    !isCmd && typeof tool.path === 'string' && tool.path.trim() ? tool.path.trim() : '';
  /** Clicking a filename reveals it in the OS file manager. Failures are surfaced, never
   *  swallowed: a silent no-op here reads as "the click does nothing". */
  const openInFolder = () => {
    if (!filePath) return;
    void systemApi
      .reveal(filePath)
      .catch((err) => useToastStore.getState().show((err as Error).message, 'error'));
  };
  const fetchUrl = tool.name === 'web_fetch' && typeof argsObj?.url === 'string' ? argsObj.url : '';
  const reasonLine = failReasonLine(tool.summary ?? '', t);
  const note =
    reasonLine ||
    (warning && typeof tool.exitCode === 'number' && tool.exitCode !== 0
      ? t('命令跑完了，退出码 {code}（非零 = 命令自己报了错，结果不一定可用）', { code: tool.exitCode })
      : '');
  return (
    <div className={`flow-row tool-card${running ? ' running' : ''}${failed ? ' error' : ''}${warning ? ' warn' : ''}${tool.status === 'cancelled' ? ' cancelled' : ''}${open ? ' open' : ''}`}>
      {pathTip &&
        createPortal(
          <div className="flow-path-tip" style={{ left: pathTip.left, top: pathTip.top }}>
            {filePath}
          </div>,
          document.body,
        )}
      <button
        className="flow-row-main"
        onClick={() => setOpen(!open)}
        aria-expanded={open}
        aria-label={t('{status}：{title}', { status: t(STATUS_ZH[tool.status]), title })}
      >
        <Icon
          name={statusIcon(tool)}
          size={14}
          className={`flow-row-icon${running ? ' tool-spin' : ''}`}
        />
        <span className="flow-row-title">
          {parts.length > 0 && parts.some((p) => p.green) ? (
            parts.map((p, k) =>
              p.green ? (
                <span
                  key={k}
                  className="flow-row-file"
                  role="link"
                  tabIndex={0}
                  onMouseEnter={(e) => {
                    if (!filePath) return;
                    const r = e.currentTarget.getBoundingClientRect();
                    setPathTip({ left: r.left, top: r.bottom + 4 });
                  }}
                  onMouseLeave={() => setPathTip(null)}
                  // The whole row is a button that toggles details, so opening the folder
                  // must not bubble — otherwise clicking the name would also expand the card.
                  onClick={(e) => {
                    e.stopPropagation();
                    openInFolder();
                  }}
                  onKeyDown={(e) => {
                    if (e.key !== 'Enter') return;
                    e.stopPropagation();
                    openInFolder();
                  }}
                >
                  {p.t}
                </span>
              ) : (
                <span key={k}>{p.t}</span>
              ),
            )
          ) : (
            title
          )}
          {repeatText && <span className="flow-row-reason">{repeatText}</span>}
          {durText && <span className="flow-row-reason"> · {durText}</span>}
        </span>
        {fetchUrl && (
            <span
              className="tool-open-web"
              role="button"
              tabIndex={0}
              title={t('在浏览器打开：{url}', { url: fetchUrl })}
              aria-label={t('在浏览器打开该网页')}
              onClick={(e) => {
                e.stopPropagation();
                void openExternal(fetchUrl);
              }}
              onKeyDown={(e) => {
                if (e.key === 'Enter' || e.key === ' ') {
                  e.stopPropagation();
                  void openExternal(fetchUrl);
                }
              }}
            >
              <Icon name="external" size={13} />
            </span>
          )}
          <Icon name="chevron" size={14} className={`flow-row-caret${open ? ' open' : ''}`} />
        </button>
        {(failed || warning) && note && (
          <div className="tool-fail-note">
            <span className="tool-fail-label">{failed ? t('失败原因') : t('注意')}</span>
            {note}
          </div>
        )}
        {open && (
          <DetailWindow>
            <ToolDetail tool={tool} />
          </DetailWindow>
        )}
    </div>
  );
});
