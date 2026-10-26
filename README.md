# Real

<p>
  <a href="https://github.com/Azzrenz/Real/releases/latest"><img alt="Release" src="https://img.shields.io/github/v/release/Azzrenz/Real"></a>
  <a href="LICENSE"><img alt="License" src="https://img.shields.io/badge/license-MIT-blue"></a>
  <img alt="Platform" src="https://img.shields.io/badge/platform-Windows%20%C2%B7%20verified-lightgrey">
  <img alt="Stack" src="https://img.shields.io/badge/stack-Rust%20%2B%20Tauri%202-orange">
</p>

**能交给模型的都交给模型，能算准的都交给后端。**

### 这是什么

Real 是 Windows 上的本地 AI 工作台（开源，MIT）。读代码、改代码、跑命令、跑测试——同类工具都能做，这不算它的特点。

它只有一条主张，其余都是这条的后果：**模型负责判断，后端负责算准；算出来的状态直接改下一步。**

数据落在 `D:\RealData`——Real 没有服务器，除了你自己配的模型接口，没有第二处会收到你文件夹里的内容。

### 它和同类工具差在哪

四件事，都能在运行记录里查到，不是宣传口径。

**① 下一步由账决定，不由叙述决定**

改没改看磁盘（比 mtime 与内容摘要），跑完没看退出码，有没有进展看账：改成功是实体进展，首读新文件、换一条新命令是探索进展，复读旧文件、重跑同一条不算。

这三个数不是拿去写总结的——它们直接决定：这一轮要不要触发验证、要不要在对账里点名、下面该唤醒、该收窄，还是该停机。

**② 判不出来的时候有出口，不用「没改」或「成功」糊过去**

- 扫盘超过 20,000 个条目 → 返回空，这一轮就没有改动清单，也不编一份出来
- 写进去了但读不回来（文件被占用、读回内容对不上）→ 直说「已写入、未回滚，请你自己 `read` 确认实际内容」
- 找不到能跑的解释器（Python 装在别处、PATH 上是个空壳）→ 语法这步跳过，不拦也不回滚；宁可不校验，也不拿「解释器没跑起来」当你的语法错

**③ 错误是一份契约，不是一句 `failed`**

位置（哪个文件、第几行）、原因（违反哪条约束）、怎么改（下一步该写成什么样）。

`find` 没命中，它把文件里最接近的段落贴出来，并给两条路：逐字构造（含空白、缩进、行尾），或改用 `line` 模式指定行号。行号越界、路径层级不对、文件被占用，同样带着原因和下一步。

能由后端算的就不留给模型试：行号在不在、`find` 命中几次、枚举值写歪了按语义归一、参数边界写进工具描述。只丢一个 `failed` 是甩手——模型只能猜，然后多调一次 API 再错一次。**省下的不是字数，是往返次数。**

**④ 卡住就退，但给你留断点**

按进展记账，不按时间：连续 4 轮零进展 → 注入停滞唤醒（必须换策略，或者来问你）；8 轮 → 只读收窄，只能交收束总结或提问；12 轮 → 收束停机。

停机不是封死：回一句「继续」，带断点和已有发现接着跑。一次都没改过文件的任务按「只诊断」看待，30 轮才收口——长任务的开头不该被误杀。

### 下载哪个

| 版 | 体积 | 拿到手是什么 |
|---|---|---|
| **安装版** | 12MB | 双击安装，装完有开始菜单和卸载项；系统缺 WebView2 时在安装阶段联网补一次 |
| **绿色版** | 11MB | 解压双击 `Real.exe`，不写注册表，能放 U 盘带走 |

文件名是 `Real_x.y.z_x64_en-US.msi` 和 `Real-x.y.z-win-x64.zip`，`x.y.z` 是版本号（当前 **v0.1.2**，以顶部徽章为准）。
每个包旁边有同名 `.sha256`，下完可以核。

命令行装绿色版，一行：

