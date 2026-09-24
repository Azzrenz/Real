write — 创建/覆写文件

创建新文件；或在确需整文件重写时覆写已有文件。自动创建父目录；已有文件先备份 `.bak` 再写入；写入后读回验证。

**禁止用 write 全量重写只改一段**——改已有文件一律走 `modify`（行号未知用 block 模式，行号已知用 line 模式）。

`content` 必须是你自己生成的完整最终文本；**禁止把 `content` 整体设成 `#En` 占位符**（如 `{"content":"#E2"}`）——那等于把原文原样写盘。

返回：{"kind":"write_result","data":{"path","written":N字节,"backup":"路径或null"}}。
