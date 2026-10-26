#!/usr/bin/env python3
"""Real 模块依赖方向检查器（ADR-009 可执行部分）

扫描 server/src 下所有 .rs 文件的 `use crate::X` 引用，验证是否违反分层纪律。
分层（ADR-009，目标结构）：
    orchestration（编排，顶层）
      ↓
    planning / execution / evaluation / memory（域逻辑，agent 子包）
      ↓
    tools / model / db / mcp / sse / path / security / cost / pricing
    / scheduler / config / state / error / context_gauge / backbone / prompts（支撑域）

用法：
    python server/tools/check_layers.py [--src DIR] [--verbose]
退出码：0 = 通过；1 = 存在违规。

规则说明：
- 每个模块声明 allowed（可依赖的 crate 顶层模块）；未列出者视为"不可依赖"。
- 豁免：db/repos → agent::plan 属数据契约（insert_plan 签名）。
- 注释掉的 use 不计；测试模块（#[cfg(test)] mod tests）从宽（测试可用 super::* 拉全量）。
"""
from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

# GBK 控制台下 print 叉/勾 会 UnicodeEncodeError，门禁静默失效 —— 强制 UTF-8 输出
try:
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")
except Exception:
    pass

# ── 分层规则（键 = crate 顶层模块；值 = 允许依赖的 crate 顶层模块集合）──
# "self" 表示模块内部互相引用（不检查）。
RULES: dict[str, set[str]] = {
    # 编排层：只做流程控制，依赖所有域逻辑 + 支撑域
    "routes": {"agent", "db", "model", "security", "sse", "error", "state", "config",
               "scheduler", "tools", "mcp", "cost", "pricing", "path", "context_gauge",
               "agent::memory", "auth"},  # agent::memory：记忆面板直调 journal；auth：登录态校验（2026-10-25 补登记）
    "agent::orchestration": {"agent::planning", "agent::execution", "agent::evaluation",
                             "agent::memory", "agent::context", "tools", "model", "db", "sse", "security",
                             "cost", "error", "mcp", "path", "state", "config", "pricing",
                             "context_gauge", "prompts", "backbone"},  # agent::context：切 prompt 段（零依赖）
    # 域逻辑：只依赖支撑域 + planning（memory 需 plan 的工具函数）
    "agent::planning": {"model", "mcp", "error", "prompts", "tools", "path"},
    "agent::execution": {"agent::planning", "mcp", "model", "security", "error", "db",
                         "path", "tools", "workspace", "prompts", "cost"},
    "agent::evaluation": {"agent::planning", "model", "error", "prompts"},
    "agent::memory": {"agent::planning", "db", "path", "error", "tools", "mcp"},  # mcp：project_profile 用 Registry 做项目预扫
    # 历史治理：只认消息类型与错误类型，不碰 db/orchestration（否则会绕回来）
    "agent::history": {"model", "error"},
    # 纯文本切段工具（从 prompt 文件切 marker 段、剥注释）：零 crate 依赖，最底层
    "agent::context": set(),
    # 支撑域：只依赖底层，禁止反向依赖 agent
    "tools": {"mcp", "path", "error", "contract", "db"},
    "mcp": {"tools", "error"},
    "model": {"error", "config"},
    "scheduler": {"agent::orchestration", "db", "model", "error", "state", "tools", "config"},
    "security": {"sse", "error", "db"},
    # 豁免：db/repos 使用 agent::plan 类型做数据契约（insert_plan），允许单向
    # config：连接池要拿数据库路径（2026-10-25 补登记）
    "db": {"agent::planning", "error", "config"},
    # 基础支撑：无外部依赖或仅依赖自身
    "path": set(),
    "sse": set(),
    "config": set(),
    # 组装根：AppState 就是那个把所有部件拿在手里的 struct，天然依赖它们。
    # 检查它的意义不在「少依赖」，而在禁止别人反向依赖它（除 routes/scheduler/sse）。
    "state": {"config", "mcp", "model", "sse"},
    "error": set(),
    "cost": {"pricing", "model", "error"},
    "pricing": set(),
    "context_gauge": set(),
    "backbone": set(),
    "prompts": set(),
    # ── 2026-10-25 补登记 ──
    # 背景：门禁原先只覆盖 90/159 个文件，下面这批模块根本没进规则表（等于不检查）。
    # 每条都按「它实际 use 了什么、方向对不对」定，不是照抄现状放行。
    "auth": {"error"},                       # 令牌签发/校验；凭证存 settings，不碰 db
    "confirm": {"path", "sse", "state"},     # 确认门：读写策略、推事件、持 AppState
    "facts": {"path"},                       # 事实存活判定，只查路径/环境变量存在性
    "wt": set(),                             # 磁盘扫描，零依赖 —— 这正是它可单测的原因
    "domains": {"mcp"},                      # 内置工具域，向 registry 注册
    "agent::evolution": {"model", "state"},
    "agent::session_title": {"model", "state", "agent::output_lang"},
    "agent::output_lang": {"error"},
    "agent::naming": set(),                  # 纯字符串规则，无依赖
    "agent::narration": set(),
    "agent::progress": set(),
    "agent::specs": set(),
}

