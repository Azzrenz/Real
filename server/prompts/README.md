# 话术部门编制与治理规范

> Real 话术（prompt）部门的组织章程。改话术前**必读**。
> 话术是后端资产：模型只说收到的词，喂什么词由这里的编制决定。
>
> 最后校订：2026-09-16（中国时区 UTC+8）——工具面收四件套、四份描述瘦身、新增 SP-EDITSAFE

## 一、编制表

界面上要给人看的东西只有两样，其余全是噪音：**旁白**（干活的思路与结论）与**命令执行的意图**（工具行标题那句话）。
参数、内部状态、推理过程、对账语言都不该进面板——推理留给思考块，参数留给工具详情。

| 文件 | 段 | 管什么 | 注入点 | 时机 |
|---|---|---|---|---|
| `workflow/system.md` | `#persona` | 人格与执行纪律：模式判定、语言与风格、自主边界、执行方式、执行纪律、两条底线 | `context.rs` | 每轮常驻·最前 |
| `workflow/narration.md` | `#narration` | 旁白形态：叙述纪律、对话态切换、输出形态、收敛即止、措辞 | `context.rs` | 每轮常驻 |
| `workflow/journal.md` | `#journal` | 留痕纪律：工作日志与长期记忆**怎么记**（写到哪由记忆块给，见 §五） | `context.rs` | 每轮常驻 |
| `workflow/round_frame.md` | `#frame` | 通道与预算：通道优先级、联网、参数预算、脚本库、语言提醒 + 目标槽位 | `workflow.rs` | 每任务一次 |
| `workflow/round_frame.md` | `#memory` | 记忆包装框 | `workflow.rs` | 每任务一次·有记忆才注入 |
| `workflow/mirror.md` | `#action` `#progress` `#attribution` `#terminate` | 触发式对账与归因 | `workflow.rs` | 触发式 |
| `../specs/rules/*.md` | — | **工作规范正文**（按需加载，见 §七） | `agent/specs.rs` | 命中的任务·每任务一次 |
| `../specs/summaries/*.md` | — | 规范一行摘要（超预算时随尾巴列出） | 同上 | 同上 |

### 工具契约面

> **工具面 = 四件套：`read` / `write` / `modify` / `run`**（2026-09-16 定案）。
> 注册在 `tools/mod.rs::builtins()`；**契约测试**是 `mcp/registry_tests.rs::tools_catalog_byte_stable_across_calls`
> 的名单断言——加回/裁掉任何工具都必须同步它。
> 裁掉的 `search` / `ask` / `web_search`（以及更早的 `list` / `web_fetch` / `proc_status` / `edit`）
> **实现全部保留**，只是不挂进工具面；加回须同步话术与测试。

| 位置 | 内容 | 规矩 |
|---|---|---|
| `tools/{run,read,write,modify}.md` | 四个**在册**工具 description | 走 `.rs` 流程（`include_str!` 编译期嵌入） |
| `tools/edit.md` | **已裁剪**的 `EditTool` description（不挂工具面，随 `fs_write.rs` 保留） | 同左 |
| `tools/reason_instruction.md` | 工具行标题（`reason` 字段）的教学文案 | 由 `mcp/registry.rs` 注入到每个工具 |
| `reason` 字段 schema | 参数边界 | **声明了 `minimum`/`maximum` 的参数，边界必须写进 `description`**——机器可读关键字模型不读；有机器检查 `bounded_params_state_bounds_in_description` |

**description 的长度纪律**：**一两句话 + 返回值一行**，只写「怎么写这个工具」。
判据是「这段字是否**每次请求**都要发」——`tools/*.md` 进常驻前缀，按未命中价付费；
「怎么干活」进了 `specs/`，只在命中时注入。

**机器守着三条**，改描述前先知道它们会红：
`every_tool_description_declares_return_shape`（须声明返回格式）、
`every_tool_description_declares_price_tag`（`read` 须含「大文件」+「自动」；`run` 须含 `Remove-Item` + `dir /b`）、
`bounded_params_state_bounds_in_description`（参数边界）。

