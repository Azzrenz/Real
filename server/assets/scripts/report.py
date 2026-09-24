"""report · 把结构化数据渲染成单文件交互式 HTML 报告（KPI 卡 / 分组折叠 / 搜索 / 标签）

用法: python <脚本库>/report.py --data <数据文件> --title <标题> [选项]

数据文件: .json（对象数组，或 {"rows": [...]}）/ .tsv / .csv
  --group <列>       按该列分组，每组一个折叠块，标题旁带条目数
  --group-note <列>  该列的值作为折叠块标题上的注释（取组内首个非空值）
  --note <列>        该列作为条目的说明文字，缩在首列下方（可重复）
  --tag <列>         该列的值映射成 ok / warn / bad / new 标签（大小写不敏感）
  --cols <a,b,c>     只展示这几列，按给定顺序（缺省：全部列，按数据里的出现顺序）
  --kpi <值=说明>    追加一张 KPI 卡（可重复）；另有自动统计：总条目 / 分组数 / 各标签数
  --sort <列>        按该列排序；--desc 反序
  --subtitle <文字>  副标题（数据来源之类）
  -o, --out <文件>   输出路径（缺省：与数据同目录同名 .html）
  --no-header        tsv/csv 无表头，列名用 0 基序号（"0" "1" ...）

例:
  python report.py --data _controls.tsv --title "Cubase 控件底表" --group 模块 --group-note 说明 --note 功能 --tag 状态
  python report.py --data rows.json --title "排查结论" --group 文件 --cols 行号,结论

输出: 单文件自包含 HTML（样式/脚本内联、无外部资源），可直接双击打开。
"""

import argparse
import csv
import html
import json
import os
import sys
from collections import Counter, OrderedDict

sys.stdout.reconfigure(encoding="utf-8", errors="replace")

TAG_ALIASES = {
    "ok": "ok", "done": "ok", "pass": "ok", "success": "ok", "yes": "ok",
    "已实现": "ok", "已完成": "ok", "通过": "ok", "是": "ok", "有": "ok",
    "warn": "warn", "warning": "warn", "pending": "warn", "partial": "warn",
    "待确认": "warn", "部分": "warn", "未确认": "warn", "存疑": "warn",
    "bad": "bad", "fail": "bad", "error": "bad", "no": "bad", "missing": "bad",
    "未实现": "bad", "缺失": "bad", "否": "bad", "无": "bad", "报错": "bad",
    "new": "new", "added": "new", "todo": "new", "待做": "new", "新增": "new", "计划": "new",
}

