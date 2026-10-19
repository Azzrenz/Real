/** 手机端配对页：把「电脑上生成的 6 位码」换成设备 token。
 *
 *  这是手机第一次打开 Real 时唯一能看到的界面（未配对 ⇒ 网关对所有 `/api/*` 统一回 404，
 *  业务界面拿不到任何数据）。所以它必须：① 能改服务器地址（否则填错地址就死循环），
 *  ② 只做一件事 —— 输码 → 连接，③ 失败时说清「怎么办」，而不是甩一个状态码。
 */
import { useEffect, useRef, useState } from 'react';
import { http, normalizeBase, storedBase } from '../../../services/http';
import { useAuthStore } from '../store/authStore';
import { Icon } from '../../../shared/ui/icons';
import { useT } from '../../../shared/i18n';
import './pair.css';

export function PairPage() {
  const t = useT();
  const pair = useAuthStore((s) => s.pair);
  const pairing = useAuthStore((s) => s.pairing);
  const error = useAuthStore((s) => s.error);

  // 地址默认回填：存过的地址 → 当前生效地址（?api= 或默认值）。
  const [server, setServer] = useState(() => normalizeBase(storedBase() || http.baseUrl));
  const [code, setCode] = useState('');
  // 第一次配对（没存过地址）时直接把地址框摊开：那时候默认值 127.0.0.1 一定是错的，
  // 让用户先点开再填是多一步无用功。
  const [showServer, setShowServer] = useState(() => !storedBase());
  const codeRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    codeRef.current?.focus();
  }, []);

  const submit = () => {
    if (pairing) return;
    void pair(server, code);
  };

  const ready = code.trim().length === 6 && !pairing;

  return (
    <div className="pair-screen">
      <div className="pair-card">
        <div className="pair-brand">
          <Icon name="flame" size={40} className="pair-flame" />
          <span className="pair-name">Real</span>
        </div>

        <h1 className="pair-title">{t('连接你的电脑')}</h1>
        <p className="pair-sub">
          {t('在电脑上打开 Real，进入「设置 → 设备」生成一个 6 位配对码，然后在这里输入。')}
        </p>

        <label className="pair-label" htmlFor="pair-code">
          {t('6 位配对码')}
        </label>
        <input
          id="pair-code"
          ref={codeRef}
          className="pair-code"
          value={code}
          onChange={(e) => setCode(e.target.value.replace(/\D/g, '').slice(0, 6))}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && ready) submit();
          }}
          inputMode="numeric"
          autoComplete="one-time-code"
          pattern="[0-9]*"
          maxLength={6}
          placeholder="······"
          aria-label={t('6 位配对码')}
          disabled={pairing}
        />

        {!showServer ? (
          <button
            type="button"
            className="pair-advanced-toggle"
            onClick={() => setShowServer(true)}
            disabled={pairing}
          >
            {t('服务器地址：{url}（点此修改）', { url: normalizeBase(server) || t('未设置') })}
          </button>
        ) : (
          <>
            <label className="pair-label" htmlFor="pair-server">
              {t('电脑端服务器地址')}
            </label>
            <input
              id="pair-server"
              className="pair-server"
              value={server}
              onChange={(e) => setServer(e.target.value)}
              inputMode="url"
              autoCapitalize="none"
              autoCorrect="off"
              spellCheck={false}
              placeholder="http://100.64.0.1:8943"
              aria-label={t('电脑端服务器地址')}
              disabled={pairing}
            />
            <p className="pair-hint">
              {t('手机和电脑要在同一个网络里（常用 Tailscale）。地址就是电脑上 Real 服务的地址，含端口。')}
            </p>
          </>
        )}

        {error && (
          <div className="pair-error" role="alert">
            <Icon name="x" size={14} />
            <span>{error}</span>
          </div>
        )}

        <button type="button" className="pair-submit" onClick={submit} disabled={!ready}>
          {pairing ? (
            <>
              <Icon name="spinner" size={15} className="icon-spin" />
              {t('连接中…')}
            </>
          ) : (
            t('连接')
          )}
        </button>

        <p className="pair-foot">
          {t('配对码 5 分钟内有效、只能成功一次；如果提示不对，请到电脑上重新生成一个。')}
        </p>
      </div>
    </div>
  );
}
