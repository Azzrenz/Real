**装包**：优先项目内 `npm install`（读 `package.json`）；临时工具用 `npx <tool>`，**不加 `-g`**（全局装污染用户环境）。

**锁文件决定包管理器**：有 `package-lock.json` 用 `npm ci`；有 `pnpm-lock.yaml` 用 `pnpm i`；有 `yarn.lock` 用 `yarn`。**不要混用**——混装会让锁文件互相覆盖。

**`node_modules` 的处理**：不进版本控制；搜索与遍历要**显式排除**（否则命中量淹没真信号，且拖慢一切）。

**跑脚本先看 `package.json` 的 `scripts`**：有 `npm run build` 就别猜 `npx tsc` —— 项目声明的才是权威命令。

**版本**：看 `package.json` 的 `engines` 与 `.nvmrc`；`node -v` 先确认，别用默认版本硬跑。