> **职责边界**：
> `tools/*.md` 只写**「怎么写这个工具」**——参数、返回值、边界、拒绝形态。
> **「怎么干活」**——检索策略、成败判据、改动次序、脚本库、联网入口、输出控量——
> 一律进 `specs/`，**按关键词命中才注入**。判据见 §七。
>
> 反例（改前确实住在 `run.md` 里）：锚点降级路径、判结束三级信号、批量改动次序。
> 它们与「run 怎么填参」无关，占着固定前缀的 miss 价，却只在少数任务里用得上。
>
> **2026-09-16 一次性还清**（工具面收四件套时把四份描述按此规矩重写，迁出去向）：
>
> | 原住 | 内容 | 迁到 |
> |---|---|---|
> | `run.md` | 命令怎么串、删除通道、`del` 正斜杠禁令 | **SP-SHELL** `shell_dispatch.md` |
> | `run.md` | 后台与判结束（`tasklist /FI` 拿 pid 判完成） | **SP-VERDICT** `verdict_signals.md`（原本已完整覆盖） |
> | `read.md` | 「要改就一次读全 → 之后全用 line 模式」的次序 | **SP-EDITSAFE** `edit_safety.md`（新） |
> | `write.md` | write/modify 分派、`#En` 占位禁令、多行 `replace` 转义 | **SP-EDITSAFE** |
> | `write.md` | CRLF 字节保真 | **SP-WINENC** `win_encoding.md` |
> | `write.md` | HTML 外部依赖本地化 | **SP-VISUAL** `visual_artifact.md` |
> | `run.md` | 改前留底 | **SP-REFACTOR** `refactor_order.md`（原本已覆盖） |
> | `run.md` | 用例清单 | 删（模型不需要） |

### 注入拓扑

```
每轮常驻（system prompt）：  persona → narration → journal     ← 静态前缀，顺序永不重排
任务首轮（user 块）：        frame[ 语言 → 思考骨架 → 思考纪律 → 目标 + memory 段 + contract ]
   └ contract 槽 = 域话术（域注册表当前为空）+ specs 命中的工作规范（关键词触发，预算 2400 字符）
中途/异常（尾部追加）：      mirror: action / progress / attribution / terminate
工具 schema（每工具）：      tools/*.md（只含执行方式）+ reason_instruction
```

## 二、不变量（改话术的铁律）

1. **改话术不改逻辑，改逻辑不改话术**：行为调优去 `.md`，执行链路去 `.rs`。两边一起动必出事。
2. **话术变更视同代码变更**：改完必须 `cargo check` + 提交——`include_str!` 是编译期嵌入，改 md 不重编译 = 改动不生效。
3. **前缀缓存三律**：静态模板在前、动态值在后；条款顺序一旦稳定**永不重排**（重排 = 全量缓存失效）；同场景措辞零变体（禁时间戳/随机数/计数进模板正文，计数只进尾部注入的镜像块）。
4. **每条契约一个职责，且只住一个文件**：同一条纪律在 N 处出现 = 模型读 N 遍，既费 token 又互相稀释。

## 三、话术纪律

1. **数据块里只放事实与约束，不放"输出成什么样"**。要规定写成什么样 → 归 `narration.md`。
   （数据块里的形态纪律必然被模型实现成固定句式，且绕过本编制表。）
2. **常态纪律住常驻槽**（persona / narration / journal）；"每任务一次"的槽位只放该任务当下需要知道的。
   把常态纪律放进一次性槽位 = 大半个任务里模型根本没收到它。
