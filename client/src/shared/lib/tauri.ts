/** 是不是跑在 Tauri 壳里。
 *
 *  这**不是**环境探测的好奇心，而是必要判断：`@tauri-apps/api` 的 `listen` / `emit` /
 *  `invoke` 都靠壳注入的 `window.__TAURI_INTERNALS__` 工作。直接用浏览器打开 vite 的
 *  开发地址（调试界面时最省事的一条路）时它不存在，`listen()` 会在挂载阶段抛
 *  `Cannot read properties of undefined (reading 'transformCallback')` ——
 *  报错本身没意义，因为那个能力本来就不存在。
 *
 *  所以：**壳里的行为一个字不改，壳外静默跳过**。这类同步（侧栏宽度、产物推送）
 *  本来就是两个窗口之间的协调，只有一个窗口时没有对象可协调。 */
export function inTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}