# ── 已知豁免：方向确实违规，修它要动结构，先登记 ──
# 不藏起来：每次运行都会打出来。豁免不是「没问题」，是「已知、欠着」。
# 新增一条之前先问：是真的改不动，还是只是现在不想改。
KNOWN: dict[tuple[str, str], str] = {
    ("sse", "state"): "sse 内含 HTTP 端点（router/handler），与 state 成环；修法是把端点迁回 routes",
    ("sse", "db"): "同上：端点里的 handler 直查 repos",
    ("sse", "error"): "同上：端点返回 AppError",
    ("path", "tools"): "path 用 tools::contract::ToolError；根因是 contract 未独立成顶层模块",
    ("mcp", "agent::history"): "mcp 读 agent::history::th 的阈值常量；应改为向 config 取值",
}

# 顶层模块白名单（crate 根声明的 mod；不在表内的模块不检查其 out 依赖）
TOP_MODULES = set(RULES.keys()) | {"agent"}
# 入口：组装根，必然依赖全部；它没有「上游」可讲，只拦它别引用不存在的模块
RULES["main"] = set(TOP_MODULES) - {"main"}
RULES["agent"] = set()  # 模块根，只 re-export；这里出现 use 说明有逻辑塞错地方了

USE_RE = re.compile(r"^\s*use\s+crate::([a-zA-Z_][a-zA-Z0-9_:]*)", re.MULTILINE)


def crate_top_module(path: Path, src_root: Path) -> str | None:
    """文件所属的模块键 —— **必须落在规则表的键上**，否则规则永远不生效。

    三处修正（2026-10-25）：
    - **二级名问题（最严重）**：过去 `src/db/repos.rs` 返回 `db::repos`，而规则表的键是 `db`，
      于是 `db`/`tools`/`mcp`/`model`/`routes` 这些多文件模块**从录入规则那天起就没被检查过**。
      现在按顶层归并：`db/repos.rs` → `db`，`tools/cmd_bash.rs` → `tools`。
    - 平铺文件过去返回 `agent/naming.rs`（带后缀），永远匹配不上；现在剥 `.rs`。
    - 独立测试文件 `*_tests.rs` 与内嵌 `#[cfg(test)] mod tests` 同等从宽：返回 None 不检查。
      过去只认内嵌那种，于是 `memory_tests.rs` 里的测试 `use` 被当成生产依赖报违规。
    """
    name = path.name
    if name.endswith("_tests.rs"):
        return None
    parts = path.relative_to(src_root).as_posix().split("/")
    stem = name[:-3] if name.endswith(".rs") else name
    if parts[0] == "agent":
        # agent 是唯一按子包分层的：agent/memory/*.rs → agent::memory；agent/naming.rs → agent::naming
        if len(parts) >= 3:
            return f"agent::{parts[1]}"
        return "agent" if stem == "mod" else f"agent::{stem}"
    return stem if len(parts) == 1 else parts[0]


def normalize_dep(target: str, file_module: str) -> str | None:
    """use crate::X 的 X → 规则键。agent::plan 归并为 agent::planning（目标结构）。"""
    parts = target.split("::")
    top = parts[0]
    if top == "agent" and len(parts) >= 2:
        sub = parts[1]
        # 目标结构子包映射（旧平铺名 → 新子包）
        MIGRATE = {
            "plan": "planning", "planner": "planning", "placeholder": "planning",
            "tool_summary": "planning",
            "worker": "execution", "replanner": "execution", "prepare_args": "execution",
            "memory": "memory", "workspace": "memory", "project_profile": "memory",
            "solver": "evaluation", "verifier": "evaluation",
            "orchestrator": "orchestration", "events": "orchestration",
        }
        sub = MIGRATE.get(sub, sub)
        return f"agent::{sub}"
    if top == "self":
        return None
    return top if top in TOP_MODULES else None


