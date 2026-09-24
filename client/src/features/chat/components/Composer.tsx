import { useEffect, useRef, useState } from 'react';
import { SkillPicker, preloadSkills } from './SkillPicker';
import { Icon } from '../../../shared/ui/icons';
import { useT } from '../../../shared/i18n';
import { ModelPicker } from '../../settings/components/ModelPicker';
import { useToastStore } from '../../../shared/store/toastStore';
import { useSettings } from '../../settings/store/settingsStore';
import { MAX_ATTACHMENTS, mergeAttachments, type Attachment } from '../lib/attachments';
import './composer.css';

/** Textarea max height - also the overflow line for pasted text: a paste that needs more room
 *  than this becomes a .txt attachment instead of filling the box.
 *  Fixed rather than a share of the window, so the drag ceiling and this line always agree. */
const INPUT_H_MAX = 480;

/** Textarea baseline height, doubling as the drag lower bound — the two must match,
 *  otherwise the box jumps on the first drag. Paired with `.chat-input-box textarea { height }`
 *  in composer.css: change one, change the other.
 */
const INPUT_H_BASE = 84;

/** Auto-grow ceiling: the box follows the text up to here, then scrolls. Kept well below
 *  INPUT_H_MAX (the paste-to-attachment line) and mirrored by `.chat-input-box textarea
 *  { max-height }` in composer.css: change one, change the other.
 */
const INPUT_H_GROW_MAX = 164;

/** Fallback for overflowsInput() when the textarea is not mounted and cannot be measured. */
const PASTE_AS_FILE_CHARS = 1000;

/** Would this text need more room than the box can ever show?
 *  Measured on an offscreen probe that copies the live textarea's metrics, so it is a real
 *  wrap calculation rather than a character-count guess. Only ever asked about text arriving
 *  through a paste event: whatever the user types or dictates stays in the box no matter how
 *  long it gets - telling those two sources apart is the whole point. */
function overflowsInput(text: string): boolean {
  const ta = document.querySelector<HTMLTextAreaElement>('.chat-input-box textarea');
  if (!ta) return text.length > PASTE_AS_FILE_CHARS;
  const cs = getComputedStyle(ta);
  const probe = document.createElement('textarea');
  probe.style.position = 'absolute';
  probe.style.left = '-9999px';
  probe.style.visibility = 'hidden';
  probe.style.width = `${ta.clientWidth}px`;
  probe.style.font = cs.font;
  probe.style.lineHeight = cs.lineHeight;
  probe.style.letterSpacing = cs.letterSpacing;
  probe.style.padding = cs.padding;
  probe.style.border = 'none';
  probe.style.boxSizing = cs.boxSizing;
  probe.style.whiteSpace = 'pre-wrap';
  probe.style.wordBreak = 'break-word';
  probe.value = text;
  document.body.appendChild(probe);
  const h = probe.scrollHeight;
  probe.remove();
  return h > INPUT_H_MAX;
}

interface Props {
  value: string;
  onChange: (v: string) => void;
  attachments: Attachment[];
  onAttachmentsChange: (list: Attachment[]) => void;
  onSend: () => void;
  onCancel: () => void;
  onKeyDown: (e: React.KeyboardEvent<HTMLTextAreaElement>) => void;
  inputRef: React.Ref<HTMLTextAreaElement>;
  busy: boolean;
  sending: boolean;
  showJumpDown: boolean;
  onJumpToBottom: () => void;
  onManageSkills: () => void;
  pickedSkill: string[];
  onPickSkill: (name: string) => void;
  onRemoveSkill: (name: string) => void;
  /** Idle label on the send button: the start page calls it "开始", a task calls it "发送". */
  sendLabelIdle?: string;
}

