import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Icon } from '../../shared/ui/icons';
import { useT } from '../../shared/i18n';
import './file-tree.css';

interface FileEntry {
  name: string;
  path: string;
  is_dir: boolean;
  ext: string;
}

export function FileTree({
  activePath,
  onOpen,
}: {
  activePath: string | null;
  onOpen: (path: string) => void;
}) {
  const t = useT();
  const [drives, setDrives] = useState<FileEntry[] | null>(null);

  useEffect(() => {
    let alive = true;
    invoke<string[]>('list_drives')
      .then((ds) => {
        if (alive) {
          setDrives(ds.map((d) => ({ name: d, path: d, is_dir: true, ext: '' })));
        }
      })
      .catch(() => {
        if (alive) setDrives([]);
      });
    return () => {
      alive = false;
    };
  }, []);

  if (drives === null) return <p className="dock-msg">{t('读取中…')}</p>;

  // One tree only, rooted at "This PC": drives are the user's mental model.
  // The workspace used to get a second root, but that root is just the process
  // cwd (src-tauri when run via cargo) -- neither meaningful nor distinct from
  // this tree. Its path is already shown in the summary line above.
  return (
    <ul className="ft-list">
      <TreeNode
        entry={{ name: t('此电脑'), path: '', is_dir: true, ext: '' }}
        preset={drives}
        defaultOpen
        level={0}
        activePath={activePath}
        onOpen={onOpen}
      />
    </ul>
  );
}

function TreeNode({
  entry,
  level,
  activePath,
  onOpen,
  preset,
  defaultOpen,
}: {
  entry: FileEntry;
  level: number;
  activePath: string | null;
  onOpen: (path: string) => void;

  preset?: FileEntry[];
  defaultOpen?: boolean;
}) {
  const t = useT();
  const [open, setOpen] = useState(!!defaultOpen);
  const [children, setChildren] = useState<FileEntry[] | null>(null);
  const [err, setErr] = useState<string | null>(null);

  const load = useCallback(() => {
    if (preset) {
      setChildren(preset);
      return;
    }
    invoke<FileEntry[]>('list_dir', { path: entry.path })
      .then(setChildren)
      .catch((e) => setErr(String(e)));
  }, [entry.path, preset]);

  useEffect(() => {
    if (defaultOpen && children === null) load();
  }, [defaultOpen, children, load]);

  const click = useCallback(() => {
    if (!entry.is_dir) {
      onOpen(entry.path);
      return;
    }
    const next = !open;
    setOpen(next);

    if (next && children === null) load();
  }, [entry, open, children, load, onOpen]);

  const empty = children !== null && children.length === 0;

  return (
    <li>
      <button
        type="button"
        className={`ft-row${entry.is_dir ? ' is-dir' : ''}${
          entry.path === activePath ? ' on' : ''
        }`}
        style={{ paddingLeft: 6 + level * 13 }}
        onClick={click}
        title={entry.path || entry.name}
        aria-expanded={entry.is_dir ? open : undefined}
      >
        {entry.is_dir ? (
          <Icon name="chevron" size={11} className={`ft-chev${open ? ' on' : ''}`} />
        ) : (
          <span className="file-dot" data-ext={entry.ext} aria-hidden />
        )}
        <span className="ft-name">{entry.name}</span>
      </button>

      {open && (
        <ul className="ft-list">
          {err ? (
            <li className="dock-msg">{err}</li>
          ) : children === null ? (
            <li className="dock-msg">{t('读取中…')}</li>
          ) : empty ? (
            <li className="dock-msg">{t('空目录')}</li>
          ) : (
            children.map((c) => (
              <TreeNode
                key={c.path}
                entry={c}
                level={level + 1}
                activePath={activePath}
                onOpen={onOpen}
              />
            ))
          )}
        </ul>
      )}
    </li>
  );
}