def check_file(path: Path, src_root: Path, verbose: bool) -> tuple[list[str], list[str]]:
    text = path.read_text(encoding="utf-8", errors="replace")
    # 剔除注释块与字符串里的 use（粗略但够用：行首 use 才算）
    lines = text.splitlines()
    file_module = crate_top_module(path, src_root)
    if file_module is None or file_module not in RULES:
        return [], []
    allowed = RULES[file_module] | {"self"}

    violations: list[str] = []
    exempted: list[str] = []
    in_test_mod = False
    for lineno, line in enumerate(lines, 1):
        stripped = line.strip()
        if stripped.startswith("//") or stripped.startswith("/*") or stripped.startswith("*"):
            continue
        if "#[cfg(test)]" in stripped:
            in_test_mod = True
            continue
        if stripped.startswith("}") and in_test_mod:
            in_test_mod = False
        if in_test_mod:
            continue
        m = USE_RE.match(line)
        if not m:
            continue
        dep = normalize_dep(m.group(1), file_module)
        if dep is None or dep == file_module:  # 同包子模块互引（如 orchestration 内 events↔orchestrator）不算违规
            continue
        if dep in allowed:
            continue
        if (file_module, dep) in KNOWN:
            exempted.append(f"{path.relative_to(src_root).as_posix()}:{lineno}: {file_module} → {dep}")
            continue
        violations.append(f"{path.name}:{lineno}: {file_module} 依赖了未授权的 {dep}（use {m.group(1)}）")
    return violations, exempted


def main() -> int:
    ap = argparse.ArgumentParser(description="Real 模块依赖方向检查（ADR-009）")
    ap.add_argument("--src", default=None, help="server/src 目录（默认自动探测）")
    ap.add_argument("--verbose", action="store_true")
    ap.add_argument("--strict", action="store_true",
                    help="存在「未登记规则」的模块时也算失败（默认只报告，不算失败）")
    args = ap.parse_args()

    src_root = Path(args.src) if args.src else Path(__file__).resolve().parent.parent / "src"
    if not src_root.is_dir():
        print(f"错误：找不到 src 目录 {src_root}", file=sys.stderr)
        return 2

    all_violations: list[str] = []
    all_exempt: list[str] = []
    unchecked: dict[str, int] = {}   # 模块 → 文件数：规则表里没有，本轮没检查
    checked = 0
    skipped = 0                      # 测试文件（从宽）
    for path in sorted(src_root.rglob("*.rs")):
        if any(seg in {"target", "node_modules"} for seg in path.parts):
            continue
        fm = crate_top_module(path, src_root)
        if fm is None:
            skipped += 1
            continue
        if fm not in RULES:
            unchecked[fm] = unchecked.get(fm, 0) + 1
            continue
        checked += 1
        v, e = check_file(path, src_root, args.verbose)
        all_violations.extend(v)
        all_exempt.extend(e)

    if args.verbose:
        print(f"已检查 {checked} 个文件，规则模块 {len(RULES)} 个，测试文件从宽 {skipped} 个")

    # 未登记模块必须看得见：门禁不能假装检查了它没检查的东西
    if unchecked:
        names = "、".join(f"{k}({v})" for k, v in sorted(unchecked.items()))
        print(f"⚠️  {len(unchecked)} 个模块未登记规则，本轮未检查：{names}")

    # 已登记的违规同样每次都要出现，不然一年后没人记得它们
    if all_exempt:
        print(f"⚠️  {len(all_exempt)} 条已登记豁免（方向确实违规，欠着）：")
        for e in all_exempt:
            print(f"   {e}")
        print("   修完请从 KNOWN 里删掉对应项，否则它会一直绿着。")

    if all_violations:
        print("❌ 依赖方向违规：")
        for v in sorted(all_violations):
            print(f"   {v}")
        print(f"\n共 {len(all_violations)} 处违规。修复方向见 docs/adr/ADR-009-modular-boundaries.md")
        return 1

    if unchecked and args.strict:
        print(f"❌ --strict：{len(unchecked)} 个模块未登记规则，视为失败")
        return 1

    print(f"✅ 依赖方向检查通过（{checked} 个文件）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
