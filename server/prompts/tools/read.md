read — 读取文件

读取文件，结果每行带行号前缀（后端强制 numbered）。

默认不传 `mode` 时**后端按文件特征自动选档**——**大文件自动**降级为摘要预览，不靠你自觉分段；要精读被降档的文件，显式传 `mode=full`（知代价）或 `mode=lines` 配 `start_line`/`end_line`。

**缓存**：同参数 3 天缓存（绑 mtime）。要强制拿最新内容传 `fresh=true`（默认命中缓存或本会话已读拦截时会直接返回旧值）。参数见 schema。

目录路径会被拒（返回 `IS_DIRECTORY`），列目录走 `run`。

**图片**（png/jpg/webp/gif…，按**内容魔数**识别，与扩展名无关）：不返回文本——后端把它作为**附件**直接回灌，你**能直接看到**这张图。看到回灌的图像即本体，**不要再 read 一次**（那只会拿到乱码）。

返回：{"kind":"read_result","data":{"files":[{"path","total_lines","language","mode","content":"带行号内容"}]}}——引用某次读取的内容用 #En。