```
scoop install https://raw.githubusercontent.com/Azzrenz/Real/main/scoop/real.json
```

安装版也能用 winget：清单已提交 `microsoft/winget-pkgs`，审核中。

### 第一次打开

双击后有几秒没界面——后端在同一个进程里建数据库、起本地服务，之后就秒开。

**没有 API Key 也能进。** 内置一个模拟模型：界面、流程、工具调用都走得通，但回复是假的，不会真动你的文件。要它真干活，去「设置 → 模型与凭据」填一个 Key，OpenAI 兼容接口都行。

<!-- 待录：45–60 秒演示。录好后取消下面这行注释，并删掉本行提示。
![一句话跑完一个任务](screenshots/00-demo.gif)
-->

> **项目还在开发中**，功能还在往上加，接口和界面可能调整。当前版本可以正常使用；
> 想要稳定版请从 [Releases](https://github.com/Azzrenz/Real/releases) 下载，跟着 `main` 走则是开发中版本。


后端 Rust（Axum + SQLx + SQLite），桌面壳 Tauri 2 + React 18。
**全部状态落库、每轮可追溯、每次改动有对照。**

MIT License · 测试全绿 · Windows 完整验证

> **平台现状**：后端与桌面壳在 Windows 上完整验证。代码含 `cfg(unix)` / `cfg(target_os = "macos")`
> 分支，但命令通道（cmd / PowerShell 分派、`dir` / `findstr` / `Select-Object` / `.bat`）
> 是 Windows 语义，其测试以 `#![cfg(windows)]` 门住。macOS / Linux 上可编译，属**未验证**状态 ——
> 我们选择如实标注，而不是声称一个没跑过的支持矩阵。

> **English readers** → [README.en.md](docs/README.en.md) · [INSTALL.en.md](docs/INSTALL.en.md)
> （本中文版是权威文档，含完整机制与架构说明；英文版是面向国际读者的概览）

---

## 目录

- [这是什么](#这是什么)
- [它和同类工具差在哪](#它和同类工具差在哪)
- [下载哪个](#下载哪个)
- [第一次打开](#第一次打开)
- [我为什么做它](#我为什么做它)
- [三条设计取向](#三条设计取向)
- [核心机制](#核心机制)
- [架构](#架构)
- [快速开始](#快速开始)
- [HTTP 接口](#http-接口)
- [数据落点](#数据落点)
- [测试](#测试)
- [参与贡献](#参与贡献)
- [还没做完的部分](#还没做完的部分)
- [关于我](#关于我)
- [许可证](#许可证)

---

## 我为什么做它

我是做音乐的 —— 编曲、制作。

写 Real 的念头来自一件很具体的事：我想给编曲加一个能听懂人话的 AI 助手。
我说「给这段吉他加一段鼓」，它就加；我说「照这个旋律补一条贝斯」，它就补。
说得出，就做得到。

为这件事我开始写一个音乐工作站。写着写着停下来：
我真正缺的不是一个更聪明的助手，而是一个**自己的、可控的、不裸奔的** Agent 平台 ——
它得在我自己的机器上跑，读我自己的文件，每一步都留得下记录——谁改的、改了什么、什么时候改的。

2026 年 5 月底，我开始搭 Real。四个月后，它长成了现在这样。

**这六万六千行代码（不含测试），我一个字都没有手写。**

我不是程序员，用的是另一套东西：边界划在哪、谁归谁管、什么该留下、什么该删掉。
Real 是我把这种想法变成能跑起来的东西的方式。

所以 Real 对我不是终点，是手段 —— 我要做的是那个音乐软件，
而它需要一个安全、自主、能长期演化的开发底座。**先造工具，再造产品。**

这也是它开源的原因。这类东西不是"写完就完、一年打个补丁"的软件，
它是跟人打交道的，只不过这个"人"是模型：有幻觉、有不确定性、还在演化 ——
**这是个长期工程。** 一个人做不完，多一双眼睛就少一个坑。

---

## 三条设计取向

这三条决定了 Real 的样子。改任何一处之前，请先理解它们。

### 一、工具面只留四件套

```
read · write · modify · run
```

工具面是**每次请求的最前段**，是前缀里最贵的一块。工具越多，模型每轮还要多做一次
「该用哪个」的选择。所以按**能力**收敛到四条：读写改 + 万能命令通道。

被裁掉的 `search` / `ask` / `web_search` **实现全部保留**，只是不挂进工具面：

| 裁掉 | 能力去哪 |
|---|---|
| `search` | 并入 `run`（`grep -rn` 走原生壳） |
| `ask` | 由正文提问承担（不确定就问一句，不用选项菜单） |
| `web_search` | 并入 `run` + `curl` |

> 加回任何一个都必须同步 `mcp/registry_tests.rs` 的工具名单断言 —— 那是工具面的**对外契约**，不是注释。

### 二、话术当代码管（编制化）

提示词不散落在各处，而是**按部门编制**，每个文件一个职责、一个注入点、一个时机。
全部走 `include_str!` 编译期嵌入 —— 改 `.md` 不重编译，改动不生效。

| 文件 | 段 | 管什么 | 时机 |
|---|---|---|---|
| `workflow/system.md` | `#persona` | 人格与执行纪律 | 每轮常驻·最前 |
| `workflow/output_discipline.md` | `#output_discipline` | 语言纪律、思考骨架与思考纪律 | 每轮常驻 |
| `workflow/narration.md` | `#narration` | 旁白形态 | 每轮常驻 |
| `workflow/journal.md` | `#journal` | 留痕纪律 | 每轮常驻 |
| `workflow/round_frame.md` | `#frame` · `#memory` | 通道与预算 + 目标槽位 · 记忆包装框 | 每任务一次 |
| `workflow/mirror.md` | `#action` `#progress` `#attribution` `#terminate` | 触发式对账与归因 | 触发式 |
| `tools/{read,write,modify,run}.md` | — | 在册的四个工具**怎么填参** | 工具 schema |
| `tools/reason_instruction.md` | — | 工具行标题（`reason` 字段）的教学文案 | 注入进每个工具 |
| `tools/spill_note.md` | — | 超长输出落盘规则（多工具共用） | 随工具 |
| `reason` 字段 schema | — | 参数边界——**声明了 `min`/`max` 的必须写进 description** | 每个工具 |

排在前四的 `persona → output_discipline → narration → journal` 拼成**固定系统前缀**
（`agent/context.rs::CONVERGENT_SYSTEM_PROMPT`）—— 顺序即缓存，永不重排。

配套四条铁律（原文见 `server/prompts/README.md` §二）：

1. **改话术不改逻辑，改逻辑不改话术。** 行为调优去 `.md`，执行链路去 `.rs`。两边一起动必出事。
2. **话术变更视同代码变更。** 改完必须 `cargo check` + 提交 —— `include_str!` 是编译期嵌入。
3. **前缀缓存三律**：静态模板在前、动态值在后；条款顺序一旦稳定**永不重排**；同场景措辞零变体。
4. **每条契约一个职责，且只住一个文件。** 同一条纪律写两处 = 模型读两遍，既费 token 又互相稀释。

### 三、工作知识按需注入

「怎么写这个工具」（参数、返回值、边界）住 `tools/`，进固定前缀 —— 每次请求都要发。

「怎么干活」（检索策略、判结束信号、改动次序、编码坑）住 `specs/rules/`，
**按关键词命中才注入**，预算 2400 字符。现有 19 条规范，一个触发词都没命中的任务
由 `BASELINE` 兜底注入 `SP-SHELL` + `SP-LOCATE`。

> 判据：**搬家本身不省**（固定前缀与命中槽都是「每轮都发」）。
> 省的是「没命中就不注入」。所以 `specs/` 只放**窄触发**的知识 ——
> 几乎每个任务都要用的（如工具通道选择）应住 persona 常驻槽。
> **真正白赚的只有两类：删重复、删模型已知。**

---

## 核心机制

一次真实任务进行中的样子 —— 用户提问、模型旁白、工具调用，以及被注入的工作规范：


### 实现上的几处取舍

给读代码的人：六处核心做法，都能在 `server/src` 里逐条对照。

**扫盘：白名单 + 显式栈 + 访问上限，「判不出」在类型上就是 `None`**  
`wt.rs` 只认源码/配置/文档后缀，跳过 `node_modules`、`target`、`.git` 这类目录；用显式栈迭代而不是递归，深目录不爆栈；
访问计数一过 20,000 直接 `return None`。调用方拿到 `None`，就只能说「扫不出」，没有第二条路能编出一份清单。
命中按 mtime 倒序取前 12 条，路径转成相对工作区的 POSIX 形式再喂给模型。

**止损：判据是纯标量函数，账本只负责摊平**  
`stall_reason_of(stall_count, rounds, last_edit_round, no_files, 三个阈值)`——全用标量入参、阈值显式传入。
`Ledger` 有 30 多个字段，整体构造成本高，所以判据本体与状态解耦，可以穷举单测。
两条判据互补：一条抓「原地打转」（复读旧文件、重跑同一条命令让 `stall_count` 涨）；
一条抓「一直在读新东西却迟迟不动手」（这类任务 `stall_count` 恒为 0，前一条永远抓不到，会一路烧到轮次上限）。

还有一条历史：早期另有一档「零起点空转」，上线后全量日志 grep 零命中——首轮就被「首读新文件」置位，条件恒假。
撤掉，不留死代码。

**进展怎么算：靠账，不靠模型自述**  
阅读账记「文件 → 首读轮次 / 读取次数」，重复读会被当面点名：这是第几次读、首读在第几轮、内容还在上下文里；
`run` 用参数签名去重，同签名重跑不算探索。于是「这一轮有没有进展」是一条可复算的账，不是模型的自我评价。

**前缀即缓存：话术编译期嵌入，顺序固定**  
`prompts/*.md` 通过 `include_str!` 在编译期拼进二进制，按 `<!-- #persona -->` 这类 marker 切片，
再按固定顺序拼成系统前缀（静态在前、动态在后）。代价是改 `.md` 必须重编译才生效；
换来的是每次请求前缀字节一致、缓存稳定命中。所以在这套工程里，「改话术」在流程上等同改代码。

**写盘是一笔小事务**  
备份 → 写入 → 读回逐字节比对 → Python 再过一遍语法；任何一步不过，用备份盖回，并在错误里写明「已回滚到哪个文件」。
不留半成品，也不把回滚说成成功。

**记账：逐笔入账，事后不重算**  
每次调用按**当时**的模型与峰谷倍率算一次，累加进总额；input / output / cached 三项分开记，展示时舍到分。
事后调价不会篡改历史账——这也是成本数字能被信的原因。

### 收敛循环

任务不是「问一句答一句」，而是**一轮一轮收敛**。每轮只做一件事：
把当前上下文交给模型表态，再按它的表态分相处理。

```
一轮 = 决策相（上下文 → 模型表态，≤1 次 LLM 调用）
         ├─ 有工具调用 → 执行相：执行 + 回执进流 → 进入下一轮
         └─ 纯文本回答 → 收敛相：这就是答案，循环结束
```

循环在四种情况下停：模型自己收敛（纯文本）、达到轮次上限（默认 100，`REAL_MAX_ROUNDS`）、
用户取消、或撞上停滞止损。轮次上限属**非收敛终止** —— 后端照实记成未完成，
不判定工作质量。

### 停滞止损

后端按**进展**记账：改文件成功 = 实体进展；首读新文件 / 换新命令成功 = 探索进展；
复读旧文件与重跑同命令**不算进展**。连续零进展按档位升级：

| 档位 | 默认 | 动作 |
|---|---|---|
| 唤醒 | 4 轮 | 注入「停滞唤醒」——必须换策略或向用户提问 |
| 收窄 | 8 轮 | 进入只读收窄——只能交收束总结或提问 |
| 停机 | 12 轮 | 收束停机交付；用户回「继续」即带断点续跑 |
| 纯诊断兜底 | 30 轮 | 全程一个文件都没改过 ⇒ 按"只诊断"看待，到时收口 |
| 总门限 | 20 轮 | **轮数不足 20 不判停机** —— 长任务的开头不该被误杀 |

停机只对「曾改过文件」的任务生效；一次都没改过的任务走纯诊断兜底那条。
参数可由环境变量 `REAL_STALL_*` 覆盖（`WARN` / `NARROW` / `STOP` / `DRY` / `MIN`）。

### 「改了没有」不由模型自己申报

两层都写死，缺一层就有空子。

**第一层 · 后端自己扫**（`server/src/wt.rs`）—— 模型说「改好了」不算数。
每轮扫一遍工作区，按 mtime 找出**自某道标记以来被写过的源文件**，新→旧最多列 12 条。
这个清单不由模型计算，也不由模型解释。它喂三处：

| 用途 | 干什么 |
|---|---|
| 触发自动验证 | 这一轮有写入 ⇒ 该跑一次验证（`tools/verify.rs::plan_for`） |
| 卡壳反射 | 轮数 ≥ 8 且**本轮无任何写入证据** ⇒ 注入对账事实（只反射，不判定、不强停） |
| 改动清单 | 并入给模型看的「这轮改了哪些文件」（含经 `run` / 脚本改的） |

扫描有硬顶：单次最多访问 20,000 个条目，超限判「扫不出」而不是硬扛；
判不出时该会话标记为不可用并跳过后续扫描 —— **不假装有数据**。

**第二层 · 人格里的自证纪律**（`prompts/workflow/system.md` §外部裁判）—— 要求模型每轮跑一次
`git status --porcelain` 与 `git diff --stat`，把**实数原样**贴进回复；**连续两轮该数字为空
⇒ 判定为亏**，无论叙事多漂亮。注意这条是**写给模型的**（提示词层），不是后端在跑 git。

### 上下文与成本治理

- **热 / 冷闸门**：热会话一字不改；冷会话或越线才压缩。
- **读缓存**：首次读落的「指纹 + 符号骨架」存入记忆，跨任务指纹一致时定向提示，变更自动刷新。
- **spill 归档**：超长工具输出落盘归档、上下文只留引用；真会话保留 7 天、孤儿 1 天。
- **会话退场**：静默满阈值的会话自动摘要或清除，由后台定时器续跑。

### 证据链

| 表 / 目录 | 存什么 |
|---|---|
| `sessions` / `events` | 每轮事件流（SSE 实时推送 + 落库回放） |
| `turn_logs` | 每轮计量与结果 |
| `memories` | 长期记忆、工作日志、读缓存骨架 |
| `scheduled_jobs` | 定时触发 |
| spill 目录 | 超长工具输出的全文归挡 |

---

## 架构

```
Real/
├── server/                    Rust 后端
│   ├── src/
│   │   ├── agent/             收敛循环 · 上下文装配 · 历史治理 · 记忆
│   │   │   ├── orchestration/   workflow.rs —— 收敛循环与上下文装配
│   │   │   ├── history/         窗口 · 治理 · 归档 · 退场
│   │   │   ├── memory/          记忆 · 工作日志 · 项目识别 · 主题
│   │   │   ├── execution/       参数准备
│   │   │   └── planning/        规划域（现只剩占位符残留检测）
│   │   ├── tools/             四件套实现 + 契约 + 校验
│   │   ├── mcp/               MCP 客户端 · 工具注册表 · spill 归档
│   │   ├── model/             LLM 客户端 · 模型目录
│   │   ├── routes/            HTTP 接口
│   │   ├── db/                连接池 · 仓储
│   │   ├── config/            配置 · 运行时设置 · 调优原子
│   │   ├── sse/               事件流
│   │   ├── scheduler/         定时触发
│   │   ├── path/              路径 · 数据根
│   │   ├── backbone/          脏活层：文件索引 · 信息抽取
│   │   ├── domains/           域插件注册表（U 盘式接入；当前未注册任何域）
│   │   ├── confirm.rs         确认门 —— 危险操作的判据
│   │   ├── wt.rs              工作区改动扫描（mtime）
│   │   ├── facts.rs           事实判定
│   │   └── cost.rs · pricing.rs   计量与计价
│   ├── prompts/               话术（`include_str!` 编译期嵌入）
│   ├── specs/                 按需注入的工作规范
│   ├── migrations/            SQLite 迁移
│   └── .env.example           配置模板
└── client/                    Tauri 2 桌面壳 + React 18
    ├── src/
    │   ├── app/               应用外壳与装配
    │   ├── features/          chat · sessions · settings · mcp · memory · schedule · tools · thinking
    │   ├── services/          与后端契约的单一入口
    │   ├── shared/            跨功能复用
    │   └── styles/            设计 token
    └── src-tauri/             Rust 外壳
```

> **部门化是硬规矩**：后端、桌面壳、业务各归各部门，跨部门代码禁止混放。
> 详见 [CONTRIBUTING.md](docs/CONTRIBUTING.md#二代码组织按部门归属禁止混放)。

---

## 快速开始

### 依赖

| | 版本 | 必需 |
|---|---|---|
| Rust | 1.75+ | 是（[rustup.rs](https://rustup.rs/) 默认安装即可） |
| Node.js | 18+ | 是（[nodejs.org](https://nodejs.org/) 的 LTS 版） |
| Python | 3.9+ | 否（语法与导入校验会用到；未装时自动降级） |
| Tauri 2 系统依赖 | — | 仅桌面壳需要，见 [官方前置条件](https://tauri.app/start/prerequisites/) |

### 方式零：一条命令（Windows 包管理器）

```powershell
# 绿色版：解压即用，不写注册表、不动系统目录
scoop install https://raw.githubusercontent.com/Azzrenz/Real/main/scoop/real.json

# 安装版：清单已提交 microsoft/winget-pkgs，审核通过后可直接
winget install Azzrenz.Real
```

> **为什么不是 `npx`**：`npx` 那种「一行命令直接跑」的前提是源码本身可执行（Node 项目）。
> Real 是 Rust + Tauri 的桌面程序，用户拿到必须是编译好的二进制——所以等价物是包管理器：
> 一条命令装好，双击即用。两条清单由 `scripts/release/release.py` 在每次发版时自动生成。

### 方式一：Windows 安装包（零依赖）

从 [Releases](https://github.com/Azzrenz/Real/releases/latest) 页下载 `.msi` 安装即可 —— **不需要装 Node 或 Rust**。

安装包里已经带好：

- 后端程序（作为 sidecar 与主程序同级，启动时由桌面壳自动拉起）
- WebView2 运行时（离线安装包，Windows 10 老版本也能装）

> 首次启动窗口可能要等几秒：后端要建库、起服务。
> 首次打开先配置 API Key（Settings → Model & credentials），否则发任务会提示未配置。

### 方式二：一键脚本（Windows，开发者）

下载仓库后**双击 `build-run.bat`**。脚本按序做五件事：

1. 检查工具链 —— 缺 Node 或 Rust 会给出下载地址并停下，不会假装在跑
2. 首次运行自动从 `.env.example` 生成 `server\.env`
3. 首次运行自动 `npm install`
4. 编译后端
5. 起后端（`127.0.0.1:8943`）→ 起桌面壳

> **首次运行要等几分钟。** cargo 先编译整个后端，Tauri 再编译桌面壳 ——
> 那是编译器在干活，不是卡死，让它跑完。

> **API Key 必填。** 在 `server/.env` 填 `DEEPSEEK_API_KEY`（或智谱的 Key），
> 也可以在应用的 Settings 里填。没有 Key 时任务会明确报「未配置模型」，不会假装在干活。

日志落在数据根的 `logs/` 下（Windows 默认 `D:\RealData\logs`；机器上没有 D 盘时回落 `%APPDATA%\real-agent\logs`）。
后端起不来时脚本会把日志尾部直接打在屏幕上。

### 方式三：手动

```bash
# 1. 配置
cp server/.env.example server/.env
#    编辑 server/.env，填入 DEEPSEEK_API_KEY
#    支持 DeepSeek 或智谱 GLM，二选一；Key 在对应平台免费注册申请

# 2. 起后端（终端 A）
cd server
cargo build
./target/debug/real-server          # 监听 127.0.0.1:8943

# 3. 起桌面壳（终端 B）
cd client
npm install
npm run tauri dev                   # Vite dev server 在 8618
```

> 后端只监听回环地址。**不要把它暴露到公网** —— 它没有认证层，设计前提是单用户本地运行。
> 详见 [SECURITY.md](docs/SECURITY.md)。

### 验证起来了

```bash
curl http://127.0.0.1:8943/health   # 存活
curl http://127.0.0.1:8943/ready    # 就绪（DB 已连、工具面已注册）
```

### 设置面板

**没有 API Key 也能用** —— 不填时自动走内置模拟模型，全流程可走通。要接真实模型就在这里填：

![设置 · 模型与凭据](screenshots/03-settings-model.png)

它的记忆不是黑盒，是面板里一张能改的表：长期记忆、用户画像、加权主题都列在那儿。**同一个话题再聊一次加 10 分，到 90 分定成永久、永不淘汰；攒满 20 条以后，分最低最旧的先被挤出去。**

![设置 · 记忆与上下文](screenshots/04-settings-memory.png)

### 接入你自己的模型

**内置两家**：DeepSeek 和智谱 GLM。设置面板底部的模型芯片可以一键互切 —— 后端会同时换端点和 Key。

要接**第三家**（任何 OpenAI 兼容服务或自建代理都行）——**不用改代码**：

1. 往 `server/assets/providers/` 放一个 `<厂商id>.json`
2. 重启 Real（启动日志会打印「已写入厂商档案」）
3. 在设置面板给这家填**一次** API Key

那个目录就是「模型公司注册表」，**一家公司一个 JSON 文件**，OpenAI 兼容的厂商直接照抄模板即可。

**完整字段说明 + 模板 + 两个容易踩的坑** 见：

→ [`server/assets/providers/接入新公司-读我.md`](server/assets/providers/接入新公司-读我.md)

> 最容易踩的一条先放这儿：**`id` 和 `label` 是两回事，都要填。**
> `id` 是发给厂商的 `model` 参数（必须与官方文档一字不差，写错立刻 400）；
> `label` 是界面上给人看的版本名（不填就只能看到 API 名，你看不出花的钱买的是哪个版本）。

---

## HTTP 接口

后端是纯 API —— 桌面壳只是它的一个客户端。

| 域 | 接口 |
|---|---|
| 健康 | `GET /health` · `GET /ready` |
| 会话 | `GET/POST /api/sessions` · `GET/PATCH/DELETE /api/sessions/{id}` · `POST /api/sessions/{id}/chat` · `POST …/cancel` · `POST …/confirm` |
| 插话 | `POST /api/sessions/{id}/interject` · `GET /api/sessions/{id}/interjections` |
| 事件 | **`GET /api/sessions/{id}/events`（SSE 实时流；`?poll=true` 退化为增量拉取）** · `GET …/events/before` · `GET …/thinking` |
| 设置 | `GET/PUT /api/settings` · `POST /api/settings/test` |
| 记忆 | `GET/PUT /api/memory/persona` · `GET/POST/DELETE /api/memory/preferences` · `GET /api/memory/themes` · `POST /api/memory/themes/pin` · `DELETE /api/memory/themes/{id}` · `GET /api/memory/digests` · `GET /api/memory/stats` · `GET /api/memory/files`（项目的记忆文件清单） |
| MCP | `GET/PUT /api/mcp/servers` · `POST /api/mcp/servers/apply` · `POST /api/mcp/servers/{name}/reconnect` |
| 技能 | `GET/POST /api/skills` · `DELETE /api/skills/{name}` · `POST …/rename` · `POST …/open` · `GET …/raw` · `POST /api/skills/absorb` |
| 定时 | `GET/POST /api/schedules` · `DELETE /api/schedules/{id}` · `POST /api/schedules/{id}/toggle` · `POST /api/schedules/{id}/run` |
| 其他 | `POST /api/feedback` · `POST /api/reveal` · `POST /api/self-improve/events` · `POST /api/self-improve/status` |

完整定义见 `server/src/routes/mod.rs`。

---

## 数据落点

运行期数据**不在仓库里**。数据根按优先级解析：

```
REAL_DATA_DIR 环境变量  >  设置面板选定的目录  >  D:\RealData（出厂默认）  >  %APPDATA%/real-agent（无 D 盘时回落）  >  项目根 .real/
```

默认落 **D 盘**（`D:\RealData`）：`db/`（SQLite）、`logs/`、`spill/`、`tmp/`、`skills/`、`providers/`、`attachments/`、`archive/`。

- **换盘/换位置**：设置 →「数据目录」，面板直接给出可用盘符（选 `E:` 即落 `E:\RealData`），也可手填任意绝对路径。改后**重启生效**。
- **首次启动自动迁移**：新位置还没有库、而旧位置（C 盘）有数据时，启动会把旧目录整目录复制到新位置（连 `-wal`/`-shm` 一起），**源目录保留**，想回滚改回 C 盘即可。
- 为什么不默认 C 盘：`%APPDATA%` 会随重装系统一起消失，重装后数据和设置一起清零。

`.gitignore` 已按「人写的源码进仓库 / 机器生成的产物与私有数据不进仓库」这一条判据覆盖全部分类。

---

## 测试

```bash
cd server && cargo test                 # 全部通过（另有 3 项 #[ignore]，需 --ignored 才跑）
cd client && npm run typecheck && npm run build
```

提交前请确认 `cargo check --all-targets` **零警告**。

---

## 参与贡献

欢迎。开工前请读 [CONTRIBUTING.md](docs/CONTRIBUTING.md)，其中最关键的一条是：

> **动手前先回答三个问题**：这个改动归哪个部门？现有部门能否承载？
> 这是架构缺口还是实现 bug？

渠道分两类，别走错：

- **Bug、功能建议、使用提问** → 直接 [开 Issue](https://github.com/Azzrenz/Real/issues)。
- **安全问题** → **不要**开公开 Issue，按 [SECURITY.md](docs/SECURITY.md) 走 GitHub 私密上报通道。

---

## 还没做完的部分

有件事得先说清楚：**设置面板里有一部分功能还没有完全对接上。**

不是设计上的取舍，是我确实没有精力继续往下做了，就先摆在那儿。
后面有时间我会接着完善；**也欢迎你基于这个版本自己往下做**，不用等我。

---

## 关于我

音乐制作人。不写代码，用架构的方式想问题。

Real 从 2026 年 5 月底开始做，为的是给另一件事（一个音乐软件）
造一个自己的、可控的开发底座。

GitHub：[@Azzrenz](https://github.com/Azzrenz)

---

## 许可证

[MIT](LICENSE) © 2026 RealBody
