命令在 Windows 下经 `cmd /C` 执行。后端按命令形态**自动分派**到合适的壳，按习惯写即可：

| 类 | 例子 | 走哪 |
|---|---|---|
| Unix 原生 | `ls` `cat` `grep` `find` `sed` `awk` `head` `tail` `wc` `mkdir -p` `rm` `cp` `mv` | **Git Bash 原样执行**——真 grep（UTF-8 精确，`-n` / `-A/-B/-C` / `\|` 交替都可用），引号是标准规则、不被剥 |
| cmd 内建的安全等价 | `type f`、`findstr …` | 后端先改写成 `cat` / `grep` 再交 Git Bash（只认 `/i` `/n` `/v` `/s` `/C:`；表外 flag 退回原 cmd 路径） |
| Windows 专属 | `tasklist` `netstat` `taskkill` `robocopy`、PS cmdlet | cmd / PowerShell |
| 外部程序 | `cargo` `python` `node` `git` | cmd |

路径三种写法都通：`/d/proj/src`、`D:/proj/src`、`D:\proj\src`。

**读文件用 `cat`、找词用 `grep`**（走 Git Bash——中文精确、引号不丢）。
**中文别用 `findstr`**——GBK 编解码必然静默失真，搜不到也不报错。

**引号**：`$var` / `$(...)` 是 PowerShell 语法，cmd 通道内禁用。多行或含嵌套引号的 Python 代码，直接用 `script` 参数（绕开 cmd）。

**一条命令只用一个壳的语法**：要用 Git Bash 的命令（`cat` / `grep` / `rm`）之间用 `;` 串联；cmd 侧命令（`cargo` / `python` / `git`）保持单条。混用两种壳会被确定性拒绝（如 `Remove-Item … & dir /b`）——照后端给的替代写，不要重试。

**删除**：`rm -f '<绝对路径>'`（Git Bash，引号与中文正常、可与别的 Unix 命令用 `;` 串联、幂等）；删目录树 `rm -rf`；cmd 侧单路径删除用 `Remove-Item '<单个路径>'`。**禁用 `del` / `rd` / `erase` 传正斜杠绝对路径**（如 `D:/x/y`）——`/` 会被 cmd 当成开关。