# -*- coding: utf-8 -*-
"""把 winget 三件套自动提交为 microsoft/winget-pkgs 的 PR。

用法：
    python scripts/release/publish_winget.py --version 0.1.1 --dir "D:\\Real-发行包\\v0.1.1\\winget"

流程：fork（若已存在则复用）→ 建分支 → 上传 3 个 YAML → 开 PR。
token 取自 git 凭据管理器；需要 HTTPS_PROXY（国内直连 443 不通时）。
"""
import argparse
import base64
import json
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

OWNER = "Azzrenz"
UPSTREAM = "microsoft/winget-pkgs"
ROOT = Path(__file__).resolve().parents[2]


def token() -> str:
    p = subprocess.run(["git", "credential", "fill"],
                       input="protocol=https\nhost=github.com\n\n",
                       capture_output=True, text=True, cwd=str(ROOT))
    for line in p.stdout.splitlines():
        if line.startswith("password="):
            return line.split("=", 1)[1]
    sys.exit("[x] 取不到 GitHub 凭据")


def api(method, url, tok, data=None, ok404=False):
    body = None if data is None else json.dumps(data).encode()
    h = {"Authorization": "Bearer " + tok, "Accept": "application/vnd.github+json",
         "X-GitHub-Api-Version": "2022-11-28", "User-Agent": "real-winget"}
    if body:
        h["Content-Type"] = "application/json"
    r = urllib.request.Request(url, data=body, headers=h, method=method)
    try:
        with urllib.request.urlopen(r, timeout=180) as resp:
            raw = resp.read()
            return json.loads(raw) if raw else {}
    except urllib.error.HTTPError as e:
        if ok404 and e.code == 404:
            return None
        print(f"[!] {method} {url} -> {e.code} {e.read().decode(errors='replace')[:300]}")
        raise


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--version", required=True)
    ap.add_argument("--dir", required=True)
    args = ap.parse_args()
    v = args.version
    tok = token()
    d = Path(args.dir)
    files = sorted(d.glob("*.yaml"))
    if len(files) != 3:
        sys.exit(f"[x] 期望 3 个 YAML，实际 {len(files)}")
    print("[1/5] 检查 fork …")
    fork = api("GET", f"https://api.github.com/repos/{OWNER}/winget-pkgs", tok, ok404=True)
    if fork is None:
        print("      创建 fork（大仓库，可能要一会儿）…")
        api("POST", f"https://api.github.com/repos/{UPSTREAM}/forks", tok, {})
        for _ in range(30):
            time.sleep(5)
            fork = api("GET", f"https://api.github.com/repos/{OWNER}/winget-pkgs", tok, ok404=True)
            if fork:
                break
        if not fork:
            sys.exit("[x] fork 未就绪，稍后重试")
    else:
        print("      已有 fork，复用")

    print("[2/5] 建分支 …")
    branch = f"real-{v}"
    base = api("GET", f"https://api.github.com/repos/{OWNER}/winget-pkgs/git/ref/heads/master", tok)
    base_sha = base["object"]["sha"]
    try:
        api("POST", f"https://api.github.com/repos/{OWNER}/winget-pkgs/git/refs", tok,
            {"ref": f"refs/heads/{branch}", "sha": base_sha})
        print(f"      {branch} ← master@{base_sha[:8]}")
    except urllib.error.HTTPError:
        print("      分支已存在，复用")

    print("[3/5] 上传清单 …")
    rel_dir = f"manifests/a/Azzrenz/Real/{v}"
    for f in files:
        content = base64.b64encode(f.read_bytes()).decode()
        path = f"{rel_dir}/{f.name}"
        url = f"https://api.github.com/repos/{OWNER}/winget-pkgs/contents/{path}"
        existing = api("GET", f"{url}?ref={branch}", tok, ok404=True)
        payload = {"message": f"Add Azzrenz.Real {v}: {f.name}",
                   "content": content, "branch": branch}
        if existing:
            payload["sha"] = existing["sha"]
        api("PUT", url, tok, payload)
        print(f"      {path}")

    print("[4/5] 开 PR …")
    pr = api("POST", f"https://api.github.com/repos/{UPSTREAM}/pulls", tok, {
        "title": f"New package: Azzrenz.Real version {v}",
        "head": f"{OWNER}:{branch}",
        "base": "master",
        "body": (
            f"本地 AI 工作台 Real {v}（Windows x64，MIT）。\n\n"
            "- 一句话：模型负责判断，代码负责事实；路径/退出码/字节一致性由后端裁决。\n"
            "- 发行：GitHub Releases 提供 msi 与绿色版 zip，本 PR 提交 msi 清单。\n"
            f"- 校验：安装包 SHA256 见 installer.yaml。\n"
            f"- 仓库：https://github.com/{OWNER}/Real\n"
        ),
        "maintainer_can_modify": True,
    })
    print(f"[5/5] PR: {pr['html_url']}")


if __name__ == "__main__":
    main()
