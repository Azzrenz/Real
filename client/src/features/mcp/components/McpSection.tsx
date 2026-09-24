import { useEffect, useState } from 'react';
import { Icon } from '../../../shared/ui/icons';
import { mcpApi } from '../../../services/domains/mcp';
import type { McpServerConfig, McpServerStatus, McpTransport } from '../../../services/contracts';
import './mcp.css';
import { t } from '../../../shared/i18n';

interface Row {
  key: number;
  name: string;
  transport: McpTransport;
  command: string;
  argsText: string;
  url: string;
  headersText: string;
}

let rowSeq = 1;

function newRow(): Row {
  return { key: rowSeq++, name: '', transport: 'stdio', command: '', argsText: '', url: '', headersText: '' };
}

function rowFromConfig(c: McpServerConfig): Row {
  return {
    key: rowSeq++,
    name: c.name,
    transport: c.transport === 'http' ? 'http' : 'stdio',
    command: c.command ?? '',
    argsText: (c.args ?? []).join(' '),
    url: c.url ?? '',
    headersText: c.headers && Object.keys(c.headers).length ? JSON.stringify(c.headers, null, 2) : '',
  };
}

function configFromRow(r: Row): McpServerConfig {
  const isStdio = r.transport === 'stdio';
  return {
    name: r.name.trim(),
    transport: r.transport,
    command: isStdio ? r.command.trim() : null,
    args: isStdio ? r.argsText.split(/\s+/).filter(Boolean) : [],
    url: isStdio ? null : r.url.trim(),
    headers: !isStdio && r.headersText.trim() ? (JSON.parse(r.headersText) as Record<string, string>) : {},
  };
}

function validate(rows: Row[]): string {
  const seen = new Set<string>();
  for (const r of rows) {
    const name = r.name.trim();
    if (!name) return t('有服务器还没填名称');
    if (seen.has(name)) return t('名称重复：{name}', { name });
    seen.add(name);
    if (r.transport === 'stdio' && !r.command.trim()) return t('{name}：stdio 传输需要填命令', { name });
    if (r.transport === 'http' && !r.url.trim()) return t('{name}：http 传输需要填地址', { name });
    if (r.transport === 'http' && r.headersText.trim()) {
      try {
        const h = JSON.parse(r.headersText) as unknown;
        if (!h || typeof h !== 'object' || Array.isArray(h)) return t('{name}：请求头必须是 JSON 对象', { name });
      } catch {
        return t('{name}：请求头不是合法 JSON', { name });
      }
    }
  }
  return '';
}

