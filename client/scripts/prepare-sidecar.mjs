// 把后端 release 产物准备成 Tauri 的 sidecar。
//
// Tauri 的 externalBin 要求文件名带 target triple：
//   binaries/real-server-x86_64-pc-windows-msvc.exe
// 打包时 Tauri 会把它复制到安装目录、并去掉 triple 后缀，
// 于是主程序旁边就是 real-server.exe —— 壳启动时直接拉起它。
//
// 后端没编译过就先编译，让 `npm run release` 真正是一步到位。
import { execSync } from 'node:child_process';
import { copyFileSync, existsSync, mkdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(here, '..', '..');
const serverDir = join(repoRoot, 'server');

const isWin = process.platform === 'win32';
const ext = isWin ? '.exe' : '';
const srcPath = join(serverDir, 'target', 'release', `real-server${ext}`);

if (!existsSync(srcPath)) {
  console.log('[sidecar] 后端 release 产物不存在，先编译…');
  execSync('cargo build --release', { cwd: serverDir, stdio: 'inherit' });
}

if (!existsSync(srcPath)) {
  console.error(`[sidecar] 编译后仍未找到：${srcPath}`);
  process.exit(1);
}

const triple = execSync('rustc -vV', { encoding: 'utf8' })
  .split('\n')
  .find((l) => l.startsWith('host:'))
  .slice('host:'.length)
  .trim();

const outDir = join(here, '..', 'src-tauri', 'binaries');
mkdirSync(outDir, { recursive: true });
const dest = join(outDir, `real-server-${triple}${ext}`);
copyFileSync(srcPath, dest);

console.log(`[sidecar] ${srcPath}`);
console.log(`[sidecar]   -> ${dest}`);
