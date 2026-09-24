//! tools/cmd_tools.rs 的测试外置（部门盘查：测试全部移出核心文件）
#![cfg(windows)]

use super::*;

/// 测试用工作目录 = 编译期注入的 crate 根（`server/`），任何机器上都存在。
const WS: &str = env!("CARGO_MANIFEST_DIR");

#[cfg(test)]
mod cargo_rebuild_guard_tests {
    use super::*;
    #[test]
    fn detects_rebuild_commands() {
        assert!(is_cargo_rebuild("cargo build"));
        assert!(is_cargo_rebuild("cargo run"));
        assert!(is_cargo_rebuild("cargo test"));
        assert!(is_cargo_rebuild("cargo build --release"));
        assert!(is_cargo_rebuild("D:/tools/cargo/bin/cargo.exe run"));
        assert!(!is_cargo_rebuild("cargo check"));
        assert!(!is_cargo_rebuild("cargo fmt"));
        assert!(!is_cargo_rebuild("python -c \"cargo build\""));
        assert!(!is_cargo_rebuild("npm run dev"));
    }
}

#[cfg(test)]
mod wait_ready_tests {
    use super::*;

    #[tokio::test]
    async fn wait_port_detects_listening_port() {
        // 监听真实端口 → wait_port 应立即就绪（契约 P0-1：就绪即返回成功）
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        let r = wait_for_ready(Some(port), None, None, Duration::from_secs(3)).await;
        assert!(r.is_ok(), "监听端口应立即就绪: {r:?}");
    }

    #[tokio::test]
    async fn wait_port_times_out_on_closed_port() {
        // 端口无监听 → 轮询直到超时 → Err（不 kill 进程的前提：无进程）
        let port = {
            let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            l.local_addr().unwrap().port()
        };
        let r = wait_for_ready(Some(port), None, None, Duration::from_millis(1200)).await;
        assert!(r.is_err(), "关闭端口应超时: {r:?}");
        assert!(r.unwrap_err().contains("超时"), "错误应含超时说明");
    }
}

#[cfg(test)]
mod run_integration_tests {
    use super::*;