3. **归口或精简话术前，先逐条列出被删内容并各自指定去处**，再动手删——否则"收敛"会变成"丢东西"。
4. **能被上游规则推导出来的，不单独写一条**；同一个东西只给**一个名字**。两处各写一版 = 制造两个版本的同一件事。
5. **关键条款必须有测试钉住**。断言绑**段名与内容**，不绑编号（编号会随重排漂移）；对账不写进测试 = 没对账。
6. **退役一个模块 = 六处一起收**：注册、说明文件、管道、挂载点、前端显示名、话术里的清单；
   最后加一条**"禁止回归"断言**，否则下次有人只看代码会觉得它"好像还该在"。
7. **边界只写在 schema 关键字里 = 没写**：给模型看的只有 `description`。两个读者（校验器 / 模型），写一个漏一个 = 模型只能猜。
8. **防呆要响，不要静默降级**：取不到内容就返回空 + `warn!`，别回落到"返回全文"——静默降级会把配置错误伪装成正常行为。

## 四、新增/修改话术的决策树

```
要写一条新话术，它管什么？
├─ 我是谁、怎么说话、什么底线        → system.md 的 persona 段
├─ 什么时候该直接动手、什么时候该问  → system.md 的 persona 段
├─ 用哪个工具（通道选择）            → system.md 的 persona 段
├─ 干完活怎么留痕、回头怎么查        → journal.md
├─ 旁白怎么写、写到哪停、收敛        → narration.md
├─ 某个工具怎么用（参数/返回/边界）  → tools/<工具>.md（走 .rs 流程）
├─ 干活的策略 / 环境知识（用到才需要）→ specs/rules/（按需注入，见 §七）
├─ 只在某轮该想起的"对账信息"        → mirror.md 的 action / progress
├─ 工具失败后"怎么理解"              → mirror.md 的 attribution
├─ 工具行标题那句话怎么写            → tools/reason_instruction.md
└─ 首块编排顺序变了                  → round_frame.md frame 段（⚠️ 动这里 = 全量缓存失效）
```

判断口诀：**"它是常态还是此刻？"** 常态（每轮都该遵守）→ persona / narration / journal；此刻（这一轮才该想起）→ mirror。
**第二问**：**"它管的是「怎么写这个工具」还是「怎么干活」？"** 前者 → `tools/`；后者 → `specs/`（按需）。
**第三问**：**"模型本来就知道吗？"** 知道（通用命令大全、语言语法）→ **不写**。

## 五、变量契约

- 模板变量一律 `{具名}`，**禁位置参数**；新增变量必须同步登记进本表的编制表。
- 变量值是动态内容——绝不把动态值拼进模板正文（§二·3）。
- 现有变量：`frame`（`{rounds}` `{goal}` `{memory}` `{contract}`）、`memory`（`{memory}`）、mirror 各段。
- `{contract}` 槽位**只放命中域的工作流话术**；基础纪律不在此处——它们已由 persona/narration/journal 每轮常驻，再灌一遍就是同一句话讲两遍。
- 块名即契约：注入块与话术里按名引用必须**逐字一致**（如记忆块的【工作日志与长期记忆】），由测试钉住。

## 六、修改流程

1. 在 §一 编制表里找到对应单位 → 2. 改 `.md` → 3. `cargo check` + 跑相关测试 → 4. 提交（写明"话术变更"与预期行为差异）。

## 七、工具契约 vs 工作规范：分家判据

**判据**：`run.md` 这类工具描述里，大半内容与「怎么填参」无关（检索策略、判结束信号、
批量改动次序）—— 它们占着固定前缀的 miss 价，却只在少数任务里用得上。

**判据（按序问，任一命中即按右侧处置）**：

| 问 | 命中 → 去哪 |
|---|---|
| ① 这是**填这个参数**必须知道的吗？（参数名、类型、边界、返回值怎么读） | 留 `tools/<工具>.md` |
| ② 这是**干活的方法**吗？（先探还是先干、怎么判结束、改动什么次序） | `specs/rules/`（按需） |
| ③ 这是**环境事实**吗？（壳分派、编码坑、路径写法） | `specs/rules/`（按需） |
| ④ 模型**本来就知道**吗？（通用命令大全、语言语法） | **不写** |