STYLE = """
:root{--bg:#1b1d21;--pan:#232629;--pan2:#2a2e33;--fg:#d7dae0;--dim:#8b939c;
--acc:#5aa9e6;--line:#33383e;--ok:#5ec27a;--warn:#e0a94f;--bad:#e06c6c;--new:#b48ee6;
--mono:"Cascadia Mono",Consolas,monospace}
@media (prefers-color-scheme:light){:root{--bg:#f6f7f9;--pan:#fff;--pan2:#eef1f4;--fg:#22262b;
--dim:#6b737c;--acc:#1f6fb2;--line:#dfe3e8;--ok:#1f7a44;--warn:#8a6100;--bad:#a52a2a;--new:#6b46a8}}
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--fg);
font:14px/1.6 "Microsoft YaHei",system-ui,sans-serif}
header{padding:22px 28px;background:var(--pan2);border-bottom:1px solid var(--line)}
h1{margin:0 0 6px;font-size:21px}
.sub{color:var(--dim);font-size:13px}
.wrap{padding:0 28px 60px;max-width:1500px;margin:0 auto}
.kpi{display:flex;gap:14px;flex-wrap:wrap;margin:16px 0}
.k{background:var(--pan);border:1px solid var(--line);border-radius:6px;padding:10px 16px;min-width:120px}
.k b{display:block;font-size:22px;color:var(--acc);font-family:var(--mono)}
.k span{font-size:12px;color:var(--dim)}
.bar{display:flex;gap:10px;align-items:center;margin:10px 0 18px}
#q{flex:1;padding:9px 12px;background:var(--pan);border:1px solid var(--line);color:var(--fg);
border-radius:6px;font-size:13px}
.bar button{padding:8px 14px;background:var(--pan);border:1px solid var(--line);color:var(--fg);
border-radius:6px;font-size:12px;cursor:pointer}
.bar button:hover{background:var(--pan2)}
.hint{color:var(--dim);font-size:12px;margin:0 0 14px}
details{background:var(--pan);border:1px solid var(--line);border-radius:6px;margin:8px 0}
summary{cursor:pointer;padding:10px 14px;font-weight:600;display:flex;gap:10px;
justify-content:space-between;align-items:center}
summary:hover{background:var(--pan2)}
summary .g{flex:1}
summary .gn{color:var(--dim);font-size:12px;font-weight:400}
summary .n{color:var(--dim);font-size:12px;font-family:var(--mono)}
table{width:100%;border-collapse:collapse;font-size:13px}
th,td{padding:6px 10px;border-bottom:1px solid var(--line);text-align:left;vertical-align:top}
th{background:var(--pan2);position:sticky;top:0;font-weight:600;font-size:12px;color:var(--dim)}
tr.row:hover td{background:var(--pan2)}
.note{color:var(--dim);font-size:12px;margin-top:2px}
.mono{font-family:var(--mono);font-size:12px}
.tag{display:inline-block;padding:1px 7px;border-radius:10px;font-size:11px;border:1px solid;white-space:nowrap}
.t-ok{color:var(--ok);border-color:var(--ok)}
.t-warn{color:var(--warn);border-color:var(--warn)}
.t-bad{color:var(--bad);border-color:var(--bad)}
.t-new{color:var(--new);border-color:var(--new)}
.legend{display:flex;gap:12px;flex-wrap:wrap;color:var(--dim);font-size:12px;margin:4px 0 16px}
.empty{color:var(--dim);font-size:13px;padding:10px 14px}
"""

SCRIPT = """
const q=document.getElementById('q');
const groups=[...document.querySelectorAll('details')];
const initial=new Map(groups.map(g=>[g,g.open]));
function apply(){
  const v=q.value.trim().toLowerCase();
  for(const d of groups){
    let hit=0;
    for(const tr of d.querySelectorAll('tr.row')){
      const on=!v||tr.textContent.toLowerCase().includes(v);
      tr.style.display=on?'':'none';
      if(on)hit++;
    }
    d.style.display=(v&&hit===0)?'none':'';
    d.open=v?hit>0:initial.get(d);
  }
}
q.addEventListener('input',apply);
document.getElementById('expand').addEventListener('click',()=>{
  for(const d of groups){d.open=true;initial.set(d,true);} });
document.getElementById('collapse').addEventListener('click',()=>{
  for(const g of groups){g.open=false;initial.set(g,false);} });
"""


def die(msg):
    print(f"错误: {msg}")
    sys.exit(2)


def load_table(path, has_header):
    """返回 (列名列表, 行列表)。行是 dict：列名 -> 字符串值。"""
    ext = os.path.splitext(path)[1].lower()
    if ext == ".json":
        try:
            with open(path, encoding="utf-8") as f:
                data = json.load(f)
        except OSError as e:
            die(f"读不了 {path}: {e}")
        except json.JSONDecodeError as e:
            die(f"{path} 不是合法 JSON: {e}")
        if isinstance(data, dict):
            for key in ("rows", "data", "items", "records"):
                if isinstance(data.get(key), list):
                    data = data[key]
                    break
        if not isinstance(data, list):
            die('JSON 顶层要是数组，或 {"rows": [...]}')
        rows = []
        for item in data:
            if not isinstance(item, dict):
                die("JSON 数组的元素要是对象（每行一个键值对）")
            rows.append({str(k): "" if v is None else str(v) for k, v in item.items()})
        if not rows:
            return [], []
        cols = list(OrderedDict.fromkeys(k for r in rows for k in r))
        return cols, rows

    if ext in (".tsv", ".csv", ".txt"):
        delim = "\t" if ext != ".csv" else ","
        try:
            with open(path, encoding="utf-8-sig", newline="") as f:
                raw = list(csv.reader(f, delimiter=delim))
        except OSError as e:
            die(f"读不了 {path}: {e}")
        raw = [r for r in raw if any(c.strip() for c in r)]
        if not raw:
            return [], []
        if has_header:
            cols = [c.strip() or f"col{i}" for i, c in enumerate(raw[0])]
            body = raw[1:]
        else:
            cols = [str(i) for i in range(len(raw[0]))]
            body = raw
        rows = [
            {cols[i]: (r[i] if i < len(r) else "").strip() for i in range(len(cols))}
            for r in body
        ]
        return cols, rows

    die(f"不认得的扩展名 {ext}（要 .json / .tsv / .csv）")


