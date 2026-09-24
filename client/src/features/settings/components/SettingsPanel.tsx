import { useEffect, useRef, useState } from 'react';
import { useSettings, type RealTheme } from '../store/settingsStore';
import { Icon, type IconName } from '../../../shared/ui/icons';
import { MemoryManager } from '../../memory/components/MemoryManager';
import { SchedulePanel } from '../../schedule/components/SchedulePanel';
import { McpSection } from '../../mcp/components/McpSection';
import { TuningSection } from './TuningSection';
import { useSessionStore } from '../../sessions/store/sessionStore';
import { LANGS, t, useLang, useT } from '../../../shared/i18n';
import { settingsApi } from '../../../services/domains/settings';
import {
  actionLabel,
  listTrustRules,
  setTrustRule,
  clearSessionTrust,
  clearGlobalTrust,
  clearTrustRule,
  ACTION_DESC,
} from '../../chat/lib/confirmTrust';

interface Props {
  onClose: () => void;
}

type Tab = 'appearance' | 'model' | 'chat' | 'memory' | 'tuning' | 'schedule' | 'mcp' | 'about';

/* Labels hold the Chinese source text and are translated where they are rendered. A module-level
   t() would be evaluated once at import and then ignore every later language switch. */
const TABS: Array<{ id: Tab; label: string; icon: IconName; hint: string }> = [
  { id: 'appearance', label: '外观', icon: 'sun', hint: '颜色主题' },
  { id: 'model', label: '模型与凭据', icon: 'sliders', hint: '模型 / Key / 端点' },
  { id: 'chat', label: '对话行为', icon: 'message', hint: '思考 / 系统提示词' },
  { id: 'memory', label: '记忆与上下文', icon: 'target', hint: '长期记忆 / 画像' },
  { id: 'tuning', label: '调优', icon: 'history', hint: '轮次 / 预算 / 压缩阈值' },
  { id: 'schedule', label: '定时任务', icon: 'refresh', hint: '按点自动跑任务' },
  { id: 'mcp', label: 'MCP 服务器', icon: 'wrench', hint: '接入外部工具服务' },
  { id: 'about', label: '关于', icon: 'flame', hint: '服务状态' },
];

/** The four confirm actions a default can be set for (select_workspace / ask are per-request). */
const TRUST_ACTIONS = ['system_command', 'sensitive_write', 'delete', 'registry_write'];

const TAB_TITLE: Record<Tab, string> = {
  appearance: '外观',
  model: '模型与凭据',
  chat: '对话行为',
  memory: '记忆与上下文',
  tuning: '调优',
  schedule: '定时任务',
  mcp: 'MCP 服务器',
  about: '关于 Real',
};

import { FALLBACK_MODELS } from '../store/settingsStore';
import { prettyModel } from '../../../shared/lib/format';