**⚠️ 搬家本身不省**：固定前缀与 `{contract}` 槽**都是「每轮都发」**，成本等价。
省的是**没命中就不注入**。故 `specs/` 只放**窄触发**的知识——几乎每个任务都要用的
（如工具通道选择）应住 persona 常驻槽，塞进 specs 反而失真。
**真正白赚的只有两类：删重复、删模型已知。**

**现有规范（16 条 · 2026-09-15 扩容）**：

| 类别 | id | 管什么 | 触发词示例 |
|---|---|---|---|
| 执行 | `SP-SHELL` | 壳分派、引号与转义 | 命令·bash·管道 |
| 执行 | `SP-OUTPUT` | 输出控量（别全量打印） | 日志·刷屏·截断 |
| 执行 | `SP-VERDICT` | 判「结束没有」只看硬信号 | 后台·端口·服务 |
| 执行 | `SP-SCRIPT` | 脚本库与一次性脚本 | 脚本·统计·解析 |
| 定位 | `SP-LOCATE` | 检索锚点优先级 | 搜索·定位·锚点 |
| 定位 | `SP-GLOB` | 通配符与批量 | 通配符·批量 |
| 定位 | `SP-READBIG` | 大文件分级读、噪声目录排除 | 大文件·全文·遍历 |
| 改动 | `SP-REFACTOR` | 大批量改动的固定次序 | 重构·批量·改名 |
| 改动 | `SP-BACKUP` | 改前留底、基线看权威源 | 备份·恢复·覆盖 |
| 网络 | `SP-NET` | 联网取数 | curl·网页·api |
| 语言 | `SP-PYENV` | 解释器/venv 选择、pip 隔离 | python·venv·依赖 |
| 语言 | `SP-RUSTC` | check 与 build 分工、编译期嵌入 | cargo·编译·target |
| 语言 | `SP-NODE` | npm 不加 -g、锁文件、node_modules | npm·package.json |
| 平台 | `SP-WINENC` | 中文乱码、findstr 陷阱、空格路径 | 乱码·编码·路径 |
| 流程 | `SP-GIT` | status 先行、精确 add、备份≠原版 | git·提交·回滚 |
| 流程 | `SP-TEST` | 先跑基线、失败三态归因 | 测试·断言·回归 |

**一条规范的写法**：

- 正文 `specs/rules/<name>.md` —— **无标记、无标题**（标题由代码拼 `### <ID>`）
- 摘要 `specs/summaries/<ID>.md` —— **一行**，超预算时给模型看
- 触发词在 `agent/specs.rs` 的 `Spec.triggers` 里**就地声明**，不另设匹配表
- **触发词要宽不要窄**：漏命中 = 模型静默失去知识；误命中 = 只多几百字节。
  但避免单字（「等」「跑」）——那会让几乎每个任务都命中，等于没分层

**改完必跑** `cargo check --all-targets`。`specs_tests` 钉住五件事：
触发词全小写（否则永远不命中）、id/摘要/正文非空、**无命中回落基线**、
**有命中不叠基线**、预算闸门生效。

**基线兜底**（`specs.rs` 的 `BASELINE`）：**一个触发词都没命中的任务**注入
`SP-SHELL` + `SP-LOCATE` —— 跑命令与找代码是编码任务的地基，不能因为题面是
一句中文短话就两手空空。已命中任一规范的任务不叠基线。

> ⚠️ **别用「加宽触发词」来解决漏命中** —— 宽到人人命中就等于变回常驻，白做分层。
> 兜底交给 `BASELINE`，触发词保持精准。

**跨层断言**：知识搬到哪一层，测试就在哪一层断 ——
`cmd_tools_tests` 的 `locate_strategy_reaches_model` / `hard_signal_rule_reaches_model`
同时断言 description 侧与 specs 侧。改分层时**必须同步改断言**，否则测试守的是一个不存在的契约。
