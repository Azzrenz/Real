"""svc · 服务与进程排查（端口占用 / 进程列表 / exe 与运行进程的时间对照）

用法: python <脚本库>/svc.py <port|proc|exe|http> <目标> [-n 20]
"""
import argparse
import io
import json
import os
import re
import subprocess
import sys
import urllib.request
from datetime import datetime

sys.stdout.reconfigure(encoding="utf-8", errors="replace")


def sh(cmd):
    p = subprocess.run(cmd, shell=True, capture_output=True, text=True, errors="replace")
    return p.stdout or "", p.stderr or "", p.returncode


def ps(script):
    out, err, rc = sh(f'powershell -NoProfile -Command "{script}"')
    return out.strip(), err.strip(), rc


def cmd_port(target):
    out, _, _ = sh(f"netstat -ano | findstr :{target}")
    if not out.strip():
        print(f"端口 {target}: 未在监听")
        return
    pids = sorted({m.group(1) for m in re.finditer(r"LISTENING\s+(\d+)", out)})
    print(f"端口 {target}: 正在监听，PID {', '.join(pids) or '(未知)'}")
    for pid in pids:
        t, _, _ = sh(f'tasklist /FI "PID eq {pid}" /FO CSV /NH')
        print("  " + t.strip().replace('","', "  ").strip('"'))


def cmd_proc(name):
    out, _, _ = sh(f'tasklist /FI "IMAGENAME eq {name}*" /FO CSV /NH')
    if not out.strip() or "No tasks" in out:
        print(f"进程 {name}: 未找到")
        return
    for line in out.strip().splitlines():
        print("  " + line.replace('","', "  ").strip('"'))
    t, _, _ = ps(
        f"Get-Process -Name '{name.rstrip('.exe')}' -ErrorAction SilentlyContinue | "
        "Select-Object Id,StartTime | ConvertTo-Json -Compress"
    )
    if t:
        print("  启动时间: " + t.replace("\n", " "))


def cmd_exe(path):
    if not os.path.isfile(path):
        print(f"文件不存在: {path}")
        return
    mt = datetime.fromtimestamp(os.path.getmtime(path)).strftime("%Y-%m-%d %H:%M:%S")
    print(f"{path}")
    print(f"  文件修改时间: {mt}  大小 {os.path.getsize(path):,} 字节")
    stem = os.path.splitext(os.path.basename(path))[0]
    t, _, _ = ps(
        f"Get-Process -Name '{stem}' -ErrorAction SilentlyContinue | "
        "Select-Object Id,StartTime,Path | ConvertTo-Json -Compress"
    )
    if not t:
        print(f"  进程 {stem}: 未在运行")
        return
    try:
        data = json.loads(t)
        data = data if isinstance(data, list) else [data]
    except Exception:
        print("  " + t.replace("\n", " "))
        return
    for d in data:
        st = str(d.get("StartTime", ""))[:19]
        print(f"  运行中 PID {d.get('Id')}  启动 {st}  exe={d.get('Path')}")
    print("  判读: 启动时间 早于 文件修改时间 → 跑的是**旧二进制**（需重启）")


def cmd_http(url):
    try:
        with urllib.request.urlopen(url, timeout=5) as r:
            print(f"{url} → HTTP {r.status}（{len(r.read(400))} 字节起）")
    except Exception as e:
        print(f"{url} → 不可达: {type(e).__name__}: {e}")


def main():
    ap = argparse.ArgumentParser(add_help=False)
    ap.add_argument("cmd", nargs="?", help="port|proc|exe|http")
    ap.add_argument("target", nargs="?", default="")
    ap.add_argument("--help", "-h", action="store_true")
    a = ap.parse_args()
    if a.help or not a.cmd:
        print(__doc__)
        return
    {"port": cmd_port, "proc": cmd_proc, "exe": cmd_exe, "http": cmd_http}.get(
        a.cmd, lambda _: sys.exit(f"未知子命令: {a.cmd}（用 --help 看用法）")
    )(a.target)


if __name__ == "__main__":
    main()
