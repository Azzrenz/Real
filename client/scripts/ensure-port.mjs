// 启动前端口保障（坑位 J50）：npm run dev 前自动清理 8618 残留 vite 进程
// 背景：vite strictPort:true + tauri devUrl 固定 8618 → 端口被占直接报错退出，
// 且 tauri 窗口加载 fixed URL 不能换端口。残留进程（窗口关了 node 还在）是唯一
// 常态占用源 → 检测到 node 进程占 8618 就自动终止，非 node 进程给出明确提示。
// 2026-09-08：端口 8608→8618，此前守门脚本没跟上改端口，strictPort 必炸（本次故障根因）。
import { execSync, spawnSync } from 'node:child_process';
import net from 'node:net';

const PORT = 8618;

function isPortFree(port) {
  return new Promise((resolve) => {
    const srv = net.createServer();
    srv.once('error', () => resolve(false));
    srv.once('listening', () => srv.close(() => resolve(true)));
    srv.listen(port, '127.0.0.1');
  });
}

function getListeningPid(port) {
  // 用 PowerShell Get-NetTCPConnection 取 PID（locale 无关，比 netstat findstr 稳）
  try {
    const out = execSync(
      `powershell -NoProfile -Command "(Get-NetTCPConnection -LocalPort ${port} -State Listen -ErrorAction SilentlyContinue).OwningProcess"`,
      { encoding: 'utf8', windowsHide: true },
    );
    const pid = out.trim().split(/\r?\n/).map((s) => s.trim()).find((s) => /^\d+$/.test(s));
    return pid || null;
  } catch {
    return null;
  }
}

function isNodeProcess(pid) {
  try {
    const out = execSync(`tasklist /FI "PID eq ${pid}" /FO CSV /NH`, { encoding: 'utf8', windowsHide: true });
    return out.toLowerCase().includes('node.exe');
  } catch {
    return false;
  }
}

async function main() {
  if (await isPortFree(PORT)) {
    process.exit(0); // 端口空闲，直接启动
  }

  console.log(`[port] ${PORT} 被占用，检测残留 dev server ...`);
  const pid = getListeningPid(PORT);
  if (!pid) {
    console.error(`[port] ${PORT} 被占用但无法定位进程，请手动关闭后重试。`);
    process.exit(1);
  }
  if (!isNodeProcess(pid)) {
    console.error(`[port] ${PORT} 被非 Node 进程 (PID ${pid}) 占用，请手动关闭后重试。`);
    process.exit(1);
  }

  console.log(`[port] 检测到残留 node/vite 进程 (PID ${pid})，自动终止 ...`);
  const r = spawnSync('taskkill', ['/PID', pid, '/F', '/T'], { encoding: 'utf8', windowsHide: true });
  if (r.status !== 0) {
    console.error(`[port] 终止失败: ${(r.stderr || r.stdout || '').trim()}`);
    process.exit(1);
  }

  // 等待端口释放（taskkill 异步生效）
  for (let i = 0; i < 10; i++) {
    await new Promise((res) => setTimeout(res, 200));
    if (await isPortFree(PORT)) {
      console.log(`[port] ${PORT} 已释放，继续启动。`);
      process.exit(0);
    }
  }
  console.error(`[port] ${PORT} 终止后仍未释放，请手动检查。`);
  process.exit(1);
}

main();
