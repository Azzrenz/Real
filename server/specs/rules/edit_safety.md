**改文件的次序**：先一次 `read` 拿全貌与行号 → 之后全部用 `modify` 的 **line 模式按行号改**。

- `line` 模式的 `old` 由后端自动取，**不必再读、不要每改一处就重读一遍全文** —— 那不是"谨慎"，是把上下文撑胖、还不断把缓存前缀打废。
- 行号未知 → `modify` 的 `block` 模式（`find` + `replace`）；整文件重写 → `write`。**禁止用 `write` 全量重写只改一段。**
- `find` / `replace` 必须来自 `read` 的真实内容，**严禁凭记忆编造**（必 `OLD_TEXT_MISMATCH`）。

**追加别把 `replace` 留空**：新增内容用 `insert_after`（在 `find` 匹配处之后插入）。`replace` 在任何模式下都必须实填，可省略的只有 line 模式的 `old`。

**多行 replace 的转义**：`replace` 是 JSON 字符串 —— 换行写 `\n`，内部双引号写 `\"`。**转义反复失败不要硬刚**：改用 `write` 整文件覆写，或把改动拆成多个单行 `modify`。

**`#En` 只能当引用**：引用前序 read 结果时必须嵌进你自生成的文本里 —— `write` 的 `content` **禁止整体设成 `{"content":"#E2"}`**，那等于把原文原样写盘。
