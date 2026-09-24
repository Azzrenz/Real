**中文乱码的根因**：Windows 控制台默认代码页是 GBK。Git Bash 里跑的命令若输出中文，先设 `PYTHONIOENCODING=utf-8` 或 `chcp 65001`；读文件时显式 `encoding='utf-8'`（Python 默认编码在 Windows 上不是 UTF-8）。

**`findstr` 是 UTF-8 杀手**：它按 GBK 解码，遇到 UTF-8 中文**静默出错** —— 要么漏命中、要么输出乱码，且不报错。**搜中文一律用 `grep -rn`**（Git Bash 自带），不要用 `findstr`。

**含空格的路径**：一律双引号包裹（`"D:\my dir\a.txt"`）。Git Bash 下用正斜杠更稳，注意反斜杠在双引号里会被当转义符吃掉。

**`.bat` 脚本一律纯 ASCII 英文**：非 936 代码页下，批处理里的中文会被当命令执行。

**长路径**：Windows 默认 260 字符上限，深层 `node_modules` 会撞。遇到 `path too long` 用 `subst` 映射短盘符，或把仓库移到更浅的根目录。

**写入不做换行转换**：`write` / `modify` 的 content 按原样字节落盘。BAT/cmd 等 CRLF 敏感文件**必须显式写 `\r\n`**（JSON 转义）——写成 LF-only 的 BAT 会让 cmd.exe 解析异常或双击闪退。