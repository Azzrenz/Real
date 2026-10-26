# -*- coding: utf-8 -*-
"""Real 发版工具：打包绿色版 → 算哈希 → 生成 winget / scoop / 发布说明。

用法（在仓库根目录）：
    python scripts/release/release.py --version 0.1.1

产物落在 --out（默认 D:\\Real-发行包\\v<版本>）：
    Real_<ver>_x64_en-US.msi           安装版（Tauri 产出，直接复制）
    Real-<ver>-win-x64.zip             绿色版（两个 exe + 说明）
    Real_<ver>_x64_en-US.msi.sha256    校验
    Real-<ver>-win-x64.zip.sha256
    winget/*.yaml                      winget 三件套（提交 microsoft/winget-pkgs 用）
    scoop/real.json                    scoop 清单
    RELEASE_NOTES.md                   发布说明（人话 + 下载表 + 校验 + 已知限制）

设计口径与 DSH 对齐：源码在 GitHub，用户不走 Release 页翻包，
而是一条命令拿到（winget / scoop），Release 只做资产与台账。
"""
import argparse
import hashlib
import json
import shutil
import subprocess
import sys
import time
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def sha256_of(p: Path) -> str:
    h = hashlib.sha256()
    with p.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def find_msi(version: str) -> Path:
    base = ROOT / "client" / "src-tauri" / "target" / "release" / "bundle" / "msi"
    hit = sorted(base.glob("*.msi"))
    if not hit:
        sys.exit(f"[x] 没找到 msi：{base}")
    return hit[-1]


def find_exe(name: str) -> Path:
    for p in (
        ROOT / "client" / "src-tauri" / "target" / "release" / name,
        ROOT / "client" / "src-tauri" / "binaries" / name,
        ROOT / "server" / "target" / "release" / name,
    ):
        if p.is_file():
            return p
    sys.exit(f"[x] 没找到 {name}，先跑 npm run release")