export function McpSection() {
  const [rows, setRows] = useState<Row[]>([]);
  const [statuses, setStatuses] = useState<McpServerStatus[]>([]);
  const [busy, setBusy] = useState<'' | 'load' | 'save' | 'apply'>('load');
  const [msg, setMsg] = useState('');
  const [err, setErr] = useState('');

  const load = async () => {
    setBusy('load');
    setErr('');
    try {
      const r = await mcpApi.list();
      setStatuses(r.servers ?? []);
      setRows((r.saved ?? []).map(rowFromConfig));
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy('');
    }
  };

  useEffect(() => {
    void load();
  }, []);

  const patch = (key: number, part: Partial<Row>) => {
    setRows((cur) => cur.map((r) => (r.key === key ? { ...r, ...part } : r)));
  };

  const save = async () => {
    const bad = validate(rows);
    if (bad) {
      setErr(bad);
      setMsg('');
      return;
    }
    setBusy('save');
    setErr('');
    try {
      const configs = rows.map(configFromRow);
      const r = await mcpApi.save(configs);
      if (!r.ok) {
        setErr(r.error ?? t('保存失败'));
      } else {
        setMsg(t('已保存 {n} 个服务器——点「应用到运行时」生效', { n: r.count ?? configs.length }));
      }
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy('');
    }
  };

  const apply = async () => {
    setBusy('apply');
    setErr('');
    try {
      const r = await mcpApi.apply();
      setStatuses(r.servers ?? []);
      const parts = [t('新增/更新 {n}', { n: r.applied.length })];
      if (r.removed.length) parts.push(t('摘除 {n}', { n: r.removed.length }));
      if (r.failed.length) parts.push(t('失败 {n}', { n: r.failed.length }));
      if (r.failed.length) setErr(r.failed.map((f) => `${f.name}：${f.error}`).join('；'));
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy('');
    }
  };

  const connected = statuses.filter((s) => s.connected).length;

  return (
    <>
      <section className="settings-sec">
        <div className="sec-title">{t('MCP 服务器')}</div>
        <p className="field-note">
          {t('接入外部工具服务（MCP 协议：本地命令或 HTTP 地址）。保存只写入配置，点「应用到运行时」才真正连上——连接失败的原因会显示在对应服务器下面。')}
        </p>
        <div className="mcp-summary">
          <span className="mcp-summary-item">{t('{n} 个配置', { n: rows.length })}</span>
          <span className="mcp-summary-item">{t('已连接 {n}', { n: connected })}</span>
          {statuses.length - connected > 0 && (
            <span className="mcp-summary-item is-bad">{t('异常 {n}', { n: statuses.length - connected })}</span>
          )}
          <span className="spacer" />
          <button className="btn ghost" onClick={() => void load()} disabled={busy !== ''}>
            <Icon name="refresh" size={15} />
            {t('刷新状态')}
          </button>
        </div>
      </section>

      <section className="settings-sec">
        <div className="sec-title">{t('配置')}</div>
        {rows.length === 0 && <p className="field-note">{t('还没有配置。点下面的「添加服务器」开始。')}</p>}

        <div className="mcp-list">
          {rows.map((r) => {
            const st = statuses.find((s) => s.name === r.name.trim());
            return (
              <div className="mcp-row" key={r.key}>
                <div className="mcp-row-head">
                  <input
                    className="input mcp-name"
                    placeholder={t('名称，如 filesystem')}
                    value={r.name}
                    onChange={(e) => patch(r.key, { name: e.target.value })}
                  />
                  <select
                    className="input mcp-transport"
                    value={r.transport}
                    onChange={(e) => patch(r.key, { transport: e.target.value as McpTransport })}
                  >
                    <option value="stdio">{t('本地命令')}</option>
                    <option value="http">{t('HTTP 地址')}</option>
                  </select>
                  <span className="spacer" />
                  <button
                    className="btn ghost sm"
                    title={t('移除这条配置')}
                    onClick={() => setRows((cur) => cur.filter((x) => x.key !== r.key))}
                  >
                    <Icon name="trash" size={14} /> {t('移除')}
                  </button>
                </div>

                {r.transport === 'stdio' ? (
                  <div className="mcp-row-grid">
                    <label className="field">
                      <span className="field-label">{t('命令')}</span>
                      <input
                        className="input mono"
                        placeholder="npx"
                        value={r.command}
                        onChange={(e) => patch(r.key, { command: e.target.value })}
                      />
                    </label>
                    <label className="field">
                      <span className="field-label">{t('参数（空格分隔）')}</span>
                      <input
                        className="input mono"
                        placeholder="-y @modelcontextprotocol/server-filesystem D:////work"
                        value={r.argsText}
                        onChange={(e) => patch(r.key, { argsText: e.target.value })}
                      />
                    </label>
                  </div>
                ) : (
                  <div className="mcp-row-grid">
                    <label className="field">
                      <span className="field-label">{t('地址')}</span>
                      <input
                        className="input mono"
                        placeholder="https://example.com/mcp"
                        value={r.url}
                        onChange={(e) => patch(r.key, { url: e.target.value })}
                      />
                    </label>
                    <label className="field">
                      <span className="field-label">{t('请求头（JSON，可空）')}</span>
                      <input
                        className="input mono"
                        placeholder={'{"Authorization":"Bearer …"}'}
                        value={r.headersText}
                        onChange={(e) => patch(r.key, { headersText: e.target.value })}
                      />
                    </label>
                  </div>
                )}

                <div className="mcp-row-status">
                  {st ? (
                    st.connected ? (
                      <span className="mcp-badge is-ok">{t('已连接 · {n} 个工具', { n: st.tool_count })}</span>
                    ) : (
                      <span className="mcp-badge is-bad">{st.last_error || t('连接失败')}</span>
                    )
                  ) : (
                    <span className="mcp-badge">{t('未连接到运行时')}</span>
                  )}
                </div>
              </div>
            );
          })}
        </div>

        <div className="mcp-actions">
          <button className="btn ghost" onClick={() => setRows((cur) => [...cur, newRow()])}>
            <Icon name="plus" size={15} />
            {t('添加服务器')}
          </button>
          <span className="spacer" />
          <button className="btn" onClick={() => void save()} disabled={busy !== ''}>
            {busy === 'save' ? t('保存中…') : t('保存配置')}
          </button>
          <button className="btn" onClick={() => void apply()} disabled={busy !== ''}>
            {busy === 'apply' ? t('应用中…') : t('应用到运行时')}
          </button>
        </div>

        {msg && <p className="field-note mcp-msg">{msg}</p>}
        {err && <p className="field-note mcp-err">{err}</p>}
      </section>
    </>
  );
}
