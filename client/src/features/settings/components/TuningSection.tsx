import { useSettings } from '../store/settingsStore';
import type { TuningSnapshot } from '../../../services/contracts';
import { t } from '../../../shared/i18n';

/** Tuning page: rounds / budget / context compaction / memory compaction / output cap. */
const TUNE_PRESETS = [
  {
    id: 'lean',
    name: '默认',
    tip: '出厂默认：输出上限 8192、回合上限 100、保留最少，落盘线 8,000 字符。单次最便宜、调用次数最少；代价是模型记性短，长任务可能中途迷路。',
    vals: {
      maxRounds: '100',
      maxOutputTokens: '8192',
      taskKeepOutputs: '3',
      taskStubMinBytes: '4000000',
      archiveKeepRecent: '1',
      // 落盘阈值 = **旧套**（改动前的出厂值，8K 线）：读一次更容易落盘，上下文更省。
      readSpillThreshold: '8000',
      readLinesSpillThreshold: '40000',
      spillPreviewHead: '200',
      spillPreviewTail: '120',
      spillMaxFileBytes: '8388608',
    },
  },
  {
    id: 'normal',
    name: '平衡',
    tip: '已停用（灰度）：回合 50、输出上限 65536、落盘线 16,000。',
    vals: {
      maxRounds: '50',
      maxOutputTokens: '65536',
      taskKeepOutputs: '6',
      taskStubMinBytes: '2304',
      archiveKeepRecent: '2',
      // 落盘阈值 = **新套**（2026-09-25 按 13 条落盘采样定的 16K 线）：少落盘、少回取。
      readSpillThreshold: '16000',
      readLinesSpillThreshold: '40000',
      spillPreviewHead: '200',
      spillPreviewTail: '120',
      spillMaxFileBytes: '8388608',
    },
  },
];