def build_portable(version: str, out: Path) -> Path:
    stage = out / f"Real-{version}-win-x64"
    if stage.exists():
        shutil.rmtree(stage)
    stage.mkdir(parents=True)
    for name in ("real-client.exe", "real-server.exe"):
        shutil.copy2(find_exe(name), stage / name)
    # 说明文件：有就用仓库里的，没有就跳过
    for src, dst in (
        (ROOT / "docs" / "INSTALL.md", "使用说明.txt"),
        (ROOT / "docs" / "env.example.txt", "env.example.txt"),
    ):
        if src.is_file():
            shutil.copy2(src, stage / dst)
    zpath = out / f"Real-{version}-win-x64.zip"
    if zpath.exists():
        zpath.unlink()
    with zipfile.ZipFile(zpath, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
        for f in sorted(stage.rglob("*")):
            if f.is_file():
                z.write(f, f.relative_to(stage))
    shutil.rmtree(stage)
    return zpath


def write_sha(path: Path) -> str:
    h = sha256_of(path)
    path.with_name(path.name + ".sha256").write_text(f"{h}  {path.name}\n", encoding="utf-8")
    return h


WINGET_VERSION = """# yaml-language-server: $schema=https://aka.ms/winget-manifest.version.1.6.0.schema.json
PackageIdentifier: Azzrenz.Real
PackageVersion: {version}
DefaultLocale: zh-CN
ManifestType: version
ManifestVersion: 1.6.0
"""

WINGET_INSTALLER = """# yaml-language-server: $schema=https://aka.ms/winget-manifest.installer.1.6.0.schema.json
PackageIdentifier: Azzrenz.Real
PackageVersion: {version}
InstallerType: wix
Scope: user
UpgradeBehavior: install
InstallModes:
  - interactive
  - silent
  - silentWithProgress
Installers:
  - Architecture: x64
    InstallerUrl: https://github.com/Azzrenz/Real/releases/download/v{version}/Real_{version}_x64_en-US.msi
    InstallerSha256: {sha}
ManifestType: installer
ManifestVersion: 1.6.0
"""

WINGET_LOCALE = """# yaml-language-server: $schema=https://aka.ms/winget-manifest.defaultLocale.1.6.0.schema.json
PackageIdentifier: Azzrenz.Real
PackageVersion: {version}
PackageLocale: zh-CN
Publisher: Azzrenz
PackageName: Real
PackageUrl: https://github.com/Azzrenz/Real
License: MIT
LicenseUrl: https://github.com/Azzrenz/Real/blob/main/LICENSE
ShortDescription: 本地 AI 工作台——模型负责判断，代码负责事实。
Description: 安装在 Windows 上的开源 AI 工具：给它一个文件夹、一句话，它在本地读、改、跑，做完了才回来。确定性的事由后端算（路径/退出码/字节一致性），判不出的如实说判不出。
Tags:
  - ai
  - agent
  - local-first
  - tauri
ManifestType: defaultLocale
ManifestVersion: 1.6.0
"""


def write_winget(version: str, msi_sha: str, out: Path) -> Path:
    d = out / "winget"
    d.mkdir(exist_ok=True)
    (d / "Azzrenz.Real.yaml").write_text(WINGET_VERSION.format(version=version), encoding="utf-8")
    (d / "Azzrenz.Real.installer.yaml").write_text(
        WINGET_INSTALLER.format(version=version, sha=msi_sha), encoding="utf-8")
    (d / "Azzrenz.Real.locale.zh-CN.yaml").write_text(
        WINGET_LOCALE.format(version=version), encoding="utf-8")
    return d


def write_scoop(version: str, zip_sha: str, out: Path) -> Path:
    d = out / "scoop"
    d.mkdir(exist_ok=True)
    manifest = {
        "version": version,
        "description": "本地 AI 工作台——模型负责判断，代码负责事实。",
        "homepage": "https://github.com/Azzrenz/Real",
        "license": "MIT",
        "architecture": {
            "64bit": {
                "url": f"https://github.com/Azzrenz/Real/releases/download/v{version}/Real-{version}-win-x64.zip",
                "hash": zip_sha,
            }
        },
        "shortcuts": [["real-client.exe", "Real"]],
        "checkver": {"github": "https://github.com/Azzrenz/Real"},
        "autoupdate": {
            "architecture": {
                "64bit": {
                    "url": f"https://github.com/Azzrenz/Real/releases/download/v$version/Real-$version-win-x64.zip"
                }
            }
        },
    }
    p = d / "real.json"
    p.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return p


NOTES = """# Real v{version}

> {tagline}

## 本次修了什么（人话）

{changes}

## 下载

| 方式 | 适合谁 | 文件 |
|---|---|---|
| 安装版（msi） | 常规使用，装完有开始菜单项 | `Real_{version}_x64_en-US.msi` |
| 绿色版（zip） | 不想装、想放 U 盘或移动硬盘 | `Real-{version}-win-x64.zip` |
| 国内加速 | GitHub 慢的时候 | Gitee 镜像（同步发布） |

命令行安装（Windows 包管理器）：

```powershell
winget install Azzrenz.Real
# 或（绿色版）
scoop install https://raw.githubusercontent.com/Azzrenz/Real/main/scoop/real.json
```

## 校验

```
{msi_name}
  SHA256 {msi_sha}

{zip_name}
  SHA256 {zip_sha}
```

## 已知限制

- 首次启动需要填 API Key；没有 Key 时客户端会提示，不会静默失败。
- 本机若中途安装 Python，编辑校验要重启客户端才生效。
- 只提供 Windows x64 构建；其他平台未验证。

## 许可

MIT。源码：https://github.com/Azzrenz/Real
"""


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--version", required=True)
    ap.add_argument("--out", default=None)
    ap.add_argument("--tagline", default="模型管这一次调用，后端管这一次事实。")
    ap.add_argument("--changes-file", default=None)
    args = ap.parse_args()

    v = args.version
    out = Path(args.out) if args.out else Path(r"D:\Real-发行包") / f"v{v}"
    out.mkdir(parents=True, exist_ok=True)

    print(f"[1/4] 收集安装版 msi …")
    msi_src = find_msi(v)
    msi = out / f"Real_{v}_x64_en-US.msi"
    shutil.copy2(msi_src, msi)
    msi_sha = write_sha(msi)
    print(f"      {msi.name}  {msi.stat().st_size / 1048576:.1f}MB  {msi_sha[:16]}…")

    print(f"[2/4] 打包绿色版 zip …")
    zp = build_portable(v, out)
    zip_sha = write_sha(zp)
    print(f"      {zp.name}  {zp.stat().st_size / 1048576:.1f}MB  {zip_sha[:16]}…")

    print(f"[3/4] 生成 winget / scoop 清单 …")
    wd = write_winget(v, msi_sha, out)
    sp = write_scoop(v, zip_sha, out)
    print(f"      {wd}  /  {sp}")

    print(f"[4/4] 生成发布说明 …")
    changes = "- （在此填写人话变更）"
    if args.changes_file:
        changes = Path(args.changes_file).read_text(encoding="utf-8").strip()
    notes = NOTES.format(
        version=v, tagline=args.tagline, changes=changes,
        msi_name=msi.name, msi_sha=msi_sha, zip_name=zp.name, zip_sha=zip_sha,
    )
    (out / "RELEASE_NOTES.md").write_text(notes, encoding="utf-8")
    print(f"      {out / 'RELEASE_NOTES.md'}")

    print("\n下一步：")
    print(f"  python scripts/release/publish_github.py --version {v} --dir \"{out}\"")
    print(f"  （先在仓库里提交版本号并打 tag：git tag v{v} && git push origin v{v}）")


if __name__ == "__main__":
    main()
