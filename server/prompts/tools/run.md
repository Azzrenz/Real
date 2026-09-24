# run 工具

执行命令，**默认通道**——一条命令 + 管道能完成的都用它（列目录 `dir /b`；删除 `rm -f '<绝对路径>'` 走 Git Bash，cmd 侧单路径删除用 `Remove-Item '<单个路径>'`）。

Windows 经 `cmd /C`；后端按命令形态**自动分派壳**（Unix 原生命令走 Git Bash），按习惯写即可。**一条命令只用一个壳的语法**——混用两种壳会被确定性拒绝并附上替代写法，照替代写、不要重试。**禁用 `del` / `rd` / `erase` 传正斜杠绝对路径**（`/` 会被 cmd 当成开关）。危险操作的弹窗确认是预期行为，不要为了躲它换写法。

参数见 schema。返回 `{"kind":"run_result","data":{"command","exit_code","stdout","stderr","termination","spill_path"}}`。`spill_path` 非空 = 输出已截断，读该文件、**勿重跑命令**。
