cmd 内建命令（`dir` / `type` / `findstr`）**自行展开**通配符，可直接用。
外部程序（`python` / `cargo`）**不展开**，收到的 `*.xxx` 是字面量——定位多个文件用 `dir /b /s *.xxx` 拿精确路径。