export function Composer({
  value,
  onChange,
  attachments,
  onAttachmentsChange,
  onSend,
  onCancel,
  onKeyDown,
  inputRef,
  busy,
  sending,
  showJumpDown,
  onJumpToBottom,
  onManageSkills,
  pickedSkill,
  onPickSkill,
  onRemoveSkill,
  sendLabelIdle,
}: Props) {
  const t = useT();
  const [dragOver, setDragOver] = useState(false);
  const [skillsOpen, setSkillsOpen] = useState(false);
  const [skillRect, setSkillRect] = useState<DOMRect | null>(null);
  const skillBtnRef = useRef<HTMLButtonElement>(null);
  const showToast = useToastStore((s) => s.show);
  const autoApprove = useSettings((s) => s.autoApproveDanger);
  const toggleAutoApprove = useSettings((s) => s.toggleAutoApproveDanger);

  const fileRef = useRef<HTMLInputElement>(null);

  const addFiles = async (files: File[]) => {
    if (files.length === 0) return;
    const { list, rejected } = await mergeAttachments(attachments, files);
    onAttachmentsChange(list);
    for (const r of rejected) showToast(r, 'error');
  };

  const canSend = (!!value.trim() || attachments.length > 0) && !busy && !sending;

  const interjectReady = busy && !!value.trim();
  const doSend = () => {
    if (interjectReady) {
      onSend();
      return;
    }
    if (!canSend) return;
    onSend();
  };

  const [resizeH, setResizeH] = useState<number | null>(null);

  // Follow the text up to the ceiling, then scroll. A manual drag (resizeH) owns the height
  // once used, so auto-grow steps aside rather than fighting it.
  useEffect(() => {
    const ta = document.querySelector<HTMLTextAreaElement>('.chat-input-box textarea');
    if (!ta || resizeH) return;
    ta.style.height = 'auto';
    ta.style.height = `${Math.min(Math.max(ta.scrollHeight, INPUT_H_BASE), INPUT_H_GROW_MAX)}px`;
  }, [value, resizeH]);

  const startResize = (e: React.MouseEvent) => {
    e.preventDefault();
    const startY = e.clientY;
    const ta = (e.currentTarget.closest('.chat-input-box')?.querySelector('textarea') as HTMLTextAreaElement | null);
    const startH = ta?.getBoundingClientRect().height ?? INPUT_H_BASE;
    const onMove = (ev: MouseEvent) => {
      const delta = startY - ev.clientY;
      setResizeH(Math.min(INPUT_H_MAX, Math.max(INPUT_H_BASE, startH + delta)));
    };
    const onUp = () => {
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('mouseup', onUp);
      document.body.classList.remove('is-resizing-h');
    };
    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp);
    document.body.classList.add('is-resizing-h');
  };

  return (
    <div
      className={`chat-input-wrap${dragOver ? ' drag-over' : ''}`}
      onDragOver={(e) => {
        e.preventDefault();
        setDragOver(true);
      }}
      onDragLeave={(e) => {
        if (e.currentTarget.contains(e.relatedTarget as Node)) return;
        setDragOver(false);
      }}
      onDrop={(e) => {
        e.preventDefault();
        setDragOver(false);
        void addFiles([...e.dataTransfer.files]);
      }}
    >
      {showJumpDown && (
        <button
          type="button"
          className="jump-down"
          onMouseDown={(e) => e.preventDefault()}
          onClick={onJumpToBottom}
          aria-label={t('一键到底')}
        >
          <Icon name="chevron-down" size={20} strokeWidth={2} />
        </button>
      )}
      <div className="input-drag-handle" onMouseDown={startResize} title={t('拖动调整输入区高度')}>
        <span className="input-drag-grip" aria-hidden />
      </div>
      <div className="chat-input-box">
        {dragOver && (
          <div className="attach-drop-hint">
            <Icon name="doc" size={16} />
            {t('松手即添加附件（最多 {n} 个）', { n: MAX_ATTACHMENTS })}
          </div>
        )}

        {pickedSkill.length > 0 && (
          <div className="compose-skill-row">
            {pickedSkill.map((n) => (
              <span key={n} className="compose-skill-chip" title={t('已挂技能：{name}', { name: n })}>
                <Icon name="flame" size={12} className="compose-skill-icon" />
                <span className="compose-skill-name">{n}</span>
                <button
                  type="button"
                  className="compose-skill-x"
                  onClick={() => onRemoveSkill(n)}
                  title={t('移除技能')}
                  aria-label={t('移除技能 {name}', { name: n })}
                >
                  <Icon name="x" size={11} />
                </button>
              </span>
            ))}
            <span className="compose-skill-hint">
              {pickedSkill.length > 1
                ? t('本轮按这 {n} 个技能执行', { n: pickedSkill.length })
                : t('本轮按此技能执行')}
            </span>
          </div>
        )}

        {attachments.length > 0 && (
          <div className="attach-row">
            {attachments.map((a) => (
              <span className="attach-chip" key={a.id}>
                {a.kind === 'image' ? (
                  <img className="attach-img" src={a.dataUrl} alt={a.name} />
                ) : (
                  <span className="attach-file-icon">
                    <Icon name="doc" size={14} />
                  </span>
                )}
                <span className="attach-name" title={a.name}>{a.name}</span>
                <button
                  className="attach-remove"
                  onClick={() => onAttachmentsChange(attachments.filter((x) => x.id !== a.id))}
                  title={t('移除附件')}
                  aria-label={t('移除附件 {name}', { name: a.name })}
                >
                  <Icon name="x" size={12} />
                </button>
              </span>
            ))}
          </div>
        )}

        <textarea
          ref={inputRef}
          value={value}
          placeholder={
            busy
              ? t('任务运行中——输入文字按 Enter 插话（入队，下一轮自动生效）')
              : t('描述任务或随便聊聊…（可拖入/粘贴文件作为附件）')
          }
          onChange={(e) => onChange(e.target.value)}
          onKeyDown={onKeyDown}
          onPaste={(e) => {
            const files = [...e.clipboardData.files];
            if (files.length > 0) {
              e.preventDefault();
              void addFiles(files);
              return;
            }
            const text = e.clipboardData.getData('text/plain');
            // 粘贴**矢量图**：SVG 在剪贴板里是**文本**（`clipboardData.files` 为空），
            // 旧逻辑只会在它超长时包成 `.txt` —— 用户拿到的是一坨文字而不是插图，
            // 看上去就是"只能发文字、发不了附件"。识别成 SVG 就还它 `.svg` 的身份：
            // 后端按**文本 XML** 注入（见 chat.rs 的 svg 分支），模型直接读得懂图形。
            const svgSrc = text.trimStart().replace(/^<\?xml[^>]*\?>/i, '').trimStart();
            if (/^<svg[\s>]/i.test(svgSrc)) {
              e.preventDefault();
              void addFiles([
                new File([text], t('粘贴矢量图_{n}.svg', { n: text.length }), { type: 'image/svg+xml' }),
              ]);
              return;
            }
            if (text && overflowsInput(text)) {
              e.preventDefault();
              void addFiles([new File([text], t('粘贴文本_{n}字.txt', { n: text.length }), { type: 'text/plain' })]);
            }
          }}
          style={resizeH ? { height: resizeH, maxHeight: resizeH } : undefined}
        />

        <div className="chat-input-foot">
          <div className="chat-input-foot-left">
            <ModelPicker />
            <button
              type="button"
              className="btn ghost sm"
              onClick={() => fileRef.current?.click()}
              title={t('添加附件：图片 / 文档（也可直接拖入或粘贴）')}
              aria-label={t('添加附件')}
            >
              <Icon name="doc" size={14} />
            </button>
            <input
              ref={fileRef}
              type="file"
              multiple
              style={{ display: 'none' }}
              onChange={(e) => {
                // `files` 是活集合，先拷出来再清空 value —— 否则同名文件第二次选不上
                const picked = [...(e.target.files ?? [])];
                e.target.value = '';
                void addFiles(picked);
              }}
            />
            <button
              ref={skillBtnRef}
              data-skill-toggle
              className={`btn ghost sm${skillsOpen ? ' is-active' : ''}`}
              onClick={() => {
                if (skillsOpen) {
                  setSkillsOpen(false);
                  return;
                }
                const r = skillBtnRef.current?.getBoundingClientRect();
                if (r) setSkillRect(r);
                setSkillsOpen(true);
              }}
              onMouseEnter={preloadSkills}
              title={t('技能：点一条直接调用')}
              aria-label={t('技能列表')}
              aria-haspopup="menu"
              aria-expanded={skillsOpen}
            >
              {t('技能')}
            </button>
            <button
              type="button"
              className={`danger-toggle${autoApprove ? ' on' : ''}`}
              onClick={toggleAutoApprove}
              aria-pressed={autoApprove}
              title={
                autoApprove
                  ? t('危险操作自动放行中：删除/注册表/敏感写入不再弹窗，直接批准。点此恢复逐次确认')
                  : t('危险操作逐次确认中：每次删除/注册表/敏感写入都会弹窗问你。点此改为自动放行')
              }
            >
              <span className="danger-toggle-dot" aria-hidden />
              {autoApprove ? t('自动放行') : t('确认弹窗')}
            </button>
          </div>
          <div className="chat-input-foot-right">
            <button
              className={`composer-send${busy ? (interjectReady ? ' interject' : ' running') : sending ? ' sending' : ''}`}
              onClick={busy ? (interjectReady ? doSend : onCancel) : doSend}
              disabled={!busy && !canSend}
              title={busy ? (interjectReady ? t('发送插话（入队，下一轮生效）') : t('停止当前任务')) : (sendLabelIdle || t('发送'))}
              aria-label={busy ? (interjectReady ? t('发送插话') : t('停止')) : (sendLabelIdle || t('发送'))}
            >
              {busy ? (
                interjectReady ? (
                  <Icon name="send" size={13} />
                ) : (
                  <Icon name="stop" size={13} strokeWidth={2.2} />
                )
              ) : sending ? (
                <Icon name="spinner" size={13} className="icon-spin" />
              ) : (
                <Icon name="send" size={13} />
              )}
              {busy ? (interjectReady ? t('发送插话') : t('停止')) : sending ? t('发送中') : (sendLabelIdle || t('发送'))}
            </button>
          </div>
        </div>
      </div>
      {skillsOpen && skillRect && (
        <SkillPicker
          anchorRect={skillRect}
          onPick={(n) => {
            onPickSkill(n);
            setSkillsOpen(false);
            document.querySelector<HTMLTextAreaElement>('.chat-input-box textarea')?.focus();
          }}
          onManage={onManageSkills}
          onClose={() => setSkillsOpen(false)}
        />
      )}
    </div>
  );
}
