# -*- coding: utf-8 -*-
"""把 v<版本> 的资产发到 GitHub Release（token 取自 git 凭据管理器，无需手填）。

用法：
    python scripts/release/publish_github.py --version 0.1.1 --dir "D:\\Real-发行包\\v0.1.1"

行为：
  1) Release 不存在 → 创建（tag 若未推送，GitHub 会按 target 分支现打）；
  2) 同名资产已存在 → 先删再传（保证替换成新包）；
  3) 正文取 --dir/RELEASE_NOTES.md。
需要 HTTPS_PROXY（国内直连 443 不通时）。
"""
import argparse
import json
import mimetypes
import subprocess
import sys
import urllib.error
import urllib.request
from pathlib import Path

OWNER, REPO = "Azzrenz", "Real"
UPLOAD_ORDER = [
    "Real_{v}_x64_en-US.msi",
    "Real-{v}-win-x64.zip",
    "Real_{v}_x64_en-US.msi.sha256",
    "Real-{v}-win-x64.zip.sha256",
    "RELEASE_NOTES.md",
]


def token() -> str:
    p = subprocess.run(
        ["git", "credential", "fill"],
        input="protocol=https\nhost=github.com\n\n",
        capture_output=True, text=True, cwd=str(Path(__file__).resolve().parents[2]),
    )
    for line in p.stdout.splitlines():
        if line.startswith("password="):
            return line.split("=", 1)[1]
    sys.exit("[x] 取不到 GitHub 凭据：请先在浏览器/命令行登录一次 GitHub")


def req(method, url, tok, data=None, headers=None, raw=False):
    h = {"Authorization": "Bearer " + tok, "Accept": "application/vnd.github+json",
         "User-Agent": "real-release", "X-GitHub-Api-Version": "2022-11-28"}
    if headers:
        h.update(headers)
    body = data if isinstance(data, (bytes, type(None))) else json.dumps(data).encode()
    r = urllib.request.Request(url, data=body, headers=h, method=method)
    try:
        with urllib.request.urlopen(r, timeout=180) as resp:
            payload = resp.read()
            return json.loads(payload) if payload and not raw else payload
    except urllib.error.HTTPError as e:
        detail = e.read().decode(errors="replace")[:400]
        print(f"[!] {method} {url} -> {e.code} {detail}")
        raise


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--version", required=True)
    ap.add_argument("--dir", required=True)
    ap.add_argument("--notes", default=None)
    args = ap.parse_args()

    v = args.version
    tag = f"v{v}"
    d = Path(args.dir)
    tok = token()
    api = f"https://api.github.com/repos/{OWNER}/{REPO}"

    # 1) 找或建 Release
    try:
        rel = req("GET", f"{api}/releases/tags/{tag}", tok)
        print(f"[=] Release {tag} 已存在（id={rel['id']}）")
    except urllib.error.HTTPError:
        body_txt = (Path(args.notes) if args.notes else d / "RELEASE_NOTES.md").read_text(encoding="utf-8")
        rel = req("POST", f"{api}/releases", tok, {
            "tag_name": tag,
            "name": f"Real {tag}",
            "body": body_txt,
            "draft": False,
            "prerelease": False,
        })
        print(f"[+] 已创建 Release {tag}（id={rel['id']}）")

    # 2) 已有资产先删（替换语义）
    existing = {a["name"]: a["id"] for a in rel.get("assets", [])}
    for name in existing:
        req("DELETE", f"{api}/releases/assets/{existing[name]}", tok)
        print(f"[-] 删除旧资产 {name}")

    # 3) 上传
    names = [n.format(v=v) for n in UPLOAD_ORDER]
    for name in names:
        f = d / name
        if not f.is_file():
            print(f"[!] 缺文件，跳过：{name}")
            continue
        ctype = mimetypes.guess_type(name)[0] or "application/octet-stream"
        up = f"https://uploads.github.com/repos/{OWNER}/{REPO}/releases/{rel['id']}/assets?name={urllib.parse.quote(name)}"
        r = urllib.request.Request(up, data=f.read_bytes(), method="POST", headers={
            "Authorization": "Bearer " + tok,
            "Accept": "application/vnd.github+json",
            "Content-Type": ctype,
            "User-Agent": "real-release",
        })
        with urllib.request.urlopen(r, timeout=600) as resp:
            j = json.loads(resp.read())
        print(f"[+] 上传 {name}  {f.stat().st_size / 1048576:.1f}MB")

    # 4) 正文刷新
    notes = (Path(args.notes) if args.notes else d / "RELEASE_NOTES.md")
    if notes.is_file():
        req("PATCH", f"{api}/releases/{rel['id']}", tok,
            {"body": notes.read_text(encoding="utf-8")})
        print("[=] 发布说明已刷新")
    print(f"\n完成：https://github.com/{OWNER}/{REPO}/releases/tag/{tag}")


if __name__ == "__main__":
    import urllib.parse  # noqa: E402
    main()
