# Real

<p>
  <a href="https://github.com/Azzrenz/Real/releases/latest"><img alt="Release" src="https://img.shields.io/github/v/release/Azzrenz/Real"></a>
  <a href="LICENSE"><img alt="License" src="https://img.shields.io/badge/license-MIT-blue"></a>
  <img alt="Platform" src="https://img.shields.io/badge/platform-Windows%20%C2%B7%20verified-lightgrey">
  <img alt="Stack" src="https://img.shields.io/badge/stack-Rust%20%2B%20Tauri%202-orange">
</p>

**能交给模型的都交给模型，能算准的都交给后端。**

给它一个文件夹、一句话，它在里面读、改、跑；做完交回三件事——**动了哪些文件、花了多少、跑没跑完**。
这三件事不由它口述：扫磁盘、读 usage 原始数字、看进程退出码。

### 这是什么

Real 是一个跑在 Windows 上的本地 AI 工作台（开源，MIT）。

- **工作区是一个文件夹。** 你指定一个目录，它只在这里动手；要往目录外面写，会先问你。
- **任务是一句人话。**「把 `D:\素材` 里的照片按文件名里的日期分到月份文件夹」——不用写脚本，不用搭工作流。
- **状态全在本机。** Real 没有服务器，也没有云端账号：会话、工具回执、改动记录、成本账都落在 `D:\RealData`。除了你自己配的模型接口，没有第二处会收到你文件夹里的内容；卸载就是删那个文件夹。

它不做的事：不多租户、不替你决定在哪干活；要写到工作区外面，会先问你。

### 能干什么

三件真事，每件都当场能验：

**整理文件**

> 「把 `D:\素材` 里的照片按文件名里的日期分到月份文件夹，没有日期的放到『未分类』」

它扫一遍文件名取日期，建目录、移动、重名自动加后缀。做完给你一张清单：哪个文件从哪挪到哪——你打开文件夹就能对上。

**在一个真项目里改代码**

> 「把登录接口的超时改成 10 秒，顺手把相关测试跑一遍」

先读再改，改完真的去调编译器和测试。不过，就把报错带回下一轮接着改；过了，才说改完。

**跑长命令**

> 「编译整个项目，出结果告诉我」

编译、安装、批量转码按进程退出码判定：跑 40 分钟也等它自己退出，不按秒数猜，也不到点就杀。

### 下载哪个

| 包 | 体积 | 说明 |
|---|---|---|
| `Real_x.y.z_x64_en-US.msi` | 12MB | 双击安装，装完有开始菜单和卸载项；系统缺 WebView2 时在安装阶段联网补一次 |
| `Real-x.y.z-win-x64.zip` | 11MB | 解压双击 `Real.exe`，不写注册表，能放 U 盘带走 |

`x.y.z` 是版本号，当前 **v0.1.2**（以顶部徽章为准）。每个包旁边有同名 `.sha256`，下完可以核。

命令行装绿色版，一行：

```
scoop install https://raw.githubusercontent.com/Azzrenz/Real/main/scoop/real.json
```

安装版也能用 winget：清单已提交 `microsoft/winget-pkgs`，审核中。

### 第一次打开

双击后有几秒没界面——后端在同一个进程里建数据库、起本地服务，之后就秒开。

**没有 API Key 也能进。** 内置一个模拟模型：界面、流程、工具调用都走得通，但回复是假的，不会真动你的文件。要它真干活，去「设置 → 模型与凭据」填一个 Key，OpenAI 兼容接口都行。

### 模型管判断，后端管事实

模型负责判断：读不读、改哪里、下一步做什么、什么时候收尾。
后端负责事实：路径存不存在、命令退出码是多少、写进去的字节对不对。

两边都不越界——不要求模型「保证自己没错」，后端也不替模型做判断。
所以换更强的模型，它跟着变强，不用改代码；模型记错了自己干过什么，也不会照着错下去——那部分数字不是模型给的。

| 模型说 | 后端核 |
|---|---|
| 「改好了」 | 扫磁盘比对 mtime 与内容摘要，改动清单不由模型生成 |
| 「跑完了」 | 看进程退出码；预算到了也不杀，等它自己退出 |
| 「我判断不了」 | 如实记「判不出」，不填一个「没改」 |
| 「花了这些」 | 读 usage 原始数字，按未命中 / 缓存 / 输出分项记账 |

### 下一步不是猜的，是判出来的

扫盘、看退出码、跑编译，别的 Agent 也做——这些不稀奇。区别在这些结果去哪了：
写进报告，那是事后说明；**进了下一轮的输入，才叫判据**。

Real 的每一步不看模型怎么描述当前状态，先由后端算一遍，再据此决定下一步：

| 判断什么 | 谁算 | 算完决定什么 |
|---|---|---|
| 这一轮到底改了文件没有 | 扫工作区，比 mtime 与内容摘要 | 决定要不要跑验证、改动清单写哪几条、要不要在对账里点名 |
| 命令跑完了没有 | 进程退出码 | 决定这轮算完、还是把 stderr 带回下一轮接着改 |
| 这轮有没有实质进展 | 实体进展（真改了）/ 探索进展（读到新东西）分开记账 | 决定下一步是继续、先唤醒换策略、只读收窄，还是收束停机 |
| 这个手段还准不准 | 有效 / 过期 / 未知三态 | 决定继续用，还是停用该项并如实记「判不出」 |
| 这一轮花了多少 | 模型返回的 usage 原始数字 | 决定还能不能继续、预算怎么给 |

一句话：**状态由后端算出，动作由状态决定。** 模型只负责它还擅长的那部分——读什么、改哪里、下一步试什么。

这些判据都是可量的：缓存命中 **94–98%**，单任务 **¥0.06–0.09**；
同一类任务第二次跑，轮次 **30 → 16**，成本 **-33%**。

### 错误是契约，不是一句话

失败返回三样：**位置**（哪个文件哪一行）、**原因**（违反哪条约束）、**怎么改**。

只把错误丢回去，模型只能猜；给了修法，它一次就能改对。
「写前先验、写完核对、不一致回滚」都是这份契约的执行细节，不是另外三件事。

### 判不出，就说判不出

扫盘超过上限、解释器找不到、字节比对不了——这类情况如实记「判不出」，
不填一个「没改」，也不当成功交付。三态（有效 / 过期 / 未知）就是为了让「不知道」有个正经的位置。

<!-- 待录：45–60 秒演示。录好后取消下面这行注释，并删掉本行提示。
![一句话跑完一个任务](screenshots/00-demo.gif)
-->

> **项目还在开发中**，功能还在往上加，接口和界面可能调整。当前版本可以正常使用；
> 想要稳定版请从 [Releases](https://github.com/Azzrenz/Real/releases) 下载，跟着 `main` 走则是开发中版本。


```
决策 → 执行 → 回执进流 → 再决策 …… 纯文本即收敛交付
```

模型在这里不是「聊天」，而是在一个**一轮一轮收敛**的循环里推进任务：
每轮把当前上下文交给模型表态 —— 它发工具调用就执行、把回执带进下一轮；
它给出纯文本，那就是答案，循环结束。每一轮都留证据、每一轮都可追溯。

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
- [能干什么](#能干什么)
- [下载哪个](#下载哪个)
- [第一次打开](#第一次打开)
- [模型管判断，后端管事实](#模型管判断后端管事实)
- [下一步不是猜的，是判出来的](#下一步不是猜的，是判出来的)
- [错误是契约，不是一句话](#错误是契约不是一句话)
- [判不出，就说判不出](#判不出就说判不出)
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