def split_list(value):
    return [p.strip() for p in (value or "").split(",") if p.strip()]


def tag_of(value):
    return TAG_ALIASES.get(str(value).strip().lower())


def build(title, subtitle, cols, groups, kpis, note_cols, tag_col, show_group_note):
    esc = html.escape
    out = [
        "<!DOCTYPE html><html lang=\"zh-CN\"><head><meta charset=\"utf-8\">",
        f"<title>{esc(title)}</title><style>{STYLE}</style></head><body>",
        f"<header><h1>{esc(title)}</h1>",
    ]
    if subtitle:
        out.append(f'<div class="sub">{esc(subtitle)}</div>')
    out.append("</header><div class=\"wrap\">")

    out.append('<div class="kpi">')
    for value, label in kpis:
        out.append(f'<div class="k"><b>{esc(str(value))}</b><span>{esc(str(label))}</span></div>')
    out.append("</div>")

    used_tags = sorted({t for g in groups for r in g["rows"] for t in [tag_of(r.get(tag_col, ""))] if t})
    if tag_col:
        legend = []
        for t in ("ok", "warn", "bad", "new"):
            if t in used_tags:
                legend.append(f'<span class="tag t-{t}">{t}</span>')
        if legend:
            out.append('<div class="legend">标签: ' + " ".join(legend) + "</div>")

    out.append(
        '<div class="bar"><input id="q" type="search" placeholder="搜索（组内条目命中时自动展开，全部未命中则该组隐藏）">'
        '<button id="expand" type="button">展开全部</button>'
        '<button id="collapse" type="button">收起全部</button></div>'
    )
    total = sum(len(g["rows"]) for g in groups)
    out.append(f'<p class="hint">共 {total} 条 / {len(groups)} 组</p>')

    for g in groups:
        note = f' <span class="gn">{esc(g["note"])}</span>' if g.get("note") else ""
        out.append("<details>")
        out.append(
            f'<summary><span class="g">{esc(g["name"])}{note}</span>'
            f'<span class="n">{len(g["rows"])}</span></summary>'
        )
        if not g["rows"]:
            out.append('<div class="empty">（无条目）</div></details>')
            continue
        out.append("<table><thead><tr>")
        for c in cols:
            out.append(f"<th>{esc(c)}</th>")
        out.append("</tr></thead><tbody>")
        for r in g["rows"]:
            out.append('<tr class="row">')
            for i, c in enumerate(cols):
                value = str(r.get(c, ""))
                t = tag_of(value) if (tag_col and c == tag_col) else None
                shown = f'<span class="tag t-{t}">{esc(value.strip())}</span>' if t else esc(value)
                if i == 0:
                    # 标签列没被展示时，把 pill 贴到首列，免得标签只出现在图例里
                    if tag_col and tag_col not in cols:
                        lead = tag_of(str(r.get(tag_col, "")))
                        if lead:
                            text = esc(str(r.get(tag_col, "")).strip())
                            shown = f'<span class="tag t-{lead}">{text}</span> ' + shown
                    for nc in note_cols:
                        text = str(r.get(nc, "")).strip()
                        if text:
                            shown += f'<div class="note">{esc(text)}</div>'
                out.append(f"<td>{shown}</td>")
            out.append("</tr>")
        out.append("</tbody></table></details>")

    out.append(f"</div><script>{SCRIPT}</script></body></html>")
    return "".join(out)