    /// 修复前：Rust 按 CommandLineToArgvW 转义 \" → cmd 报"'\"D:\x\test.bat\"' 不是内部或外部命令"。
    #[tokio::test]
    async fn call_quoted_bat_path_executes() {
        let dir = std::env::temp_dir().join(format!("real_run_j45_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bat = dir.join("test_script.bat");
        std::fs::write(&bat, "@echo off\r\necho RUN_OK\r\n").unwrap();
        let tool = RunTool;
        let quoted = format!("call \"{}\"", bat.display());

        // 形态 1：直接 call（工具自动包 cmd /C + raw_arg）
        let out = tool
            .run(json!({"command": quoted.clone(), "cwd": dir.to_string_lossy()}))
            .await
            .unwrap();
        assert!(out.contains("RUN_OK"), "call 直接执行必须成功: {out}");
        assert!(
            !out.contains("不是内部或外部命令"),
            "不得出现引号转义错误: {out}"
        );

        // 形态 2：模型旧习惯 cmd /c call "path"（双层 cmd 也应工作）
        let out2 = tool
            .run(json!({"command": format!("cmd /c {quoted}"), "cwd": dir.to_string_lossy()}))
            .await
            .unwrap();
        assert!(out2.contains("RUN_OK"), "cmd /c call 也应成功: {out2}");
        assert!(
            !out2.contains("不是内部或外部命令"),
            "双层 cmd 不得出现引号转义错误: {out2}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 必须放行。端到端：在临时目录建 target/debug/，把 cmd.exe 复制成 log-collector.exe，
    #[tokio::test]
    async fn cargo_build_artifact_runs_not_blocked() {
        let dir = std::env::temp_dir().join(format!("real_run_art_{}", std::process::id()));
        let td = dir.join("target").join("debug");
        std::fs::create_dir_all(&td).unwrap();
        let exe = td.join("log-collector.exe");
        // 用 cmd.exe 冒充构建产物（真 .exe，可执行；/c echo 是 cmd 自身的语义）
        let cmd_src = std::env::var("ComSpec")
            .unwrap_or_else(|_| "C:\\Windows\\System32\\cmd.exe".into());
        std::fs::copy(&cmd_src, &exe).unwrap();
        let tool = RunTool;
        let out = tool
            .run(json!({
                "command": r"target\debug\log-collector.exe /c echo ARTIFACT_OK",
                "cwd": dir.to_string_lossy()
            }))
            .await
            .unwrap();
        assert!(
            !out.contains("COMMAND_NOT_ALLOWED"),
            "cargo 构建产物不得被白名单拦: {out}"
        );
        assert!(
            out.contains("ARTIFACT_OK"),
            "构建产物应真实执行成功: {out}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 不再返回 Err（避免模型把"预期红"误当工具故障）。Worker 按 is_hard_error 仍触发 Replanner。
    #[tokio::test]
    async fn nonzero_exit_returns_run_failed_error() {
        let tool = RunTool;
        let out = tool
            .run(json!({"command": "cmd /C exit 1", "cwd": WS}))
            .await;
        assert!(
            out.is_ok(),
            "非零退出必须返回 Ok（黄信封，非工具故障）: {out:?}"
        );
        let v: Value = serde_json::from_str(out.unwrap().as_str()).expect("结果必须是合法 JSON");
        assert_eq!(v["data"]["exit_code"], 1, "必须保留真实退出码");
        assert!(
            v["data"].get("cwd").is_none(),
            "主路径不应回 cwd（冗余字段，2026-09-16 已瘦身）: {}",
            v["data"]
        );
        assert_eq!(
            v["error"]["code"], "RUN_FAILED",
            "必须带结构化 RUN_FAILED 码"
        );
        assert!(
            v["error"]["suggestion"]
                .as_str()
                .map(|s| s.contains("黄"))
                .unwrap_or(false),
            "黄标注必须可见: {v}"
        );
    }

    /// {background:true, pid, log_file}（不等待退出码），日志文件被写入；stop_process 可终止。
    #[tokio::test]
    async fn background_runs_and_logs_then_stop() {
        let dir = std::env::temp_dir().join(format!("real_run_bg_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let tool = RunTool;
        // 后台启动一个持续写日志的进程（PowerShell 循环 15 次写日志，16 秒后自停防残留）
        let bg_out = tool
            .run(json!({
                "command": "powershell -NoProfile -Command \"for($i=1;$i -le 15;$i++){Write-Output ('BG_LINE_' + $i); Start-Sleep -Milliseconds 100}\"",
                "cwd": dir.to_string_lossy(),
                "background": true
            }))
            .await
            .unwrap();
        let v: Value = serde_json::from_str(&bg_out).expect("background 结果必须是合法 JSON");
        assert_eq!(v["data"]["background"], true, "必须标注 background: {v}");
        assert!(v["data"]["pid"].as_u64().is_some(), "必须返回 pid: {v}");
        let log_file = v["data"]["log_file"].as_str().unwrap().to_string();
        assert!(
            std::path::Path::new(&log_file).exists(),
            "日志文件必须已创建: {log_file}"
        );
        // 进程后台跑：等待片刻后日志应有内容
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        let content = std::fs::read_to_string(&log_file).unwrap_or_default();
        assert!(
            content.contains("BG_LINE"),
            "日志应包含后台输出，实际: {content:?}"
        );
        // stop_process 终止
        let pid = v["data"]["pid"].as_u64().unwrap();
        let stop_out = tool
            .run(json!({"command": "echo stop", "stop_process": pid, "cwd": dir.to_string_lossy()}))
            .await
            .unwrap();
        assert!(
            stop_out.contains("已终止"),
            "stop_process 应报告终止: {stop_out}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 打印+睡眠的常驻式命令，采样到点自动 kill，返回已捕获的部分输出。
    #[tokio::test]
    async fn sample_seconds_kills_and_returns_partial() {
        let dir = std::env::temp_dir().join(format!("real_run_sample_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let tool = RunTool;
        let out = tool
            .run(json!({
                "command": "powershell -NoProfile -Command \"for($i=1;$i -le 30;$i++){Write-Output ('SAMPLE_' + $i); Start-Sleep -Milliseconds 200}\"",
                "cwd": dir.to_string_lossy(),
                "sample_seconds": 3
            }))
            .await
            .unwrap();
        let v: Value = serde_json::from_str(&out).expect("结果必须是合法 JSON");
        assert_eq!(v["ok"], true, "采样必须返回 ok（非 Err）: {v}");
        let stdout = v["data"]["stdout"].as_str().unwrap_or("");
        assert!(
            stdout.contains("SAMPLE_"),
            "采样应返回截至 kill 前已打印的部分输出，实际: {stdout}"
        );
        // 3s 内约打印十几行，不该全部 30 行都回来（证明确实被 kill 了）
        assert!(
            !stdout.contains("SAMPLE_30"),
            "采样到点应已 kill，不应包含末尾行: {stdout}"
        );
        let warn = v["warnings"][0].as_str().unwrap_or("");
        assert!(
            warn.contains("采样"),
            "warning 应标注采样完成: {warn}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 也必须返回**截至超时前已捕获的部分输出**，而非空壳 TIMEOUT 错误——否则模型看到空白
    #[tokio::test]
    async fn timeout_returns_partial_output() {
        let dir = std::env::temp_dir().join(format!("real_run_to_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let tool = RunTool;
        let out = tool
            .run(json!({
                "command": "powershell -NoProfile -Command \"for($i=1;$i -le 120;$i++){Write-Output ('T_' + $i); Start-Sleep -Milliseconds 100}\"",
                "cwd": dir.to_string_lossy(),
                "timeout": 5
            }))
            .await
            .unwrap();
        let v: Value = serde_json::from_str(&out).expect("结果必须是合法 JSON");
        assert_eq!(v["ok"], true, "超时也必须返回 ok（携带部分输出）: {v}");
        let stdout = v["data"]["stdout"].as_str().unwrap_or("");
        assert!(
            stdout.contains("T_"),
            "超时必须返回部分输出，不应空白，实际: {stdout}"
        );
        let warn = v["warnings"][0].as_str().unwrap_or("");
        assert!(
            warn.contains("超时"),
            "warning 应标注超时: {warn}"
        );
        // + 可解析 marker 标记行（[timed out after Ns]），不再让模型靠 exit_code=-1 猜。
        assert_eq!(
            v["data"]["termination"]["kind"], "timed_out",
            "超时终止原因必须是 timed_out: {v}"
        );
        let marker = v["data"]["marker"].as_str().unwrap_or("");
        assert!(
            marker.contains("[timed out after 5s]"),
            "超时必须有可解析标记行，实际: {marker}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 解析错）：模型在 powershell 命令末尾手写 `2>&1`（后端已原生抓 stderr，冗余），被塞进
    #[tokio::test]
    async fn powershell_terminal_2and1_no_remoteexception() {
        let dir = std::env::temp_dir().join(format!("real_run_2and1_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let tool = RunTool;
        let out = tool
            .run(json!({
                // 内层用单引号：`\"` 是 cmd 的转义习惯，PowerShell 里不成立（PS 用反引号），
                "command": "powershell -Command \"cmd /C 'echo boom 1>&2' 2>&1\"",
                "cwd": dir.to_string_lossy()
            }))
            .await
            .unwrap();
        let v: Value = serde_json::from_str(&out).expect("结果必须是合法 JSON");
        let combined = format!(
            "{} {}",
            v["data"]["stdout"].as_str().unwrap_or(""),
            v["data"]["stderr"].as_str().unwrap_or("")
        );
        assert!(
            !combined.to_lowercase().contains("remoteexception"),
            "末尾 2>&1 被剥后不应出现 PS RemoteException 解析错，实际: {combined}"
        );
        assert!(
            combined.contains("boom"),
            "stderr 原文必须可见（Write-Error 内容），实际: {combined}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// "不存在" → 幂等归一为成功（目标已达成），不再误判"被拦截/失败"。
    #[tokio::test]
    async fn remove_item_missing_target_is_idempotent_success() {
        let tool = RunTool;
        let dir = std::env::temp_dir().join(format!("real_run_idem_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // 文件不存在就删 → PowerShell exit 1 + "找不到路径/因为该路径不存在"
        let out = tool
            .run(json!({
                "command": format!("Remove-Item -Force '{}'", dir.join("ghost.txt").display()),
                "cwd": dir.to_string_lossy()
            }))
            .await
            .unwrap();
        let v: Value = serde_json::from_str(&out).expect("结果必须是合法 JSON");
        assert_eq!(
            v["data"]["exit_code"], 0,
            "删除不存在的文件应幂等归一为成功: {v}"
        );
        assert_eq!(v["ok"], true, "ok 应为 true: {v}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 是 cmd 语义（没找到）——必须转 success + 标注，模型不得误判为命令失败。
    #[tokio::test]
    async fn findstr_no_match_is_success_not_failure() {
        let tool = RunTool;
        let out = tool
            .run(json!({
                "command": format!("dir {WS} 2>&1 | findstr /C:\"__绝对不存在的关键词_9f8d__\""),
                "cwd": WS,
                "timeout": 15,
            }))
            .await
            .expect("findstr 无匹配应转 success，不得 Err");
        let v: Value = serde_json::from_str(&out).expect("合法 JSON");
        assert_eq!(v["ok"], true, "findstr 无匹配应 ok: {out}");
        let warn = v["warnings"].as_array().map(|a| a.len()).unwrap_or(0);
        assert!(warn >= 1, "应有「无匹配」标注: {out}");
    }

    /// Select-Object 的管道应走 PowerShell 执行，不得在 cmd 下报命令不存在。
    #[tokio::test]
    async fn select_object_pipeline_runs_in_powershell() {
        let tool = RunTool;
        let out = tool
            .run(json!({
                "command": "where node | Select-Object -First 1",
                "cwd": WS,
                "timeout": 20,
            }))
            .await
            .expect("Select-Object 管道应走 powershell 成功");
        let v: Value = serde_json::from_str(&out).expect("合法 JSON");
        assert_eq!(v["ok"], true, "Select-Object 管道应成功: {out}");
        let stdout = v["data"]["stdout"].as_str().unwrap_or("");
        assert!(
            !stdout.to_lowercase().contains("不是内部或外部命令"),
            "不得出现 cmd 报错: {out}"
        );
    }

    /// 零退出仍返回 ok:true（不应误报失败）
    #[tokio::test]
    async fn zero_exit_returns_ok() {
        let tool = RunTool;
        let out = tool
            .run(json!({"command": "cmd /C echo OK", "cwd": WS}))
            .await
            .expect("零退出必须 Ok");
        assert!(
            !out.contains("RUN_FAILED"),
            "成功命令不应带 RUN_FAILED: {out}"
        );
        let v: Value = serde_json::from_str(&out).expect("结果必须是合法 JSON");
        assert_eq!(v["ok"], true);
        assert_eq!(v["data"]["exit_code"], 0);
    }

    /// run 输出截断必须保留**尾部**（test result/错误摘要常在尾部），头尾双保留。
    #[test]
    fn truncate_output_keeps_tail_with_conclusion() {
        // 超长输出：尾部结论（test result: ok）必须保留
        let mut s = String::new();
        for i in 0..3000 {
            s.push_str(&format!("行 {i}\n"));
        }
        s.push_str("test result: ok. 432 passed; 0 failed\n");
        let t = truncate_output(&s, 2000);
        assert!(
            t.contains("test result: ok. 432 passed"),
            "尾部结论必须保留: {t}"
        );
        assert!(t.contains("中间省略"), "应有省略标记: {t}");
        assert!(t.chars().count() <= 2100, "长度受限: {}", t.chars().count());
        // 短输出不截断
        assert_eq!(truncate_output("hello", 2000), "hello");
    }

    ///（模型看到"还有 N 个"后知道去哪读全文）；未超限不落盘。
    #[test]
    fn truncate_with_spill_writes_full_output_file() {
        let dir = std::env::temp_dir().join(format!("real_spill_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // 超限 → 截断 + spill
        let big = "X".repeat(5000);
        let (t, spill) = truncate_with_spill(&big, 1000, &dir, "test-cmd");
        assert!(t.contains("中间省略"), "截断文本应有省略标记: {t}");
        assert!(t.contains("全文已落盘"), "截断文本应指引全文落盘位置: {t}");
        let spill = spill.expect("超限必须返回 spill 路径");
        assert!(std::path::Path::new(&spill).exists(), "spill 文件必须已落盘: {spill}");
        let full = std::fs::read_to_string(&spill).unwrap();
        assert_eq!(full.chars().count(), 5000, "spill 必须是全文: {}", full.chars().count());
        assert!(
            spill.contains("spill"),
            "spill 应落在数据根 spill 目录（2026-09-10 数据根收口后不再用 .real）: {spill}"
        );
        // 未超限 → 原样返回，无 spill
        let short = "hi".to_string();
        let (t2, spill2) = truncate_with_spill(&short, 1000, &dir, "test-cmd");
        assert_eq!(t2, "hi", "短输出原样: {t2}");
        assert!(spill2.is_none(), "短输出不落盘");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 剥离**末尾** `2>&1`（后端已原生抓 stderr，冗余），但保留喂管道的 `2>&1 | xxx`。
    #[test]
    fn strip_terminal_redirects_handles_2and1() {
        // 末尾 `2>&1` 必须剥掉
        assert_eq!(
            strip_terminal_redirects("cargo build 2>&1").trim(),
            "cargo build",
            "末尾 2>&1 应剥离"
        );
        // 管道前的 `2>&1` 必须保留（喂管道）
        assert_eq!(
            strip_terminal_redirects("cargo build 2>&1 | findstr error").trim(),
            "cargo build 2>&1 | findstr error",
            "喂管道的 2>&1 必须保留"
        );
        assert_eq!(
            strip_terminal_redirects("Remove-Item x -Force; cargo build 2>&1").trim(),
            "Remove-Item x -Force; cargo build",
            "序列末尾 2>&1 应剥离"
        );
        // 无 `2>&1` 不变
        assert_eq!(strip_terminal_redirects("cargo build").trim(), "cargo build");
        // token 内部的 `2>&1` 子串不误剥
        assert_eq!(
            strip_terminal_redirects("echo std2>&1out").trim(),
            "echo std2>&1out"
        );
    }

    /// 形态②：模型自己包 PS 时写成 `powershell -Command "… 2>&1"`，`2>&1` 落在引号内
    #[test]
    fn strip_terminal_redirects_reaches_inside_trailing_quotes() {
        assert_eq!(
            strip_terminal_redirects("powershell -Command \"cmd /C 'echo boom 1>&2' 2>&1\"").trim(),
            "powershell -Command \"cmd /C 'echo boom 1>&2'\"",
            "引号内的末尾 2>&1 应剥离，引号本身保留"
        );
        // 引号内无 `2>&1` → 原样
        assert_eq!(
            strip_terminal_redirects("powershell -Command \"Get-ChildItem\"").trim(),
            "powershell -Command \"Get-ChildItem\""
        );
    }

    /// 护栏：`> file 2>&1` 是重定向搭配——`2>&1` 的作用是让 stderr 也写进该文件。
    #[test]
    fn strip_terminal_redirects_keeps_redirect_pairing() {
        let with_redirect_inside_quotes =
            "cmd /C \"cd /d D:\\Phoenix\\server && cargo run > D:\\Phoenix\\backend.log 2>&1\"";
        assert_eq!(
            strip_terminal_redirects(with_redirect_inside_quotes).trim(),
            with_redirect_inside_quotes,
            "引号内的重定向搭配不得剥离"
        );
        assert_eq!(
            strip_terminal_redirects("cargo build > out.txt 2>&1").trim(),
            "cargo build > out.txt 2>&1",
            "扁平形态的重定向搭配同样不得剥离"
        );
    }

    /// 形态②只对 PowerShell 承载的命令生效：`cmd /c "… 2>&1"` 由 cmd 解释、本就合法，
    #[test]
    fn strip_terminal_redirects_skips_quotes_under_cmd() {
        let under_cmd = "cmd /c \"set A=1&& cd /d D:\\project-b\\server&& cargo check 2>&1\"";
        assert_eq!(
            strip_terminal_redirects(under_cmd).trim(),
            under_cmd,
            "cmd 承载的引号内 2>&1 不得剥离"
        );
        // 后端自己包的 cmdlet 命令是扁平末尾形态，不经过这条护栏
        assert_eq!(
            strip_terminal_redirects("Get-ChildItem C:\\ -Recurse 2>&1").trim(),
            "Get-ChildItem C:\\ -Recurse",
            "扁平末尾（cmdlet 通路）仍应剥离"
        );
    }

    /// 锁文件类失败签名必须被归类，给出确定性提示而非让模型猜"残留进程"。
    #[test]
    fn locked_file_hint_classifies_os_error_5() {
        let sig = "error: failed to remove file `D:\\x\\target\\debug\\app.exe`\nCaused by:\n  拒绝访问。 (os error 5)";
        let hint = locked_file_hint(&sig.to_lowercase());
        assert!(hint.is_some(), "os error 5 应被归类");
        let h = hint.unwrap();
        assert!(h.contains("杀软") || h.contains("占用"), "提示应点明真因: {h}");
        assert!(h.contains("stop_process"), "提示应给可操作手段: {h}");

        // 非锁文件类失败不应误判
        assert!(
            locked_file_hint("command not found: foobar").is_none(),
            "普通失败不应归类为锁文件"
        );
        // Windows 共享冲突码 os error 32 也应归类
        assert!(
            locked_file_hint("the process cannot access the file (os error 32)").is_some(),
            "os error 32 应被归类"
        );
    }

    /// 经 cmd /C 执行时 cmd 剥引号 → 代码内空格拆参数 → python 收到残缺代码（SyntaxError
    #[tokio::test]
    async fn python_c_inline_code_not_truncated() {
        let tool = RunTool;
        let out = tool
            .run(json!({
                "command": "python -c \"print(sum(range(1, 11)))\"",
                "cwd": WS,
            }))
            .await
            .expect("python -c 必须执行成功");
        let v: Value = serde_json::from_str(&out).expect("结果必须是合法 JSON");
        assert_eq!(v["data"]["exit_code"], 0, "exit_code 应为 0: {out}");
        let stdout = v["data"]["stdout"].as_str().unwrap_or("");
        assert!(stdout.contains("55"), "stdout 应包含 55，实际: {stdout}");
    }

    /// `node -e "代码含正则"` 经 cmd /C 执行时引号被剥 → 代码碎 → 模型换脚本文件。原生
    #[tokio::test]
    async fn node_e_inline_code_not_misparsed() {
        let tool = RunTool;
        let out = tool
            .run(json!({
                "command": "node -e \"console.log([1,2,3].map(x=>x*2).join(','))\"",
                "cwd": WS,
            }))
            .await
            .expect("node -e 必须执行成功");
        let v: Value = serde_json::from_str(&out).expect("结果必须是合法 JSON");
        assert_eq!(v["data"]["exit_code"], 0, "exit_code 应为 0: {out}");
        let stdout = v["data"]["stdout"].as_str().unwrap_or("");
        assert!(stdout.contains("2,4,6"), "stdout 应含 2,4,6，实际: {stdout}");
    }

    #[tokio::test]
    async fn defused_inline_script_actually_runs() {
        let tool = RunTool;
        // 代码含 `"` ⇒ 命中 risky 判据 ⇒ 触发转轨（本用例的前提）
        let out = tool
            .run(json!({
                "command": "python -c \"print(\"DEFUSE_MARKER_9182\")\"",
                "cwd": WS,
            }))
            .await
            .expect("转轨后的命令必须执行成功");
        let v: Value = serde_json::from_str(&out).expect("结果必须是合法 JSON");
        let stdout = v["data"]["stdout"].as_str().unwrap_or("");
        assert!(
            stdout.contains("DEFUSE_MARKER_9182"),
            "转轨后必须真执行落盘脚本（空输出 = 落回了 `-c pass`）: {out}"
        );
    }

    /// 同型：`node -e` 代码含引号 ⇒ 同样转轨 ⇒ 同样必须真执行（不得落回 `-e` 分支）。
    #[tokio::test]
    async fn defused_node_inline_script_actually_runs() {
        let tool = RunTool;
        let out = tool
            .run(json!({
                "command": "node -e \"console.log(\"NODEMARKER_731\")\"",
                "cwd": WS,
            }))
            .await
            .expect("转轨后的命令必须执行成功");
        let v: Value = serde_json::from_str(&out).expect("结果必须是合法 JSON");
        let stdout = v["data"]["stdout"].as_str().unwrap_or("");
        assert!(
            stdout.contains("NODEMARKER_731"),
            "node 转轨后必须真执行: {out}"
        );
    }

    ///（引号正确）——错误不得再是 COMMAND_NOT_ALLOWED。
    #[tokio::test]
    async fn curl_is_whitelisted_and_native() {
        let tool = RunTool;
        // 访问本地一个必失败的地址：错误应是网络类（连接失败），而非白名单拦截
        let out = tool
            .run(json!({
                "command": "curl -s -o NUL -w \"%{http_code}\" http://127.0.0.1:1/x",
                "cwd": WS,
                "timeout": 10,
            }))
            .await
            .unwrap_or_else(|e| {
                assert!(
                    !e.contains("COMMAND_NOT_ALLOWED"),
                    "curl 不得被白名单拦截: {e}"
                );
                e
            });
        assert!(
            !out.contains("COMMAND_NOT_ALLOWED"),
            "curl 不得报白名单: {out}"
        );
    }

    /// `curl A && curl B && echo C` 时，curl 原生执行分支把 `&& curl B && echo C` 全当
    #[tokio::test]
    async fn curl_with_and_and_chain_runs_via_cmd() {
        let tool = RunTool;
        let out = tool
            .run(json!({
                "command": "curl -s -o NUL https://example.com && echo SEP-OK && echo SECOND-OK",
                "cwd": WS,
                "timeout": 20,
            }))
            .await
            .expect("curl && 链应执行（不再把 && 当参数）");
        let v: Value = serde_json::from_str(&out).expect("结果必须是合法 JSON");
        let stdout = v["data"]["stdout"].as_str().unwrap_or("");
        let stderr = v["data"]["stderr"].as_str().unwrap_or("");
        assert!(
            !stderr.contains("Could not resolve host: curl"),
            "不得再报 'Could not resolve host: curl'（&& 后的 curl 被当主机名）: {out}"
        );
        assert!(
            stdout.contains("SEP-OK") && stdout.contains("SECOND-OK"),
            "&& 分隔后的 echo 必须执行（cmd /C 正确解析分隔）: stdout={stdout:?} stderr={stderr:?}"
        );
    }

    #[tokio::test]
    async fn python3_alias_maps_to_python() {
        let tool = RunTool;
        let out = tool
            .run(json!({
                "command": "python3 -c \"print(sum(range(1, 11)))\"",
                "cwd": WS,
            }))
            .await
            .expect("python3 必须映射 python 执行成功");
        let v: Value = serde_json::from_str(&out).expect("结果必须是合法 JSON");
        assert_eq!(v["data"]["exit_code"], 0, "python3 映射执行应成功: {out}");
        let stdout = v["data"]["stdout"].as_str().unwrap_or("");
        assert!(stdout.contains("55"), "stdout 应包含 55，实际: {stdout}");
    }

}

#[cfg(test)]
mod output_contract_cmd_tests {
    use super::*;
    use crate::mcp::registry::BuiltinTool;
    use crate::tools::cmd_translate::translate_unix_pipeline;

    #[tokio::test]
    async fn cd_command_without_drive_path_not_touched() {
        let tool = RunTool;
        let out = tool
            .run(json!({"command": "cd /d D:/proj && python t.py", "cwd": "."}))
            .await
            .unwrap();
        assert!(
            out.contains("cd /d D:/proj") || out.contains("cd /d D:\\\\proj"),
            "已带 /d 不重复（斜杠形态 cmd 等价）: {out}"
        );
        let out2 = tool
            .run(json!({"command": "cd subdir && dir", "cwd": "."}))
            .await
            .unwrap();
        assert!(out2.contains("cd subdir"), "非盘符路径不加 /d: {out2}");
    }

    #[tokio::test]
    async fn cat_routes_to_git_bash_untranslated() {
        if crate::tools::cmd_bash::locate().is_none() {
            return;
        }
        let tool = RunTool;
        let out = tool
            .run(json!({"command": "cat C:/tmp/x.py", "cwd": "."}))
            .await
            .unwrap();
        let v: Value = serde_json::from_str(&out).expect("结果必须是合法 JSON");
        assert_eq!(
            v["data"]["command"], "cat C:/tmp/x.py",
            "cat 必须原样交 Git Bash（不得再翻译成 type）: {out}"
        );
    }

    #[tokio::test]
    async fn grep_routes_to_git_bash_untranslated() {
        if crate::tools::cmd_bash::locate().is_none() {
            return;
        }
        let tool = RunTool;
        let out = tool
            .run(json!({"command": "grep -n error C:/tmp/x.py", "cwd": "."}))
            .await
            .unwrap();
        let v: Value = serde_json::from_str(&out).expect("结果必须是合法 JSON");
        assert_eq!(
            v["data"]["command"], "grep -n error C:/tmp/x.py",
            "grep 必须原样交 Git Bash（不得再翻译成 findstr）: {out}"
        );
    }

    #[tokio::test]
    async fn find_and_sed_no_longer_rejected() {
        if crate::tools::cmd_bash::locate().is_none() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("real_unix_route_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let tool = RunTool;
        let out = tool
            .run(json!({"command": "find . -name \"*.rs\"", "cwd": dir.to_string_lossy()}))
            .await
            .expect("find 不该再被拒绝");
        let v: Value = serde_json::from_str(&out).expect("结果必须是合法 JSON");
        assert_eq!(
            v["data"]["command"], "find . -name \"*.rs\"",
            "find 必须原样交 Git Bash: {out}"
        );
        let out2 = tool
            .run(json!({"command": "sed -n '1p' x.py", "cwd": dir.to_string_lossy()}))
            .await
            .expect("sed 不该再被拒绝");
        let v2: Value = serde_json::from_str(&out2).expect("结果必须是合法 JSON");
        assert_eq!(
            v2["data"]["command"], "sed -n '1p' x.py",
            "sed 必须原样交 Git Bash: {out2}"
        );
    }

    #[tokio::test]
    async fn cat_in_pipeline_translated_to_type() {
        let out = translate_unix_pipeline("cat x.py | grep main").unwrap();
        assert!(out.contains("type x.py"), "管道 cat 段应翻译为 type: {out}");
        assert!(out.contains("findstr"), "管道 grep 段照常翻译: {out}");
    }
}

#[cfg(test)]
mod wait_exit_tests {
    use super::*;

    /// 行为级验证（设计决定："判断进程而非靠时间等）
    #[tokio::test]
    async fn wait_exit_returns_on_process_exit_with_log() {
        let tool = RunTool;
        let v = tool
            .run(json!({
                "command": "ping -n 2 127.0.0.1 >nul & echo WAIT_EXIT_DONE",
                "background": true,
                "wait_exit": true,
                "cwd": std::env::temp_dir().to_string_lossy().to_string()
            }))
            .await
            .expect("wait_exit 应正常完成");
        let d: Value = serde_json::from_str(&v).unwrap();
        assert_eq!(d["data"]["exit_code"], 0, "进程退出码应透传: {v}");
        let out = d["data"]["stdout"].as_str().unwrap();
        assert!(out.contains("WAIT_EXIT_DONE"), "日志尾部应含输出: {out}");
        assert!(out.contains("进程已退出"), "应标注完成信号: {out}");
    }
}

    #[test]
    fn defuse_rewrites_risky_python_c() {
        let cmd = "python -c \"import json\\nprint('$var')\" --flag";
        let (new_cmd, tmp) = crate::tools::cmd_tools::defuse_inline_script(cmd).expect("高危应转轨");
        assert!(new_cmd.starts_with("python \""), "改写为解释器+临时文件: {new_cmd}");
        assert!(new_cmd.ends_with("--flag"), "尾部参数保留: {new_cmd}");
        let code = std::fs::read_to_string(&tmp).unwrap();
        assert!(code.contains("import json"), "代码应完整落盘: {code}");
        assert!(code.contains("$var"), "$ 不被剥离");
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn defuse_passes_clean_c_through() {
        let cmd = "python -c \"print('hello')\"";
        assert!(
            crate::tools::cmd_tools::defuse_inline_script(cmd).is_none(),
            "干净单行 -c 不转轨（零打扰）"
        );
    }

    #[test]
    fn defuse_does_not_eat_quoted_trailing_arg() {
        let cmd = "python -c \"import sys;compile(open(sys.argv[1],encoding='utf-8').read(),sys.argv[1],'exec')\" \"C:\\x\\a.py\"";
        assert!(
            crate::tools::cmd_tools::defuse_inline_script(cmd).is_none(),
            "代码闭合引号在路径参数之前 ⇒ 不该转轨\
             （旧实现会跳到最后那个引号、把参数吞进代码，再写出一个语法错的脚本）"
        );
    }

    /// 同时锁住"**该转轨时，带引号的尾随参数也不能丢**"。
    #[test]
    fn defuse_keeps_quoted_trailing_arg_when_rewriting() {
        let cmd = "python -c \"import json\\nprint('$v')\" \"C:\\x\\a.py\"";
        let (new_cmd, tmp) = crate::tools::cmd_tools::defuse_inline_script(cmd).expect("高危应转轨");
        assert!(
            new_cmd.starts_with("python \""),
            "改写为解释器+临时文件: {new_cmd}"
        );
        assert!(
            new_cmd.ends_with("\"C:\\x\\a.py\""),
            "带引号的尾随参数必须保留: {new_cmd}"
        );
        let code = std::fs::read_to_string(&tmp).unwrap();
        assert!(
            !code.contains("C:\\x\\a.py"),
            "参数**不得**被吞进代码: {code}"
        );
        assert!(code.contains("print"), "代码本身要完整: {code}");
        let _ = std::fs::remove_file(&tmp);
    }

    /// `redirect_target`：给"超时且零输出"的回执指路 —— 判据必须**确定性**。
    #[test]
    fn redirect_target_finds_the_file_not_the_pipe() {
        let f = crate::tools::cmd_tools::redirect_target;
        assert_eq!(
            f("python a.py --list > \"C:\\out dir\\list.txt\"").as_deref(),
            Some("C:\\out dir\\list.txt")
        );
        // 追加模式
        assert_eq!(f("python a.py >> log.txt").as_deref(), Some("log.txt"));
        // **stderr 重定向也要认** —— 错误信息同样不在管道里、同样看不见
        assert_eq!(f("cargo build 2>err.txt").as_deref(), Some("err.txt"));
        // 合并流 `2>&1` 不落文件 ⇒ 不算
        assert_eq!(f("cargo build 2>&1"), None);
        // 引号里的 `>` 是普通字符 ⇒ 不算（命令里没有真重定向）
        assert_eq!(f("echo \"a > b\""), None);
        // 无重定向
        assert_eq!(f("cargo check --all"), None);
        // 多重定向取最后一个（后者生效）
        assert_eq!(f("a > x.txt > y.txt").as_deref(), Some("y.txt"));
    }

    #[test]
    fn defuse_ignores_non_interpreter_commands() {
        assert!(crate::tools::cmd_tools::defuse_inline_script("cargo check --all").is_none());
        assert!(crate::tools::cmd_tools::defuse_inline_script("dir /b").is_none());
    }

    #[test]
    fn resolve_workdir_normalizes_trailing_dot_form() {
        // 真实目录用畸形形态传入（尾部 \. + 混合分隔符）→ 应规范化为干净路径
        let tmp = std::env::temp_dir().join("rw267_probe");
        std::fs::create_dir_all(&tmp).unwrap();
        let malformed = format!("{}\\.", tmp.display());
        let wd = crate::tools::cmd_tools::resolve_workdir(&malformed, &tmp);
        assert_eq!(wd, tmp, "畸形形态应规范化回真实目录: {wd:?}");
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn resolve_workdir_relative_joins_base() {
        let base = std::env::temp_dir().join("rw267_rel");
        std::fs::create_dir_all(base.join("sub")).unwrap();
        let wd = crate::tools::cmd_tools::resolve_workdir("sub", &base);
        assert!(wd.ends_with("sub"), "相对路径应锚定 base: {wd:?}");
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn effective_workdir_trims_segment_tail_and_falls_back_to_ancestor() {
        let base = std::env::temp_dir().join("effwd_probe");
        std::fs::create_dir_all(base.join("a")).unwrap();
        // 段尾带空格/点（CreateProcess 267 元凶之一）→ 净化后命中真实目录
        let dirty = base.join("a .").join(".");
        let got = crate::tools::cmd_tools::effective_workdir(&dirty);
        assert_eq!(got, Some(base.join("a")), "段尾空格应被剥: {got:?}");
        // 不存在的深层目录 → 回落最近存在祖先
        let deep = base.join("no_such_dir").join("deeper");
        let got2 = crate::tools::cmd_tools::effective_workdir(&deep);
        assert_eq!(got2, Some(base.clone()), "应回落到 base: {got2:?}");
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn locate_strategy_reaches_model() {
        // ── description 侧：单壳纪律 + 删除通道 ──
        let d = <RunTool as crate::mcp::registry::BuiltinTool>::description(&RunTool);
        assert!(
            d.contains("一条命令只用一个壳的语法"),
            "单壳纪律缺失（混血命令的来源）"
        );
        assert!(d.contains("Git Bash"), "通道名要写死（改名即测试红）");
        assert!(
            d.contains("rm -f"),
            "删除要指向 Git Bash 那条通道（Remove-Item 走 PS，与 `&` 不兼容）"
        );

        // ── specs 侧：锚点与降级路径（SP-LOCATE）──
        let loc = crate::agent::specs::match_specs("搜索 定位 源码 锚点");
        assert!(loc.contains("SP-LOCATE"), "检索类任务必须命中 SP-LOCATE：{loc}");
        assert!(loc.contains("降级路径"), "降级路径缺失");
        assert!(loc.contains("全库枚举是最后手段"), "枚举的边界必须写死");
        assert!(
            loc.contains("禁止拿 A 文件的锚点去套 B 文件"),
            "改多文件前的排点纪律缺失"
        );

        // ── specs 侧：壳分派表（SP-SHELL）──
        let sh = crate::agent::specs::match_specs("命令 cmd 管道 引号");
        assert!(sh.contains("SP-SHELL"), "命令类任务必须命中 SP-SHELL：{sh}");
        assert!(sh.contains("Git Bash"), "Git Bash 通道必须写给模型");

        // ── specs 侧：大批量改动次序（SP-REFACTOR）──
        let rf = crate::agent::specs::match_specs("重构 拆分 搬移");
        assert!(rf.contains("SP-REFACTOR"), "重构类任务必须命中 SP-REFACTOR：{rf}");
        assert!(rf.contains("先扫引用面"), "拆分类改动的第①步缺失（外部零改动靠它）");
        assert!(rf.contains("改完必须对账"), "对账纪律缺失（编译通过 ≠ 内容没丢）");
        assert!(rf.contains("逐行"), "对账要写明判据是逐行比对");
        assert!(rf.contains("动刀前留底"), "留底纪律缺失");
    }

    /// 硬信号判据：**定调进 specs（SP-VERDICT），命令样例留在 description**。
    #[test]
    fn hard_signal_rule_reaches_model() {
        let d = <RunTool as crate::mcp::registry::BuiltinTool>::description(&RunTool);

        let v = crate::agent::specs::match_specs("后台 服务 进程 端口 启动");
        assert!(v.contains("SP-VERDICT"), "服务/进程类任务必须命中 SP-VERDICT：{v}");
        // 「判结束用哪条命令」属"怎么干活" ⇒ 渠道是 SP-VERDICT，不再要求住在 run 描述里。
        assert!(
            d.contains("tasklist") || v.contains("tasklist"),
            "查进程的具体命令必须可达模型（run 描述或 SP-VERDICT 任一）"
        );
        assert!(v.contains("判「结束没有」"), "硬信号定调段缺失");
        assert!(
            v.contains("不得把「日志里出现某句话」当等待条件"),
            "硬信号禁令缺失：这一条正是事故的直接对策"
        );
        assert!(
            v.contains("svc.py proc"),
            "必须指向脚本库里的现成工具，否则模型还得自己现写"
        );
    }

    #[test]
    fn run_schema_requires_command_or_script() {
        let schema = <RunTool as crate::mcp::registry::BuiltinTool>::input_schema(&RunTool);
        let any_of = schema
            .get("anyOf")
            .and_then(|v| v.as_array())
            .expect("run schema 必须声明 anyOf 条件必填（command 或 script）");
        assert!(any_of.len() >= 2, "anyOf 至少两个分支");
        // 空参数：必须在上游拦住（不允许留到运行时）
        let err = crate::tools::contract::validate(&schema, &serde_json::json!({}))
            .expect_err("空参数必须被拦：既无 command 也无 script");
        let msg = crate::tools::contract::err_text(&err);
        assert!(
            msg.contains("command") && msg.contains("script"),
            "报错需点名两个可选分支: {msg}"
        );
        // 只给修饰字段：同样必须在入口被拦
        assert!(
            crate::tools::contract::validate(&schema, &serde_json::json!({"wait_port": 8900})).is_err(),
            "只给 wait_port 必须被拦（这正是事故形态）"
        );
        // 给其一即放行
        assert!(
            crate::tools::contract::validate(&schema, &serde_json::json!({"command": "cargo check"})).is_ok(),
            "给 command 应放行"
        );
        assert!(
            crate::tools::contract::validate(
                &schema,
                &serde_json::json!({"script": {"code": "print(1)"}})
            )
            .is_ok(),
            "给 script 应放行"
        );
    }

    #[test]
    fn run_decorator_fields_declared_as_modifiers() {
        // 契约（收敛为**单点声明**）：顶层 description 是唯一事实源 ——
        let schema = <RunTool as crate::mcp::registry::BuiltinTool>::input_schema(&RunTool);
        let top = schema
            .get("description")
            .and_then(|d| d.as_str())
            .unwrap_or("");
        assert!(
            top.contains("必须与 command 或 script 同用"),
            "顶层必须声明修饰字段与 command/script 的从属关系: {top}"
        );
        assert!(
            top.contains("二选一"),
            "顶层必须声明 command / script 二选一: {top}"
        );
        for f in [
            "timeout",
            "wait_port",
            "wait_http",
            "wait_process",
            "background",
            "stop_process",
            "wait_exit",
            "sample_seconds",
            "env",
        ] {
            assert!(top.contains(f), "顶层必须逐一点名修饰字段 {f}: {top}");
        }
        // 声明必须与事实一致：点过名的修饰字段要真实存在于 properties（只说不给＝骗模型）
        let props = schema
            .get("properties")
            .and_then(|v| v.as_object())
            .expect("run schema 必须有 properties");
        for f in [
            "timeout",
            "wait_port",
            "wait_http",
            "wait_process",
            "background",
            "stop_process",
            "wait_exit",
            "sample_seconds",
            "env",
        ] {
            assert!(props.contains_key(f), "修饰字段 {f} 必须真实存在于 properties");
        }
    }

/// **bat 资产契约**：cmd.exe 只可靠地执行 **CRLF + 纯 ASCII** 的批处理。
#[cfg(test)]
mod bat_asset_tests {
    // 本模块只查字节，不从 `super` 取任何东西（不加 use，避免 unused_imports 警告）。
    const BUILD_RUN: &[u8] = include_bytes!("../../../build-run.bat");

    #[test]
    fn build_run_bat_is_crlf_only() {
        let crlf = BUILD_RUN.windows(2).filter(|w| *w == b"\r\n").count();
        let lf = BUILD_RUN.iter().filter(|b| **b == b'\n').count() - crlf;
        let cr = BUILD_RUN.iter().filter(|b| **b == b'\r').count() - crlf;
        assert_eq!(
            (lf, cr),
            (0, 0),
            "build-run.bat 必须全是 CRLF（裸 LF={lf} 裸 CR={cr}）：\
             cmd 读 LF-only 的 bat 会吃掉下一行开头 1~2 个字符，短的时候不发作、长了就炸"
        );
    }

    #[test]
    fn build_run_bat_is_pure_ascii() {
        let bad: Vec<(usize, u8)> = BUILD_RUN
            .iter()
            .enumerate()
            .filter(|(_, b)| **b > 127)
            .map(|(i, b)| (i, *b))
            .take(5)
            .collect();
        assert!(
            bad.is_empty(),
            "build-run.bat 必须纯 ASCII（首几处 偏移/字节值：{bad:?}）——\
             非 936 代码页下中文会被当成命令执行"
        );
    }
}
