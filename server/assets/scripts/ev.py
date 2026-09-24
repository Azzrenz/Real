"""ev · 查 Real 事件表（按 kind 计数 / 最近 N 条 / 按小时分布 / 错误扫描）

用法: python <脚本库>/ev.py <kinds|tail|hourly|errors> [--session last|ID] [--kind K] [-n 20] [--since 小时] [--db 路径]
"""
import argparse
import io
import json
import os
import sqlite3
import sys
from collections import Counter
from datetime import datetime, timedelta

sys.stdout.reconfigure(encoding="utf-8", errors="replace")


def default_db():
    root = os.environ.get("REAL_DATA_ROOT") or os.path.join(
        os.environ.get("APPDATA", ""), "real-agent"
    )
    return os.path.join(root, "db", "real.db")


def bj(ts):
    """UTC 时间戳串 → 北京时间 HH:MM:SS"""
    try:
        return (datetime.fromisoformat(ts.replace(" ", "T")) + timedelta(hours=8)).strftime("%H:%M:%S")
    except Exception:
        return ts[:19]


def connect(path):
    if not os.path.isfile(path):
        sys.exit(f"数据库不存在: {path}（用 --db 指定）")
    return sqlite3.connect(path)


def last_session(c):
    r = c.execute("SELECT id FROM sessions ORDER BY updated_at DESC LIMIT 1").fetchone()
    if not r:
        sys.exit("没有任何会话")
    return r[0]


def main():
    ap = argparse.ArgumentParser(add_help=False)
    ap.add_argument("cmd", nargs="?", default="kinds",
                    help="kinds|tail|hourly|errors")
    ap.add_argument("--session", default="last", help="last 或会话 id 前缀")
    ap.add_argument("--kind", default=None, help="限定事件类型")
    ap.add_argument("--since", type=int, default=0, help="只看最近 N 小时（0=全部）")
    ap.add_argument("-n", type=int, default=20, help="tail/errors 输出条数")
    ap.add_argument("--db", default=None)
    ap.add_argument("--help", "-h", action="store_true")
    a = ap.parse_args()
    if a.help:
        print(__doc__)
        return

    c = connect(a.db or default_db())
    sid = last_session(c) if a.session == "last" else a.session
    if a.session != "last":
        row = c.execute("SELECT id FROM sessions WHERE id LIKE ? LIMIT 1", (sid + "%",)).fetchone()
        if not row:
            sys.exit(f"找不到会话: {sid}")
        sid = row[0]

    where = "session_id = ?"
    args = [sid]
    if a.since:
        where += " AND created_at > datetime('now', ?)"
        args.append(f"-{a.since} hours")
    if a.kind:
        where += " AND kind = ?"
        args.append(a.kind)

    print(f"会话 {sid[:8]}  事件窗口: {'最近 %d 小时' % a.since if a.since else '全部'}")
    if a.cmd == "kinds":
        print(f"\n{'kind':24} {'条数':>9}")
        total = 0
        for k, n in c.execute(
            f"SELECT kind, COUNT(*) FROM events WHERE {where} GROUP BY kind ORDER BY 2 DESC", args
        ):
            print(f"{k:24} {n:>9,}")
            total += n
        print(f"{'合计':24} {total:>9,}")
    elif a.cmd == "hourly":
        print(f"\n{'时段(北京)':>10} {'条数':>9}")
        agg = Counter()
        for (ts,) in c.execute(f"SELECT created_at FROM events WHERE {where}", args):
            agg[bj(ts)[:2] + " 时"] += 1
        for h in sorted(agg):
            print(f"{h:>10} {agg[h]:>9,}")
    elif a.cmd == "tail":
        print(f"\n最近 {a.n} 条：")
        rows = list(c.execute(
            f"SELECT created_at, kind, payload_json FROM events WHERE {where} ORDER BY id DESC LIMIT ?",
            args + [a.n],
        ))
        for ts, k, pj in reversed(rows):
            print(f"  {bj(ts)} {k:20} {pj[:120]}")
    elif a.cmd == "errors":
        print(f"\n含 error 的事件（最近 {a.n} 条）：")
        n = 0
        for ts, k, pj in c.execute(
            f"SELECT created_at, kind, payload_json FROM events WHERE {where} ORDER BY id DESC", args
        ):
            low = pj.lower()
            if '"error"' in low or "error " in low or "failed" in low or "400" in low:
                print(f"  {bj(ts)} {k:20} {pj[:180]}")
                n += 1
                if n >= a.n:
                    break
        if n == 0:
            print("  （无）")
    else:
        sys.exit(f"未知子命令: {a.cmd}（用 --help 看用法）")


if __name__ == "__main__":
    main()
