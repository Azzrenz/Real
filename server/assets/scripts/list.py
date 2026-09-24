"""list · 列出脚本库里有哪些工具（名字 + 一句话用途 + 用法）

用法: python <脚本库>/list.py
"""
import io
import os
import re
import sys

sys.stdout.reconfigure(encoding="utf-8", errors="replace")

HERE = os.path.dirname(os.path.abspath(__file__))


def docline(path):
    """取文件顶部 docstring 的第一行作为用途（约定：脚本第一条 docstring 首行=用途）。"""
    try:
        with io.open(path, encoding="utf-8") as f:
            head = f.read(1200)
    except OSError:
        return "(读取失败)"
    m = re.match(r'\s*(?:"""|\'\'\')([^\n]*)', head)
    if m:
        return m.group(1).strip()
    for line in head.splitlines():
        if line.startswith("#"):
            return line.lstrip("# ").strip()
    return "(无说明)"


def main():
    files = sorted(f for f in os.listdir(HERE) if f.endswith(".py") and f != "list.py")
    print(f"脚本库: {HERE}")
    print(f"共 {len(files)} 个\n")
    for f in files:
        usage = ""
        try:
            with io.open(os.path.join(HERE, f), encoding="utf-8") as fh:
                for line in fh:
                    if line.startswith("用法:"):
                        usage = line.strip()[3:].strip()
                        break
        except OSError:
            pass
        print(f"- {f[:-3]:8} {docline(os.path.join(HERE, f))}")
        if usage:
            print(f"           {usage}")
    print("\n看某脚本完整用法: python <脚本库>/<名>.py --help")


if __name__ == "__main__":
    main()