export function SettingsPanel({ onClose }: Props) {
  const {
    theme, apiKey, baseUrl, model, defaultSystemPrompt,
    thinkingMode, thinkingEffort, models, modelLabels, providers,
    backend, saving, savedMsg, testing, testResult,
    setTheme, patch, saveToBackend, clearApiKey, testConnection,
    autoApproveDanger, toggleAutoApproveDanger,
  } = useSettings();
  const t = useT();
  const lang = useLang((s) => s.lang);
  const setLang = useLang((s) => s.setLang);
  const [tab, setTab] = useState<Tab>('model');
  const [confirmClear, setConfirmClear] = useState(false);
  const [trustTick, setTrustTick] = useState(0);
  const panelRef = useRef<HTMLDivElement>(null);
  const currentSessionId = useSessionStore((s) => s.currentId);
  void trustTick;
  const { sessionRules, globalRules } = currentSessionId
    ? listTrustRules(currentSessionId)
    : { sessionRules: [], globalRules: listTrustRules('').globalRules };
  const refreshRules = () => setTrustTick((t) => t + 1);
  const modelOptions = models.length ? models : FALLBACK_MODELS;
  const modelInList = modelOptions.includes(model);

  useEffect(() => {
    const panel = panelRef.current;
    if (!panel) return;
    const focusables = () => Array.from(
      panel.querySelectorAll<HTMLElement>('button, input, textarea, select, [tabindex]:not([tabindex="-1"])'),
    ).filter((el) => !el.hasAttribute('disabled'));
    const first = () => focusables()[0];
    const last = () => focusables()[focusables().length - 1];
    first()?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') { e.stopPropagation(); onClose(); return; }
      if (e.key !== 'Tab') return;
      const list = focusables();
      if (list.length === 0) return;
      const cur = document.activeElement as HTMLElement;
      const idx = list.indexOf(cur);
      if (e.shiftKey && (idx <= 0 || !list.includes(cur))) {
        e.preventDefault(); last()?.focus();
      } else if (!e.shiftKey && (idx === list.length - 1 || !list.includes(cur))) {
        e.preventDefault(); first()?.focus();
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);

  const modeLabel = backend
    ? backend.llm_mode === 'real'
      ? t('已连接（real）{extra}', {
          extra: backend.has_api_key ? ` · Key ${backend.api_key_masked}` : '',
        })
      : t('Mock 演示模式（未配置 API Key）')
    : t('后端未连接（请先启动服务）');

  const savable = tab === 'model' || tab === 'chat' || tab === 'memory' || tab === 'tuning';

  return (
    <div className="settings-overlay">
      <div
        ref={panelRef}
        className="settings-panel settings-panel--wide"
        role="dialog"
        aria-modal="true"
        aria-label={t('设置')}
      >
        <aside className="settings-nav">
          <div className="settings-nav-title">{t('设置')}</div>
          {TABS.map((item) => (
            <button
              key={item.id}
              className={`settings-nav-item${tab === item.id ? ' active' : ''}`}
              onClick={() => setTab(item.id)}
              title={t(item.hint)}
            >
              <Icon name={item.icon} size={16} />
              <span className="settings-nav-label">{t(item.label)}</span>
            </button>
          ))}
          <div className="settings-nav-foot">{t('Real · 设置即时生效')}</div>
        </aside>

        <div className="settings-main">
          <div className="settings-head">
            <span className="sh-title">{t(TAB_TITLE[tab])}</span>
            <button className="icon-btn" onClick={onClose} title={t('关闭')} aria-label={t('关闭设置')}>
              <Icon name="x" size={15} />
            </button>
          </div>

          <div className="settings-body">
            {tab === 'appearance' && (
              <>
                <section className="settings-sec">
                  <div className="sec-title">{t('颜色主题')}</div>
                  <div className="theme-options">
                    {(['dark', 'light'] as RealTheme[]).map((th) => (
                      <button
                        key={th}
                        className={`theme-option${theme === th ? ' active' : ''}`}
                        onClick={() => setTheme(th)}
                      >
                        <Icon name={th === 'dark' ? 'moon' : 'sun'} size={15} />
                        {th === 'dark' ? t('深色') : t('浅色')}
                      </button>
                    ))}
                  </div>
                </section>
                <section className="settings-sec">
                  <div className="sec-title">{t('界面语言')}</div>
                  <div className="theme-options">
                    {LANGS.map((l) => (
                      <button
                        key={l.id}
                        className={`theme-option${lang === l.id ? ' active' : ''}`}
                        onClick={() => setLang(l.id)}
                      >
                        <Icon name="globe" size={15} />
                        {l.label}
                      </button>
                    ))}
                  </div>
                </section>
              </>
            )}

            {tab === 'model' && (
              <>
                <section className="settings-sec">
                  <div className="sec-title">{t('模型')}</div>
                  <label className="field">
                    <span className="field-label">{t('模型名')}</span>
                    {modelInList ? (
                      <select
                        className="input mono"
                        value={model}
                        onChange={(e) => {
                          const v = e.target.value;
                          if (v === '__custom__') return;
                          // Endpoint from the provider archive - no vendor names hardcoded.
                          const u = providers.find((p) => p.models.some((m) => m.id === v))?.base_url;
                          patch({ model: v, ...(u ? { baseUrl: u } : {}) });
                        }}
                      >
                        {modelOptions.map((m) => <option key={m} value={m}>{prettyModel(m, modelLabels)}</option>)}
                        <option value="__custom__">{t('自定义…')}</option>
                      </select>
                    ) : (
                      <>
                        <input className="input mono" value={model} onChange={(e) => patch({ model: e.target.value })} />
                        <p className="field-note">
                          {t('当前模型不在列表（历史自定义）；输入已注册模型名即可回到下拉选择')}
                        </p>
                      </>
                    )}
                  </label>
                  <p className="field-note">
                    {t('输入框左下角的模型芯片可一键互切 DeepSeek / GLM（后端按档案联动换端点与 Key）。')}
                  </p>
                </section>

                <section className="settings-sec">
                  <div className="sec-title">{t('凭据与端点')}</div>
                  <label className="field">
                    <span className="field-label">
                      {t('API Key（{p}）', { p: model.startsWith('glm') ? t('智谱 GLM') : 'DeepSeek' })}
                    </span>
                    <input
                      className="input"
                      type="password"
                      value={apiKey}
                      placeholder={backend?.has_api_key ? t('已配置（留空则不改动）') : (model.startsWith('glm') ? t('粘贴智谱 Key（open.bigmodel.cn 获取）') : 'sk-…')}
                      onChange={(e) => patch({ apiKey: e.target.value })}
                      autoComplete="off"
                    />
                  </label>
                  <label className="field">
                    <span className="field-label">API Base URL</span>
                    <input className="input mono" value={baseUrl} onChange={(e) => patch({ baseUrl: e.target.value })} />
                  </label>
                  <div className="field-actions">
                    <button className="btn ghost sm" onClick={() => void testConnection()} disabled={testing}>
                      {testing ? t('测试中…') : t('测试连接')}
                    </button>
                    {backend?.has_api_key && (
                      <button
                        className="btn ghost sm danger"
                        onClick={() => {
                          if (confirmClear) {
                            void clearApiKey();
                            setConfirmClear(false);
                          } else {
                            setConfirmClear(true);
                            window.setTimeout(() => setConfirmClear(false), 3000);
                          }
                        }}
                      >
                        {confirmClear ? t('再点一次确认清除') : t('清除 API Key')}
                      </button>
                    )}
                  </div>
                  {testResult && (
                    <div className={`test-result ${testResult.ok ? 'ok' : 'err'}`}>
                      <Icon name={testResult.ok ? 'check' : 'x'} size={13} style={{ verticalAlign: '-2px', marginRight: 4 }} />
                      {testResult.ok
                        ? t('{note} · HTTP {status} · {ms}ms', {
                            note: testResult.note ?? t('可达'),
                            status: testResult.status ?? '—',
                            ms: testResult.latency_ms ?? '—',
                          })
                        : t('连接失败：{err}', { err: testResult.error ?? t('未知错误') })}
                      <span className="test-url">{testResult.url}</span>
                    </div>
                  )}
                  <p className="field-note">
                    {t('当前状态：{mode}。填写 API Key 保存后自动启用真实调用（real）；清空 Key 自动回 Mock 演示模式。', { mode: modeLabel })}
                  </p>
                </section>
              </>
            )}

            {tab === 'chat' && (
              <>
                <section className="settings-sec">
                  <div className="sec-title">{t('思考模式')}</div>
                  <div className="field">
                    <span className="field-label">{t('模式')}</span>
                    <div className="seg">
                      {(['auto', 'on', 'off'] as const).map((m) => (
                        <button
                          key={m}
                          className={`seg-btn${thinkingMode === m ? ' active' : ''}`}
                          onClick={() => patch({ thinkingMode: m })}
                          title={m === 'auto' ? t('按任务复杂度自动：简单问答关思考（快而省），复杂任务开思考') : m === 'on' ? t('始终开启思考') : t('始终关闭思考（最快最省）')}
                        >
                          {m === 'auto' ? t('自适应') : m === 'on' ? t('始终开启') : t('关闭思考')}
                        </button>
                      ))}
                    </div>
                    <p className="field-note">
                      {thinkingMode === 'auto'
                        ? t('简单问答（短输入/无代码意图）自动关闭思考省 token；修复/重构类任务自动开启。')
                        : thinkingMode === 'on'
                          ? t('所有规划都深度思考（更准但更慢更贵）。')
                          : t('所有规划都不思考（最快最省，复杂任务质量可能下降）。')}
                      <br />
                      {t('思考内容按输出单价计费（官方定价页），是输出成本的主要来源。DeepSeek 关闭即真关思考（reasoning.effort=none）；GLM-5.3-flash 官方不支持关闭思考（thinking.type 仅 enabled），关闭模式对 GLM 取最低档 low。')}
                    </p>
                  </div>
                  <div className="field">
                    <span className="field-label">{t('思考强度（开启时）')}</span>
                    <div className="seg">
                      {(['low', 'high', 'max'] as const).map((e) => (
                        <button
                          key={e}
                          className={`seg-btn${thinkingEffort === e ? ' active' : ''}`}
                          onClick={() => patch({ thinkingEffort: e })}
                        >
                          {e === 'low' ? t('低') : e === 'high' ? t('高') : t('最大')}
                        </button>
                      ))}
                    </div>
                    <p className="field-note">
                      {t('强度越高思考越深入、响应越慢越贵。DeepSeek 官方档位 low/high/max（medium 会被映射为 high）；GLM 官方推荐 max。日常建议「低」，复杂重构临时「最大」。')}
                    </p>
                  </div>
                </section>

                <section className="settings-sec">
                  <div className="sec-title">{t('操作确认规则')}</div>
                  <p className="field-note">
                    {t('这几类操作被 Agent 触发时会先弹窗。这里给每类定一个默认做法（对所有会话生效）；某个会话里勾过「不再询问」的会记在该会话上，见下方。')}
                  </p>

                  <div className="field">
                    <span className="field-label">{t('危险操作自动放行')}</span>
                    <div className="seg">
                      {([false, true] as const).map((on) => (
                        <button
                          key={String(on)}
                          type="button"
                          className={`seg-btn${autoApproveDanger === on ? ' active' : ''}`}
                          onClick={() => {
                            if (autoApproveDanger !== on) toggleAutoApproveDanger();
                          }}
                        >
                          {on ? t('自动放行') : t('每次询问')}
                        </button>
                      ))}
                    </div>
                    <p className="field-note">
                      {t('总开关，与输入框底部那个小按钮是同一个状态。开启后删除 / 敏感写入不再弹窗，直接批准；单类规则里设成「自动拒绝」的优先级更高，仍会拦住。')}
                    </p>
                  </div>

                  <div className="trust-rows">
                    {TRUST_ACTIONS.map((a) => {
                      const cur = globalRules.find((r) => r.action === a)?.decision ?? 'ask';
                      const set = (d: 'ask' | 'allow' | 'deny') => {
                        if (d === 'ask') clearTrustRule('', a, 'global');
                        else setTrustRule('', a, d, 'global');
                        refreshRules();
                      };
                      return (
                        <div key={a} className="trust-row">
                          <span className="trust-row-k">
                            <b>{t(actionLabel(a))}</b>
                            <em>{t(ACTION_DESC[a] ?? '')}</em>
                          </span>
                          <div className="seg">
                            {(['ask', 'allow', 'deny'] as const).map((d) => (
                              <button
                                key={d}
                                type="button"
                                className={`seg-btn${cur === d ? ' active' : ''}`}
                                onClick={() => set(d)}
                              >
                                {d === 'ask' ? t('每次询问') : d === 'allow' ? t('自动允许') : t('自动拒绝')}
                              </button>
                            ))}
                          </div>
                        </div>
                      );
                    })}
                  </div>

                  {sessionRules.length > 0 && (
                    <div className="field">
                      <span className="field-label">
                        {t('本会话（{id}）', { id: currentSessionId ? currentSessionId.slice(0, 8) : '—' })}
                      </span>
                      <div className="trust-rule-chips">
                        {sessionRules.map((r) => (
                          <span key={`s-${r.action}`} className={`trust-chip decision-${r.decision}`}>
                            {t(actionLabel(r.action))}
                            <b>{t(r.decision === 'allow' ? '自动允许' : '自动拒绝')}</b>
                            <button
                              type="button"
                              className="trust-chip-x"
                              onClick={() => {
                                if (currentSessionId) clearTrustRule(currentSessionId, r.action, 'session');
                                refreshRules();
                              }}
                              aria-label={t('清除{action}的本会话规则', { action: t(actionLabel(r.action)) })}
                            >
                              ×
                            </button>
                          </span>
                        ))}
                        <button
                          type="button"
                          className="trust-clear"
                          onClick={() => {
                            if (currentSessionId) clearSessionTrust(currentSessionId);
                            refreshRules();
                          }}
                        >
                          {t('清除本会话全部')}
                        </button>
                      </div>
                    </div>
                  )}

                  {globalRules.length > 0 && (
                    <button
                      type="button"
                      className="trust-clear trust-reset"
                      onClick={() => {
                        clearGlobalTrust();
                        refreshRules();
                      }}
                    >
                      {t('全部恢复为「每次询问」')}
                    </button>
                  )}
                </section>

                <section className="settings-sec">
                  <div className="sec-title">{t('默认系统提示词')}</div>
                  <textarea
                    className="input area"
                    value={defaultSystemPrompt}
                    onChange={(e) => patch({ defaultSystemPrompt: e.target.value })}
                    placeholder={t('留空即使用内置规则（server/prompts/workflow/system.md）')}
                    rows={3}
                  />
                  <p className="field-note">{t('新建会话时自动带入；留空沿用内置执行伙伴规则。')}</p>
                </section>
              </>
            )}

            {tab === 'memory' && (
              <>
                <section className="settings-sec">
                  <div className="sec-title">{t('长期记忆（主题 / 偏好 / 画像）')}</div>
                  <p className="field-note">
                    {t('每轮对话自动提炼主题（重要度加权，架构/方向类自动永久）、记录偏好与凭证（token 给过一次不再问）。')}
                    {t('以下可直接管理——钉永久 / 删除 / 手动补充。')}
                  </p>
                </section>
                <MemoryManager />
              </>
            )}

            {tab === 'tuning' && <TuningSection />}

            {tab === 'schedule' && <SchedulePanel />}
            {tab === 'mcp' && <McpSection />}

            {tab === 'about' && (
              <>
                <section className="settings-sec">
                  <div className="sec-title">{t('服务状态')}</div>
                  <div className="about-rows">
                    <div className="about-row"><span className="about-k">{t('后端地址')}</span><span className="about-v mono">{backend ? `${backend.server.host}:${backend.server.port}` : t('未连接')}</span></div>
                    <div className="about-row"><span className="about-k">{t('运行模式')}</span><span className="about-v">{backend ? (backend.llm_mode === 'real' ? t('real（真实调用）') : t('mock（演示）')) : '—'}</span></div>
                    <div className="about-row"><span className="about-k">{t('当前模型')}</span><span className="about-v mono">{backend ? prettyModel(backend.model, modelLabels) : '—'}</span></div>
                    <div className="about-row"><span className="about-k">{t('可选模型')}</span><span className="about-v">{t('{n} 个', { n: backend?.models?.length ?? 0 })}</span></div>
                    <div className="about-row"><span className="about-k">{t('当前生效 Key')}</span><span className="about-v">{backend?.has_api_key ? t('已配置（{masked}）', { masked: backend.api_key_masked ?? '' }) : t('未配置')}</span></div>
                    <div className="about-row"><span className="about-k">{t('GLM Key')}</span><span className="about-v">{backend?.has_glm_key ? t('已配置') : t('未配置')}</span></div>
                  </div>
                </section>
                <DataDirSection />
                <section className="settings-sec">
                  <div className="sec-title">{t('说明')}</div>
                  <p className="field-note">
                    {t('所有模型与调优设置保存后即时生效（无需重启），持久化在本机 SQLite。API Key 只存本机后端，界面永远只显示掩码。')}
                  </p>
                </section>
              </>
            )}
          </div>

          {savable && (
            <div className="settings-actions">
              <button
                className="btn primary"
                onClick={() => void saveToBackend()}
                disabled={saving}
              >
                {saving ? t('保存中…') : t('保存设置')}
              </button>
              {/* Success stays silent on purpose: the panel no longer closes and the fields
                  themselves are the receipt. Only a failure needs words. */}
              {(savedMsg?.startsWith(t('保存失败')) || savedMsg?.startsWith(t('清除失败'))) && (
                <span className="save-msg err">{savedMsg}</span>
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}


function DataDirSection() {
  const [dir, setDir] = useState('');
  const [effective, setEffective] = useState('');
  const [msg, setMsg] = useState('');
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    void settingsApi.get().then((s) => {
      setDir(s.data_dir ?? '');
      setEffective(s.data_root_effective ?? '');
    }).catch(() => setEffective(t('（后端未连接）')));
  }, []);

  const save = async (clear = false) => {
    setSaving(true);
    setMsg('');
    try {
      const s = await settingsApi.put({ data_dir: clear ? '' : dir.trim() });
      setDir(s.data_dir ?? '');
      setEffective(s.data_root_effective ?? '');
      setMsg(t('已保存——重启 Real 后生效，数据将迁移到新目录'));
    } catch (e) {
      setMsg(t('保存失败：{err}', { err: e instanceof Error ? e.message : String(e) }));
    } finally {
      setSaving(false);
    }
  };

  return (
    <section className="settings-sec">
      <div className="sec-title">{t('数据目录')}</div>
      <label className="field">
        <span className="field-label">{t('日志 / spill / 数据库存放根')}</span>
        <input
          className="input mono"
          value={dir}
          placeholder={t('留空 = 默认（{dir}）', { dir: effective || '%APPDATA%\\real-agent' })}
          onChange={(e) => setDir(e.target.value)}
        />
        <p className="field-note">
          {t('安装目录只放程序本体；运行产生的数据统一写数据根。留空用默认位置；改后需重启——启动时自动把旧数据迁入新目录。环境变量 REAL_DATA_DIR 优先级更高。')}
        </p>
      </label>
      <div className="settings-actions" style={{ justifyContent: 'flex-start' }}>
        <button className="btn primary" disabled={saving} onClick={() => void save(false)}>
          {saving ? t('保存中…') : t('保存数据目录')}
        </button>
        {dir && (
          <button className="btn" disabled={saving} onClick={() => void save(true)}>
            {t('恢复默认')}
          </button>
        )}
        {msg && <span className={`save-msg ${msg.startsWith(t('保存失败')) ? 'err' : ''}`}>{msg}</span>}
      </div>
    </section>
  );
}
