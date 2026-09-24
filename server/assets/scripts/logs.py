"""logs · 日志排查（尾部 / 关键词 / 去重计数 / 时间窗）

用法: python <脚本库>/logs.py <tail|grep|count> [关键词] [-n 40] [--file 路径] [--since-min N]
"""
import argparse
import io
import os
import re
import sys
from collections import Counter

sys.stdout.reconfigure(encoding="utf-8", errors="replace")

ANSI = re.compile(r"\x1b\[[0-9;]*m")


def default_log():
    root = os.environ.get("REAL_DATA_ROOT") or os.path.join(
        os.environ.get("APPDATA", ""), "real-agent"
    )
    return os.path.join(root, "logs", "backend.log")


def read_lines(path, since_min=0):
    if not os.path.isfile(path):
        sys.exit(f"日志不存在: {path}（用 --file 指定）")
    out = []
    with io.open(path, encoding="utf-8", errors="replace") as f:
        for line in f:
            out.append(ANSI.sub("", line).rstrip("\n"))
    return out


def main():
    ap = argparse.ArgumentParser(add_help=False)
    ap.add_argument("cmd", nargs="?", default="tail", help="tail|grep|count")
    ap.add_argument("kw", nargs="?", default="", help="关键词（grep/count 用）")
    ap.add_argument("-n", type=int, default=40)
    ap.add_argument("--file", default=None)
    ap.add_argument("--since-min", type=int, default=0, help="只看带时间戳且最近 N 分钟的行（尽力而为）")
    ap.add_argument("--help", "-h", action="store_true")
    a = ap.parse_args()
    if a.help:
        print(__doc__)
        return

    path = a.file or default_log()
    lines = read_lines(path, a.since_min)
    print(f"日志 {path}  共 {len(lines)} 行")

    if a.cmd == "tail":
        for l in lines[-a.n:]:
            print(l[:200])
    elif a.cmd in ("grep", "count"):
        if not a.kw:
            sys.exit("count/grep 需要关键词，例如: logs.py count WARN")
        hits = [l for l in lines if a.kw.lower() in l.lower()]
        print(f"\n命中「{a.kw}」{len(hits)} 行")
        if a.cmd == "count":
            c = Counter()
            for l in hits:
                body = re.sub(r"^\S+\s+(WARN|ERROR|INFO|DEBUG)\s+", "", l.strip())
                body = re.sub(r"session=\S+", "session=<id>", body)
                body = re.sub(r"\d+", "N", body)
                c[body[:110]] += 1
            print(f"\n{'次数':>5}  归一化后的形态（数字与会话 id 已折叠）")
            for body, n in c.most_common(a.n):
                print(f"{n:>5}  {body}")
        else:
            for l in hits[-a.n:]:
                print(f"  {l[:200]}")
    else:
        sys.exit(f"未知子命令: {a.cmd}（用 --help 看用法）")


if __name__ == "__main__":
    main()
