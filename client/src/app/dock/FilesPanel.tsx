import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useT } from '../../shared/i18n';
import { FileTree } from './FileTree';

interface Overview {
  path: string;
  files: number;
  dirs: number;
  bytes: number;
  by_ext: [string, number][];
}

function size(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1048576) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1073741824) return `${(n / 1048576).toFixed(1)} MB`;
  return `${(n / 1073741824).toFixed(2)} GB`;
}

export function FilesPanel({
  activePath,
  onOpen,
}: {
  activePath: string | null;
  onOpen: (path: string) => void;
}) {
  const t = useT();
  const [root, setRoot] = useState<string | null>(null);
  const [ov, setOv] = useState<Overview | null>(null);

  useEffect(() => {
    let alive = true;
    invoke<string>('workspace_root')
      .then((r) => {
        if (!alive) return null;
        setRoot(r);
        return invoke<Overview>('workspace_overview', { root: r });
      })
      .then((o) => {
        if (alive && o) setOv(o);
      })
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, []);

  if (!root) return <p className="dock-msg">{t('读取中…')}</p>;

  return (
    <div className="files-panel">

      <div className="files-sum" title={root}>
        <span className="files-sum-path">{root}</span>
        {ov && (
          <span className="files-sum-stat">
            {ov.files} {t('文件')} · {size(ov.bytes)}
          </span>
        )}
      </div>
      <div className="files-tree">
        <FileTree activePath={activePath} onOpen={onOpen} />
      </div>
    </div>
  );
}
