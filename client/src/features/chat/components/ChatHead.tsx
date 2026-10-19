/** 会话头：任务标题（可点开改名）+ 中栏的 HeadDock（任务列表 / 历史提问）。
 *
 *  桌面专属：手机版这一整条让位给 `MobileStatusBar`（状态 + 当前步骤 + 花费更值那 36px），
 *  HeadDock 改塞进状态条右侧的图标位，标题上移到标题栏。
 *  抽成组件只是为了让 ChatView 不再增长（它已经很长），DOM 与类名与原来逐字一致。
 */
import { HeadDock } from './HeadDock';
import type { ChatIndexItem } from './ChatHistory';
import { useSessionStore } from '../../sessions/store/sessionStore';
import { useT } from '../../../shared/i18n';

interface Props {
  sessionId: string;
  sessionTitle: string;
  editingTitle: boolean;
  setEditingTitle: (v: boolean) => void;
  chatIndex: ChatIndexItem[];
  onJumpChat: (msgId: string) => void;
}

export function ChatHead({
  sessionId,
  sessionTitle,
  editingTitle,
  setEditingTitle,
  chatIndex,
  onJumpChat,
}: Props) {
  const t = useT();
  return (
    <div className="chat-head">
      {editingTitle ? (
        <input
          className="chat-title-input"
          defaultValue={sessionTitle}
          autoFocus
          onFocus={(e) => e.currentTarget.select()}
          onBlur={() => setEditingTitle(false)}
          onKeyDown={(e) => {
            if (e.nativeEvent.isComposing || e.keyCode === 229) return;
            if (e.key === 'Enter') {
              const v = e.currentTarget.value.trim();
              if (v && v !== sessionTitle) void useSessionStore.getState().renameSession(sessionId, v);
              setEditingTitle(false);
            }
            if (e.key === 'Escape') setEditingTitle(false);
          }}
          onClick={(e) => e.stopPropagation()}
        />
      ) : (
        <span
          className="chat-title"
          title={sessionTitle}
          onClick={() => setEditingTitle(true)}
        >
          {sessionTitle || t('新任务')}
        </span>
      )}
      <span className="spacer" />
      <HeadDock chatIndex={chatIndex} onJumpChat={onJumpChat} />
    </div>
  );
}