def main():
    ap = argparse.ArgumentParser(add_help=True, description="把结构化数据渲染成单文件交互式 HTML 报告")
    ap.add_argument("--data", required=True)
    ap.add_argument("--title", required=True)
    ap.add_argument("--subtitle", default="")
    ap.add_argument("--group", default="")
    ap.add_argument("--group-note", default="")
    ap.add_argument("--note", action="append", default=[])
    ap.add_argument("--tag", default="")
    ap.add_argument("--cols", default="")
    ap.add_argument("--kpi", action="append", default=[])
    ap.add_argument("--sort", default="")
    ap.add_argument("--desc", action="store_true")
    ap.add_argument("-o", "--out", default="")
    ap.add_argument("--no-header", action="store_true")
    args = ap.parse_args()

    if not os.path.exists(args.data):
        die(f"找不到数据文件 {args.data}")
    cols, rows = load_table(args.data, not args.no_header)
    if not cols:
        die("数据为空（没有列）")

    for name, col in (("--group", args.group), ("--tag", args.tag), ("--sort", args.sort),
                      ("--group-note", args.group_note)):
        if col and col not in cols:
            die(f"{name} 指定的列 {col!r} 不在数据里。可用列: {', '.join(cols)}")
    for col in args.note:
        if col not in cols:
            die(f"--note 指定的列 {col!r} 不在数据里。可用列: {', '.join(cols)}")

    show = split_list(args.cols) or list(cols)
    for col in show:
        if col not in cols:
            die(f"--cols 里的 {col!r} 不在数据里。可用列: {', '.join(cols)}")

    if args.sort:
        rows.sort(key=lambda r: r.get(args.sort, ""), reverse=args.desc)

    if args.group:
        buckets = OrderedDict()
        for r in rows:
            buckets.setdefault(r.get(args.group, "") or "(未分组)", []).append(r)
        groups = []
        for name, items in buckets.items():
            note = ""
            if args.group_note:
                for it in items:
                    if str(it.get(args.group_note, "")).strip():
                        note = str(it.get(args.group_note, "")).strip()
                        break
            groups.append({"name": name, "note": note, "rows": items})
    else:
        groups = [{"name": args.title, "note": "", "rows": rows}]

    kpis = []
    for spec in args.kpi:
        if "=" not in spec:
            die(f"--kpi 要写成 值=说明，收到 {spec!r}")
        value, label = spec.split("=", 1)
        kpis.append((value.strip(), label.strip()))
    if not kpis:
        kpis.append((len(rows), "条目"))
        if args.group:
            kpis.append((len(groups), "分组"))
        if args.tag:
            counts = Counter(tag_of(r.get(args.tag, "")) for r in rows)
            for t in ("ok", "warn", "bad", "new"):
                if counts.get(t):
                    kpis.append((counts[t], f"标签 {t}"))
        if len(kpis) == 1 and cols:
            kpis.append((len(cols), "列"))

    subtitle = args.subtitle or f"数据源: {os.path.abspath(args.data)}"
    doc = build(args.title, subtitle, show, groups, kpis, args.note, args.tag, args.group_note)

    out_path = args.out or os.path.splitext(os.path.abspath(args.data))[0] + ".html"
    try:
        with open(out_path, "w", encoding="utf-8") as f:
            f.write(doc)
    except OSError as e:
        die(f"写不了 {out_path}: {e}")

    print(f"已生成: {out_path}")
    print(f"条目 {len(rows)} / 分组 {len(groups)} / 列 {len(show)} / {len(doc)} 字节")
    if args.tag:
        counts = Counter(tag_of(r.get(args.tag, "")) for r in rows)
        marks = " ".join(f"{t}={counts[t]}" for t in ("ok", "warn", "bad", "new") if counts.get(t))
        if marks:
            print(f"标签: {marks}")


if __name__ == "__main__":
    main()
