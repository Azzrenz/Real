/** 视口断点与「当前是不是手机版」判定。
 *
 *  为什么用 `matchMedia` 而不是 UA 嗅探：UA 字符串既能伪装又会过时，而真正决定布局
 *  该怎么排的是**视口有多宽**。手机浏览器、PWA、窄窗桌面浏览器三者需要的都是同一种
 *  窄屏布局 —— 判据统一到宽度上，行为才和 CSS 的 `@media (max-width: 768px)` 一致。
 *
 *  为什么 768 这个数**不会**误伤桌面壳：Tauri 窗口配了 `minWidth: 900`
 *  （`src-tauri/tauri.conf.json`），壳里视口永远 ≥900px ⇒ 这条查询在桌面上恒为 false，
 *  移动分支一次都不会进。桌面零变化由这个下限保证，不是靠约定。
 */
import { useEffect, useState } from 'react';

/** 手机版断点：≤ 此宽度走窄屏布局。改这里要同步改 `styles/mobile.css` 的同名查询。 */
export const MOBILE_MAX_WIDTH = 768;
export const MOBILE_QUERY = `(max-width: ${MOBILE_MAX_WIDTH}px)`;

function matches(): boolean {
  if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return false;
  return window.matchMedia(MOBILE_QUERY).matches;
}

/** 订阅式地返回当前是否处于手机版布局；窗口跨过断点会重新渲染。
 *  首帧同步读取真实值，避免「先桌面后手机」的闪一下。 */
export function useIsMobile(): boolean {
  const [mobile, setMobile] = useState(matches);
  useEffect(() => {
    if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return;
    const mq = window.matchMedia(MOBILE_QUERY);
    const onChange = () => setMobile(mq.matches);
    onChange();
    mq.addEventListener('change', onChange);
    return () => mq.removeEventListener('change', onChange);
  }, []);
  return mobile;
}

/** 把「当前真正可见的高度」写进 `--app-h`，供 mobile.css 使用。
 *
 *  为什么不能只靠 `100dvh`：iOS Safari 弹软键盘时**不改布局视口**，键盘直接盖在页面上
 *  ⇒ 底部输入框被埋住；Android Chrome 会改。只有 `window.visualViewport.height` 两边都
 *  跟得上键盘的收放。所以窄屏下高度以这个变量为准，`100dvh` 只作兜底。
 *
 *  只在窄屏（= 移动分支）写这个变量：桌面根本不读它，连设置动作都省掉，
 *  这样「桌面零变化」连一个自定义属性都不会多出来。 */
export function useAppHeight(): void {
  useEffect(() => {
    if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return;
    const mq = window.matchMedia(MOBILE_QUERY);
    const root = document.documentElement;
    const vv = window.visualViewport;
    const apply = () => {
      if (!mq.matches) {
        root.style.removeProperty('--app-h');
        return;
      }
      const h = vv?.height ?? window.innerHeight;
      root.style.setProperty('--app-h', `${Math.round(h)}px`);
    };
    apply();
    window.addEventListener('resize', apply);
    mq.addEventListener('change', apply);
    vv?.addEventListener('resize', apply);
    vv?.addEventListener('scroll', apply);
    return () => {
      window.removeEventListener('resize', apply);
      mq.removeEventListener('change', apply);
      vv?.removeEventListener('resize', apply);
      vv?.removeEventListener('scroll', apply);
      root.style.removeProperty('--app-h');
    };
  }, []);
}
