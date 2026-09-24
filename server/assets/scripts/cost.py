"""cost · LLM 用量与花费（逐条 / 按小时 / 成本构成 + 缓存命中率）

用法: python <脚本库>/cost.py [--session last|ID] [--since 小时] [--hourly] [-n 25] [--db 路径]
"""
import argparse
import json
import os
import sqlite3
import sys
from collections import defaultdict
from datetime import datetime, timedelta

sys.stdout.reconfigure(encoding="utf-8", errors="replace")

# 单价（元/百万 token，估算用；与实际账单可能有差）——未命中输入 / 命中输入 / 输出
P_MISS, P_HIT, P_OUT = 2.0, 0.04, 8.0


def default_db():
    root = os.environ.get("REAL_DATA_ROOT") or os.path.join(
        os.environ.get("APPDATA", ""), "real-agent"
    )
    return os.path.join(root, "db", "real.db")


def bj(ts):
    try:
        return (datetime.fromisoformat(ts.replace(" ", "T")) + timedelta(hours=8)).strftime("%H:%M:%S")
    except Exception:
        return ts[:19]


def main():
    ap = argparse.ArgumentParser(add_help=False)
    ap.add_argument("--session", default="last")
    ap.add_argument("--since", type=int, default=0, help="只看最近 N 小时")
    ap.add_argument("--hourly", action="store_true", help="按小时汇总（不逐条）")
    ap.add_argument("-n", type=int, default=25)
    ap.add_argument("--db", default=None)
    ap.add_argument("--help", "-h", action="store_true")
    a = ap.parse_args()
    if a.help:
        print(__doc__)
        return

    db = a.db or default_db()
    if not os.path.isfile(db):
        sys.exit(f"数据库不存在: {db}")
    c = sqlite3.connect(db)
    sid = c.execute("SELECT id FROM sessions ORDER BY updated_at DESC LIMIT 1").fetchone()[0]
    if a.session != "last":
        row = c.execute("SELECT id FROM sessions WHERE id LIKE ? LIMIT 1", (a.session + "%",)).fetchone()
        if not row:
            sys.exit(f"找不到会话: {a.session}")
        sid = row[0]

    where = "session_id = ? AND kind = 'llm.usage'"
    args = [sid]
    if a.since:
        where += " AND created_at > datetime('now', ?)"
        args.append(f"-{a.since} hours")

    rows = list(c.execute(
        f"SELECT created_at, payload_json FROM events WHERE {where} ORDER BY id", args
    ))
    print(f"会话 {sid[:8]}  LLM 调用 {len(rows)} 次"
          f"{'（最近 %d 小时）' % a.since if a.since else ''}")
    if not rows:
        return

    miss = hit = out = 0
    cost = 0.0
    per_hour = defaultdict(lambda: [0, 0, 0, 0.0, 0])
    print(f"\n{'时间':>8} {'输入':>8} {'命中':>8} {'未命中':>8} {'输出':>6} {'花费¥':>8} {'命中率':>6}")
    for ts, pj in rows[-a.n:]:
        d = json.loads(pj)
        i = d.get("input") or 0
        h = d.get("cached") or 0
        o = d.get("output") or 0
        k = d.get("cost_yuan") or 0
        print(f"{bj(ts):>8} {i:>8,} {h:>8,} {max(i-h,0):>8,} {o:>6,} {k:>8.4f} "
              f"{(h*100//i if i else 0):>5}%")
    for ts, pj in rows:
        d = json.loads(pj)
        i = d.get("input") or 0
        h = d.get("cached") or 0
        o = d.get("output") or 0
        k = d.get("cost_yuan") or 0
        miss += max(i - h, 0)
        hit += h
        out += o
        cost += k
        b = per_hour[bj(ts)[:2]]
        b[0] += 1
        b[1] += max(i - h, 0)
        b[2] += o
        b[3] += k
        b[4] += i

    if a.hourly:
        print(f"\n{'时段':>6} {'调用':>5} {'未命中':>9} {'输出':>7} {'花费¥':>8} {'命中率':>6}")
        for h in sorted(per_hour):
            n, m, o, k, i = per_hour[h]
            print(f"{h + ' 时':>6} {n:>5} {m:>9,} {o:>7,} {k:>8.4f} "
                  f"{(100 - m*100//max(i,1)):>5}%")

    cm, ch, co = miss * P_MISS / 1e6, hit * P_HIT / 1e6, out * P_OUT / 1e6
    t = cm + ch + co
    print(f"\n合计 未命中 {miss:,} / 命中 {hit:,} / 输出 {out:,} → ¥{cost:.4f}")
    if t:
        print(f"单价反推构成：未命中 {cm*100//t}% / 输出 {co*100//t}% / 命中 {ch*100//t}%"
              f"（按 ¥{P_MISS}/{P_HIT}/{P_OUT} 每百万估算）")
    print(f"平均单次 ¥{cost/max(len(rows),1):.4f}")


if __name__ == "__main__":
    main()