export function TuningSection() {
  const {
    maxRounds, compactTrigger, compactKeepRaw, compactPressureChars,
    taskKeepOutputs, archiveKeepRecent, maxOutputTokens,
    taskStubMinBytes, tuningDefaults, patch,
    saveToBackend, memoryScope,
    readSpillThreshold, readLinesSpillThreshold,
    spillPreviewHead, spillPreviewTail, spillMaxFileBytes,
  } = useSettings();

  // 参数区已收敛为只读：值由出厂默认决定，界面不再提供写入入口。
  const locked = true;
  const d = (k: keyof TuningSnapshot) => (tuningDefaults ? String(tuningDefaults[k]) : '—');

/** A preset is highlighted only while every buffer still matches it exactly, so editing */
  const buffers: Record<string, string> = {
    maxRounds, maxOutputTokens, taskKeepOutputs,
    taskStubMinBytes, archiveKeepRecent,
    readSpillThreshold, readLinesSpillThreshold,
    spillPreviewHead, spillPreviewTail, spillMaxFileBytes,
  };
  const activePreset = TUNE_PRESETS.find((p) =>
    Object.entries(p.vals).every(([k, v]) => buffers[k] === v),
  );

  const applyPreset = (p: (typeof TUNE_PRESETS)[number]) => {
    patch({ ...p.vals });
    void saveToBackend();
  };

  return (
    <>
      <section className="settings-sec">
        <div className="sec-title">{t('调优档位')}</div>
        <div className="field">
          <div className="seg">
            {TUNE_PRESETS.map((p) => (
              <button
                key={p.id}
                className={`seg-btn${activePreset?.id === p.id ? ' active' : ''}`}
                title={t(p.tip)}
                disabled
                onClick={() => applyPreset(p)}
              >
                {t(p.name)}
              </button>
            ))}
          </div>
          <p className="field-note">
            {t(activePreset?.tip ?? '档位已固定为出厂默认，不再开放切换。')}
          </p>
        </div>
      </section>

      <section className="settings-sec">
        <div className="sec-title">{t('高级参数')}</div>
        <p className="field-note">
          {t('以下参数已固定为出厂值，仅供查看，不再开放修改。鼠标悬停在参数名上可看到它是干什么的。')}
        </p>
      </section>

      {/* Long-term memory scope stays OUTSIDE the advanced switch: it is a user choice, not
            a parameter to tune. The backend owns the switch (repos::memory_scope_shared), shared
            by both the read (injection) and the write (persistence) path. */}
      <section className="settings-sec">
        <div className="sec-title">{t('长期记忆作用域')}</div>
        <div className="field">
          <div className="seg">
            <button
              className={`seg-btn${memoryScope !== 'session' ? ' active' : ''}`}
              title={t('同一个工作区（项目）下的会话互相看见长期记忆：结论、已做改动、主题、偏好、凭证。')}
              disabled
              onClick={() => {
                patch({ memoryScope: 'shared' });
                void saveToBackend();
              }}
            >
              {t('按项目共享')}
            </button>
            <button
              className={`seg-btn${memoryScope === 'session' ? ' active' : ''}`}
              title={t('只有本任务读得到自己的长期记忆：工作区域的记忆不注入，本任务的记录也不落进工作区域。')}
              disabled
              onClick={() => {
                patch({ memoryScope: 'session' });
                void saveToBackend();
              }}
            >
              {t('只属本任务')}
            </button>
          </div>
          <p className="field-note">
            {t('长期记忆按工作区（项目）存放——把两个会话放进同一个项目，它们就互相看得见。')}
            {t('选「只属本任务」后读与写都收在本会话内：工作区域的记忆不注入，本任务的记录也不落进工作区域，所以以后改回共享也不会翻出这段时间的旧账。')}
            {t('当前：')}
            {memoryScope === 'session' ? t('只属本任务') : t('按项目共享')}
          </p>
        </div>
      </section>

      <fieldset className="settings-locked" disabled={locked}>
        <section className="settings-sec">
          <div className="sec-title">{t('轮次与输出')}</div>
          <label className="field">
            <span
              className="field-label"
              data-tip={t('一个回合（你说一句 → 模型做完）最多跑几轮。到顶由系统终止并按未完成归档。调大 = 允许它多试几轮，更慢也更贵；调小 = 更早放弃。范围 1–200。')}
            >
              {t('最大轮次')}
            </span>
            <input
              className="input mono"
              type="number"
              min={1}
              max={200}
              placeholder={d('max_rounds')}
              value={maxRounds}
              onChange={(e) => patch({ maxRounds: e.target.value })}
            />
            <p className="field-note">
              {t('一个回合最多跑几轮；到顶由系统终止并按未完成归档。范围 1–200；默认 {n}。', { n: d('max_rounds') })}
            </p>
          </label>
          <label className="field">
            <span
              className="field-label"
              data-tip={t('一次请求里模型最多写多少 token。它是天花板，不是控制回复长短的旋钮：正常答复碰不到，超了会被当场切断，切到的是长代码和长参数。输出的单价是整份请求里最贵的一段（约命中部分的 200 倍），所以这一项收小是真的省钱；但也别小到切掉长参数。实际请求还会被所选模型的上限再压一次，取两者较小。范围 1024–1000000。')}
            >
              {t('单次输出 token 上限')}
            </span>
            <input
              className="input mono"
              type="number"
              min={1024}
              step={1024}
              placeholder={d('max_output_tokens')}
              value={maxOutputTokens}
              onChange={(e) => patch({ maxOutputTokens: e.target.value })}
            />
            <p className="field-note">
              {t('天花板，不是回复长短的旋钮：正常答复碰不到，超了会被切断，切到的是长代码与长参数。')}
              {t('输出单价最贵，收小是有效的省钱手段。范围 1024–1000000；默认 {n}。', { n: d('max_output_tokens') })}
              {t('实际还会被所选模型的上限再压一次，取两者较小。')}
            </p>
          </label>
        </section>

        <section className="settings-sec">
          <div className="sec-title">{t('上下文压缩')}</div>
          <p className="field-note">
            {t('管更早的几步还看不看得见。以下每一项动手时都会改写历史——历史一变，')}
            {t('服务商缓存的前缀就整段作废，紧接的那一次调用按全价重算，实测贵约 20 倍')}
            {t('（一发 0.008 元变 0.15 元）。所以判断标准不是省下多少字节，而是')}
            {t('「省下的字节」值不值「作废一次缓存」。')}
          </p>
          <label className="field">
            <span
              className="field-label"
              data-tip={t('同一个回合里，最近这几条工具输出永远保留原文，只有更早的才可能被压。给少 = 更早的输出被压、缓存作废（贵约 20 倍）；给多 = 少动历史，模型也能回看自己刚做过什么，代价是输入略长。范围 1–50。')}
            >
              {t('回合内保留最近工具输出条数')}
            </span>
            <input
              className="input mono"
              type="number"
              min={1}
              max={50}
              placeholder={d('task_keep_outputs')}
              value={taskKeepOutputs}
              onChange={(e) => patch({ taskKeepOutputs: e.target.value })}
            />
            <p className="field-note">
              {t('本回合内保留最近这几条工具输出原文，更早的压成头部摘要。给多 = 少动历史。范围 1–50；默认 {n}。', { n: d('task_keep_outputs') })}
            </p>
          </label>
          <label className="field">
            <span
              className="field-label"
              data-tip={t('早先的工具输出超过多少字节才被压成开头 300 字。压一次就作废一次缓存（贵约 20 倍），所以调小看着省 token，净账通常是亏的；调大 = 少动历史。压掉不是删除，全文仍留在暂存目录，但模型必须重跑一次工具才看得到。实测 2642 条工具输出：设 4608 压掉两成、设 8192 压掉一成二、设 800 压掉六成八（把编辑要用的行号和原文依据一起压没）。范围 128–4000000；想几乎不压就填上限。')}
            >
              {t('回合内工具输出压缩线')}
            </span>
            <input
              className="input mono"
              type="number"
              min={128}
              step={512}
              placeholder={d('task_stub_min_bytes')}
              value={taskStubMinBytes}
              onChange={(e) => patch({ taskStubMinBytes: e.target.value })}
            />
            <p className="field-note">
              {t('单位字节。早先的工具输出超过这个数才被压成开头 300 字。压一次就作废一次缓存，净账通常亏；')}
              {t('想几乎不压就填上限 4000000。范围 128–4000000；默认 {n}。', { n: d('task_stub_min_bytes') })}
            </p>
          </label>
          <label className="field">
            <span
              className="field-label"
              data-tip={t('跨回合时，最近这几个回合保留全文，更早的整段换成一行摘要。给少 = 更早的回合被换掉，那一刻缓存作废（贵约 20 倍）；给多 = 少动历史。实际保留数会取本值与窗口内回合数的较小值，设得比窗口还大也用不满。范围 2–200。')}
            >
              {t('正文保留最近回合数')}
            </span>
            <input
              className="input mono"
              type="number"
              min={2}
              max={200}
              placeholder={d('archive_keep_recent')}
              value={archiveKeepRecent}
              onChange={(e) => patch({ archiveKeepRecent: e.target.value })}
            />
            <p className="field-note">
              {t('除最近这几个回合，更早的回合正文整段换成一条摘要。给多 = 少动历史。范围 2–200；默认 {n}。', { n: d('archive_keep_recent') })}
            </p>
          </label>
        </section>

        <section className="settings-sec">
          <div className="sec-title">{t('落盘阈值（工具结果溢出）')}</div>
          <p className="field-note">
            {t('单次工具结果超过下面的线，就把全文落盘、上下文只留「头尾预览 + 取回指针」，')}
            {t('模型要看细节得再读一次。线给太低 = 总在落盘又总在回取（慢且贵）；')}
            {t('给太高 = 大块内容直接占着上下文窗口。调优档位会一并设好这几个值：')}
            {t('「默认」= 8,000 线（出厂值，上下文最省）；16,000 线那一档已停用。')}
          </p>
          <label className="field">
            <span
              className="field-label"
              data-tip={t('读文件（非 lines 模式）时，工具结果超过这个字符数就落盘。判据量的是整个信封（含 JSON 包装与转义），不是纯正文——转义后正文约膨胀一倍，所以这条线的一半才是模型实际看得到的正文量。范围 1000–1000000。')}
            >
              {t('read 全文落盘线')}
            </span>
            <input
              className="input mono"
              type="number"
              min={1000}
              step={1000}
              placeholder={d('read_spill_threshold_chars')}
              value={readSpillThreshold}
              onChange={(e) => patch({ readSpillThreshold: e.target.value })}
            />
            <p className="field-note">
              {t('单位字符（信封口径）。超过就落盘，只留头尾预览。范围 1000–1000000；默认 {n}。', { n: d('read_spill_threshold_chars') })}
            </p>
          </label>
          <label className="field">
            <span
              className="field-label"
              data-tip={t('read 显式点名要某一段（mode=lines）时用这条更高的线——模型说了要哪段就给它，不必逼它分段试探。范围 2000–2000000。')}
            >
              {t('read 精读落盘线')}
            </span>
            <input
              className="input mono"
              type="number"
              min={2000}
              step={1000}
              placeholder={d('read_lines_spill_threshold_chars')}
              value={readLinesSpillThreshold}
              onChange={(e) => patch({ readLinesSpillThreshold: e.target.value })}
            />
            <p className="field-note">
              {t('mode=lines 取段时用这条线（高于全文线）。范围 2000–2000000；默认 {n}。', { n: d('read_lines_spill_threshold_chars') })}
            </p>
          </label>
          <label className="field">
            <span
              className="field-label"
              data-tip={t('落盘后留在上下文里的头部原文字符数。太小看不出结果形态，太大会把落盘省下的体积又吃回去。上限 10000。')}
            >
              {t('预览保留头部')}
            </span>
            <input
              className="input mono"
              type="number"
              min={0}
              step={50}
              placeholder={d('spill_preview_head_chars')}
              value={spillPreviewHead}
              onChange={(e) => patch({ spillPreviewHead: e.target.value })}
            />
            <p className="field-note">
              {t('单位字符。落盘预览的头部原文量，≤10000；默认 {n}。', { n: d('spill_preview_head_chars') })}
            </p>
          </label>
          <label className="field">
            <span
              className="field-label"
              data-tip={t('落盘后留在上下文里的尾部原文字符数——尾部常是结论与汇总行，别设 0。上限 10000。')}
            >
              {t('预览保留尾部')}
            </span>
            <input
              className="input mono"
              type="number"
              min={0}
              step={50}
              placeholder={d('spill_preview_tail_chars')}
              value={spillPreviewTail}
              onChange={(e) => patch({ spillPreviewTail: e.target.value })}
            />
            <p className="field-note">
              {t('单位字符。落盘预览的尾部原文量，≤10000；默认 {n}。', { n: d('spill_preview_tail_chars') })}
            </p>
          </label>
          <label className="field">
            <span
              className="field-label"
              data-tip={t('单个落盘文件的字节硬上限，超过就截尾（防止一条全盘搜索把文件写到几十 MB）。单位字节，范围 1–512 MB。')}
            >
              {t('落盘单文件上限（字节）')}
            </span>
            <input
              className="input mono"
              type="number"
              min={1048576}
              step={1048576}
              placeholder={d('spill_max_file_bytes')}
              value={spillMaxFileBytes}
              onChange={(e) => patch({ spillMaxFileBytes: e.target.value })}
            />
            <p className="field-note">
              {t('单位字节，范围 1048576–536870912（1–512 MB）；默认 {n}。', { n: d('spill_max_file_bytes') })}
            </p>
          </label>
        </section>

        <section className="settings-sec">
          <div className="sec-title">{t('记忆压缩')}</div>
          <p className="field-note">
            {t('跨会话记忆攒到一定程度才会压。两个条件同时满足才动手——条数超过触发值，且累计字符超过压力阈值。')}
          </p>
          <label className="field">
            <span
              className="field-label"
              data-tip={t('跨会话记忆攒到这么多条才开始考虑压缩。它只是条件之一，还要同时满足下面的压力阈值。最小 2。')}
            >
              {t('触发条数')}
            </span>
            <input
              className="input mono"
              type="number"
              min={2}
              placeholder={d('compact_trigger')}
              value={compactTrigger}
              onChange={(e) => patch({ compactTrigger: e.target.value })}
            />
            <p className="field-note">
              {t('本工作区攒到这么多条记忆才开始考虑压缩。至少 2；默认 {n}。', { n: d('compact_trigger') })}
            </p>
          </label>
          <label className="field">
            <span
              className="field-label"
              data-tip={t('压缩时最近这几条原文保留不动，更早的才压成摘要。调大 = 记得更久，代价是记忆区更长。最小 1。')}
            >
              {t('保留原文条数')}
            </span>
            <input
              className="input mono"
              type="number"
              min={1}
              placeholder={d('compact_keep_raw')}
              value={compactKeepRaw}
              onChange={(e) => patch({ compactKeepRaw: e.target.value })}
            />
            <p className="field-note">
              {t('压缩时最近这几条原文保留不动，更早的压成摘要。至少 1；默认 {n}。', { n: d('compact_keep_raw') })}
            </p>
          </label>
          <label className="field">
            <span
              className="field-label"
              data-tip={t('记忆累计字符数低于它就不压缩。与上面的条数条件必须同时满足才动手。单位是字符，一个汉字算一个。最小 1000。')}
            >
              {t('压力阈值')}
            </span>
            <input
              className="input mono"
              type="number"
              min={1000}
              step={1000}
              placeholder={d('compact_pressure_chars')}
              value={compactPressureChars}
              onChange={(e) => patch({ compactPressureChars: e.target.value })}
            />
            <p className="field-note">
              {t('记忆累计字符数低于它就不压。至少 1000；默认 {n}。', { n: d('compact_pressure_chars') })}
            </p>
          </label>
        </section>
      </fieldset>

      <section className="settings-sec">
        <div className="sec-title">{t('生效方式')}</div>
        <p className="field-note">
          {t('保存后即时生效，无需重启；值写入本机 SQLite，下次启动时覆盖环境变量的初值。留空表示不改动该项。')}
        </p>
      </section>
    </>
  );
}
