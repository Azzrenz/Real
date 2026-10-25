# run 工具

执行命令，**默认通道**——一条命令 + 管道能完成的都用它（列目录 `dir /b`；删除 `rm -f '<绝对路径>'` 走 Git Bash，cmd 侧单路径删除用 `Remove-Item '<单个路径>'`）。

Windows 经 `cmd /C`；后端按命令形态**自动分派壳**（Unix 原生命令走 Git Bash），按习惯写即可。**一条命令只用一个壳的语法**——混用两种壳会被确定性拒绝并附上替代写法，照替代写、不要重试。**禁用 `del` / `rd` / `erase` 传正斜杠绝对路径**（`/` 会被 cmd 当成开关）。危险操作的弹窗确认是预期行为，不要为了躲它换写法。

参数见 schema。返回 `{"kind":"run_result","data":{"command","exit_code","stdout","stderr","termination","spill_path"}}`。`spill_path` 非空 = 输出已截断，读该文件、**勿重跑命令**。

**构建/安装类命令由内核按进程判定（不按时间）**：`cargo build|test|install|run|clippy`、`npm|pnpm|yarn install|build`、`pip install`、`tauri build`、`dotnet build|publish`、`msbuild`、`cmake --build`、`gradle`、`mvn`、`docker build`、`go build|test`、`git clone`、`winget|choco install`、`ollama pull`、`robocopy`、`ffmpeg` 等命中后，内核**强制**后台化 + `wait_exit`，直到进程真退出才返回——你传不传 `background`/`wait_exit`/`timeout` 都不改变这一点。

- `termination.kind = "exited"`：进程已退出，`exit_code` 是**真实退出码**（0 成功 / 非 0 看 stderr），可以直接判定成败。
- `termination.kind = "still_running"`：盯到预算上限（默认 30 分钟）仍未退出，**没有杀它**。先 `proc_status(pid=…)` 看 `active`（CPU 在动=还在编译）还是 `idle`，读 `log_file` 尾部看进度；确认卡死才 `stop_process`。**绝不重发同一条命令**——那会并行起第二份构建，互相抢 target 锁。
- 常驻服务（dev server / watch）不要用本规则：用 `wait_port` / `wait_http` / `wait_process` 等就绪信号，内核不会把带这些参数的命令后台化。
