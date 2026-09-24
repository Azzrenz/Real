**解释器选择**：优先项目自带虚拟环境（`<项目>/.venv/Scripts/python.exe`，或 `venv/`、`.conda/`）；没有就用系统 `python`。**不要**为一次性脚本新建环境。

**装包**：一律装进项目 venv —— `<venv>/Scripts/pip install <pkg>`。**禁止全局 `pip install`**：污染用户环境、不可回滚、可能与系统包冲突。

**版本门槛**：`tomllib` 需 ≥3.11；`match` 需 ≥3.10；`X | Y` 类型注解需 ≥3.10。写脚本前先 `python -V` 确认，别写完才发现语法不认。

**没有 venv 时的次序**：① 项目声明了 `uv`/`poetry`/`pdm` 就按它来；② 标准库能完成就只用标准库（最省事、零副作用）；③ 都不行**先问**，不要擅自全局装。

**中文输出**：脚本里 `print` 中文前设 `PYTHONIOENCODING=utf-8`，或显式 `sys.stdout.reconfigure(encoding='utf-8')`，否则 Windows 控制台按 GBK 编码会抛 `UnicodeEncodeError`。