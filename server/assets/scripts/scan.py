"""scan · 目录扫描（按扩展名计数 / 最大文件 / 关键词命中文件）

用法: python <脚本库>/scan.py <ext|big|hits> <目录> [关键词] [-n 20] [--skip target,node_modules]
"""
import argparse
import io
import os
import sys
from collections import Counter

sys.stdout.reconfigure(encoding="utf-8", errors="replace")

DEFAULT_SKIP = {".git", "target", "node_modules", "dist", "build", ".next", "__pycache__"}


def walk(root, skip):
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in dirnames if d not in skip]
        for f in filenames:
            yield os.path.join(dirpath, f)


def main():
    ap = argparse.ArgumentParser(add_help=False)
    ap.add_argument("cmd", nargs="?", help="ext|big|hits")
    ap.add_argument("root", nargs="?", default=".")
    ap.add_argument("kw", nargs="?", default="")
    ap.add_argument("-n", type=int, default=20)
    ap.add_argument("--skip", default="")
    ap.add_argument("--help", "-h", action="store_true")
    a = ap.parse_args()
    if a.help or not a.cmd:
        print(__doc__)
        return
    root = os.path.abspath(a.root)
    if not os.path.isdir(root):
        sys.exit(f"不是目录: {root}")
    skip = DEFAULT_SKIP | {s for s in a.skip.split(",") if s}

    if a.cmd == "ext":
        c = Counter()
        n = 0
        for p in walk(root, skip):
            c[os.path.splitext(p)[1].lower() or "(无扩展名)"] += 1
            n += 1
        print(f"{root}  文件 {n:,} 个（已跳过 {', '.join(sorted(skip))}）")
        print(f"\n{'扩展名':>12} {'个数':>7}")
        for e, k in c.most_common(a.n):
            print(f"{e:>12} {k:>7,}")
    elif a.cmd == "big":
        items = []
        for p in walk(root, skip):
            try:
                items.append((os.path.getsize(p), p))
            except OSError:
                pass
        items.sort(reverse=True)
        print(f"{root}  最大 {a.n} 个文件：")
        for size, p in items[:a.n]:
            print(f"  {size:>12,} B  {os.path.relpath(p, root)}")
    elif a.cmd == "hits":
        if not a.kw:
            sys.exit("hits 需要关键词，例如: scan.py hits D:/proj TODO")
        needle = a.kw.lower().encode()
        hits = []
        for p in walk(root, skip):
            try:
                with open(p, "rb") as f:
                    data = f.read(2_000_000)
            except OSError:
                continue
            if needle in data.lower():
                hits.append((p, data.lower().count(needle)))
        hits.sort(key=lambda x: -x[1])
        print(f"含「{a.kw}」的文件 {len(hits)} 个：")
        for p, k in hits[:a.n]:
            print(f"  {k:>4} 处  {os.path.relpath(p, root)}")
    else:
        sys.exit(f"未知子命令: {a.cmd}（用 --help 看用法）")


if __name__ == "__main__":
    main()
