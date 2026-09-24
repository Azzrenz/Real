
import { useEffect, useState } from 'react';
import { Icon } from '../../../shared/ui/icons';
import { useScheduleStore } from '../store/scheduleStore';
import { useSessionStore } from '../../sessions/store/sessionStore';
import { useToastStore } from '../../../shared/store/toastStore';
import { fmtDateTime } from '../../../shared/lib/format';
import { t } from '../../../shared/i18n';

export function SchedulePanel() {
  const { jobs, loading, error, load, create, remove, toggle, run } = useScheduleStore();
  const showToast = useToastStore((s) => s.show);
  const [name, setName] = useState('');
  const [prompt, setPrompt] = useState('');
  const [expr, setExpr] = useState('');
  const [adding, setAdding] = useState(false);

  useEffect(() => {
    void load();
  }, [load]);

  const add = async () => {
    const n = name.trim();
    const p = prompt.trim();
    const s = expr.trim();
    if (!n || !p || !s) {
      showToast(t('名称、提示词、调度时间三项都要填'), 'error');
      return;
    }
    setAdding(true);
    const ok = await create({ name: n, prompt: p, schedule: s });
    setAdding(false);
    if (!ok) {
      showToast(t('创建失败：{err}', { err: useScheduleStore.getState().error ?? t('未知错误') }), 'error');
      return;
    }
    setName('');
    setPrompt('');
    setExpr('');
    showToast(t('定时任务已创建'), 'ok');
  };

  const runNow = async (id: string, jobName: string) => {
    const ok = await run(id);
    if (!ok) {
      showToast(t('触发失败'), 'error');
      return;
    }
    await useSessionStore.getState().fetchSessions();
    showToast(t('已触发「{name}」，正在新建会话', { name: jobName }), 'ok');
  };

  return (
    <>
      <section className="settings-sec">
        <div className="sec-title">{t('新建定时任务')}</div>
        <label className="field">
          <span className="field-label">{t('任务名称')}</span>
          <input
            className="input"
            value={name}
            placeholder={t('如：每天整理轮日志')}
            onChange={(e) => setName(e.target.value)}
          />
        </label>
        <label className="field">
          <span className="field-label">{t('提示词（触发时当作一条消息发出）')}</span>
          <textarea
            className="input"
            value={prompt}
            rows={3}
            placeholder={t('如：阅读今日轮日志，提炼 3 条待办')}
            onChange={(e) => setPrompt(e.target.value)}
          />
        </label>
        <label className="field">
          <span className="field-label">{t('调度时间（cron 或自然语言）')}</span>
          <input
            className="input"
            value={expr}
            placeholder={t('0 9 * * * 或 每天早上 9 点')}
            onChange={(e) => setExpr(e.target.value)}
          />
        </label>
        <div className="field-actions">
          <button className="btn primary sm" onClick={() => void add()} disabled={adding}>
            <Icon name="plus" size={14} />
            {adding ? t('创建中…') : t('创建任务')}
          </button>
        </div>
        <p className="field-note">
          {t('支持 5 字段 cron（')}<code>0 9 * * *</code>{t('）或自然语言（')}
          <code>{t('每天早上 9 点')}</code>{t('），由后端解析。')}
        </p>
      </section>

      <section className="settings-sec">
        <div className="sec-title">
          {t('已有任务')}{jobs.length > 0 ? t('（{n} 个）', { n: jobs.length }) : ''}
        </div>
        {error && <p className="field-note sch-error">{error}</p>}
        {loading && jobs.length === 0 && <p className="field-note">{t('加载中…')}</p>}
        {!loading && jobs.length === 0 && !error && (
          <p className="field-note">{t('还没有定时任务——建一个，让 Real 按点干活。')}</p>
        )}
        {jobs.map((j) => (
          <div className="sch-row" key={j.id}>
            <div className="sch-main">
              <div className="sch-name" title={j.prompt}>{j.name}</div>
              <div className="sch-meta">
                <code>{j.cron_expr || j.natural_lang || '—'}</code>
                {j.next_run_at && j.enabled && t(' · 下次 {time}', { time: fmtDateTime(j.next_run_at) })}
                {j.last_run_at && t(' · 上次 {time}', { time: fmtDateTime(j.last_run_at) })}
                {j.last_status && j.last_status !== 'running' && (
                  <span className={`sch-status${j.last_status === 'error' ? ' bad' : ''}`}>
                    {' '}· {j.last_status === 'ok' ? t('上次成功') : t('上次失败')}
                  </span>
                )}
              </div>
              {j.last_result && <div className="sch-result" title={j.last_result}>{j.last_result}</div>}
            </div>
            <div className="sch-acts">
              <button
                className={`sch-pill${j.enabled ? ' on' : ''}`}
                onClick={() => void toggle(j.id, !j.enabled)}
                title={j.enabled ? t('点击停用') : t('点击启用')}
                aria-pressed={j.enabled}
              >
                {j.enabled ? t('已启用') : t('已停用')}
              </button>
              <button
                className="mem-act"
                onClick={() => void runNow(j.id, j.name)}
                title={t('立即运行（新建一个会话）')}
                aria-label={t('立即运行 {name}', { name: j.name })}
              >
                <Icon name="play" size={13} />
              </button>
              <button
                className="mem-act"
                onClick={() => void remove(j.id)}
                title={t('删除任务')}
                aria-label={t('删除 {name}', { name: j.name })}
              >
                <Icon name="x" size={13} />
              </button>
            </div>
          </div>
        ))}
      </section>
    </>
  );
}
