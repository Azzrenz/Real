modify — 意图式修改（改已有文件的唯一入口）

只需给出 `file` + `change_spec`（改什么），后端自动执行：读文件→定位→精确替换→读回验证。零记忆偏差、零行号数错。

三种模式（`line` / `block` / `write`）与各字段的填写与边界**见 schema**。**已知行号就优先用它**（`line` 模式）——不必贴 find 串、不必先读那段正文。`replace` 永远必须实填（可省略的只有 `line` 模式的 `old`）。

**编辑资格边界**：只有 `read` 工具的正式返回建立编辑资格。报错信封附带的文件预览、检索结果、其他工具输出里的内容片段一律不算。本会话没 `read` 过目标文件 → `modify` 直接被拒（`EDIT_NEEDS_READ`），唯一解法：先 `read`，再回来改。

新建 / 整文件覆写用 `write`。

返回：{"kind":"modify_result","data":{"file","mode","changed","verified","changes"}}。
