//! 进程活度观察工具（从 cmd_tools.rs 独立）：判断进程而非靠时间等——

use crate::mcp::registry::BuiltinTool;
use crate::tools::contract::{err_text, validate};
use async_trait::async_trait;
use serde_json::{json, Value};

// proc_status —— 进程活度观察（只读， 设计决定
pub struct ProcStatusTool;

#[async_trait]
impl BuiltinTool for ProcStatusTool {
    fn name(&self) -> &'static str { "proc_status" }
    fn description(&self) -> &'static str {
        "观察进程活度（只读，0 副作用）：判断长任务进程（后台 run / 构建等）\
         活着且在干活 / 活着但疑似卡住 / 已退出，并看清每个 PID 是谁（带命令行）。\
         用法：长任务跑很久时查它是否仍在推进——cpu_delta>0=在干活；长时间 0=疑似卡死\
         （配合日志确认后再决定 stop_process，勿凭感觉杀）。\
         参数：pid=只看指定进程（返回 active/idle/exited 判定）；name=按进程名过滤；都不给=全量\
         按内存降序前 60。\
         返回 JSON：{\"kind\":\"proc_status_result\",\"data\":{\"observed\":[{\"pid\":..,\"name\":\"..\",\
         \"cpu_delta\":..,\"mem_mb\":..,\"cmd\":\"..\"}],\"note\":\"判定说明\"}}。"
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pid": {"type": "integer", "minimum": 1, "description": "指定进程号：返回其活跃判定（active=CPU 在动 / idle=CPU 未动 / exited=已退出不存在）"},
                "name": {"type": "string", "description": "(可选)按进程名精确过滤（如 python.exe、real-server.exe）"}
            }
        })
    }
    async fn run(&self, raw: Value) -> Result<String, String> {
        let args = validate(&self.input_schema(), &raw).map_err(|e| err_text(&e))?;
        let pid_q = args.get("pid").and_then(|v| v.as_i64());
        let name_q = args
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_lowercase);
        // 探针：双采样 CPU（700ms 间隔）判活跃 + Win32_Process 取命令行（识别"哪个是哪个"）
        const PROBE: &str = r#"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$s1 = Get-Process | Select-Object Id, ProcessName, CPU, @{n='WS';e={[int]($_.WorkingSet64/1MB)}}
Start-Sleep -Milliseconds 700
$s2 = Get-Process | Select-Object Id, ProcessName, CPU, @{n='WS';e={[int]($_.WorkingSet64/1MB)}}
$cm = @{}
try { Get-CimInstance Win32_Process | ForEach-Object { $cm[[int]$_.ProcessId] = [string]$_.CommandLine } } catch {}
$out = @()
foreach ($p in $s2) {
    $p1 = $s1 | Where-Object { $_.Id -eq $p.Id }
    $d = 0.0
    if ($p1) { $d = [double]$p.CPU - [double]$p1.CPU; if ($d -lt 0) { $d = 0.0 } }
    $out += [pscustomobject]@{
        pid = [int]$p.Id
        name = [string]$p.ProcessName
        cpu_delta = [math]::Round([double]$d, 2)
        mem_mb = [int]$p.WS
        cmd = [string]($cm[[int]$p.Id])
    }
}
$out | Sort-Object mem_mb -Descending | ConvertTo-Json -Compress
"#;
        let tmp = std::env::temp_dir().join("real_proc_status_probe.ps1");
        // BOM 前缀：按 BOM 识别 UTF-8
        let mut body = String::from("\u{feff}");
        body.push_str(PROBE);
        if std::fs::write(&tmp, body.as_bytes()).is_err() {
            return Err("写入进程探针脚本失败".to_string());
        }
        let out = tokio::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"])
            // CREATE_NO_WINDOW：探针不弹 CMD 窗口；-NonInteractive 保证异常时直接退出
            .creation_flags(0x0800_0000)
            .arg(&tmp)
            .output()
            .await;
        let _ = std::fs::remove_file(&tmp);
        let bytes = match out {
            Ok(o) => o.stdout,
            Err(e) => return Err(format!("进程探针执行失败: {e}")),
        };
        let text = String::from_utf8_lossy(&bytes).trim().to_string();
        let rows: Vec<Value> = if text.is_empty() {
            Vec::new()
        } else {
            match serde_json::from_str::<Value>(&text) {
                Ok(Value::Array(a)) => a,
                Ok(v @ Value::Object(_)) => vec![v],
                _ => Vec::new(),
            }
        };
        // 过滤：pid / name；pid<=4（System Idle/System）不进入结果
        let mut matched: Vec<Value> = rows
            .into_iter()
            .filter(|r| {
                let ok_pid = pid_q
                    .map(|q| r.get("pid").and_then(|v| v.as_i64()) == Some(q))
                    .unwrap_or(true);
                let ok_name = match &name_q {
                    Some(q) => r
                        .get("name")
                        .and_then(|v| v.as_str())
                        .map(|n| n.to_lowercase() == *q)
                        .unwrap_or(false),
                    None => true,
                };
                ok_pid && ok_name
            })
            .filter(|r| !r.get("pid").and_then(|v| v.as_i64()).map(|p| p <= 4).unwrap_or(false))
            .collect();
        for r in matched.iter_mut() {
            if let Some(c) = r.get_mut("cmd") {
                let s = c.as_str().unwrap_or("");
                let t: String = s.chars().take(150).collect();
                *c = json!(t);
            }
            let delta = r.get("cpu_delta").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let st = if delta > 0.0 { "active" } else { "idle" };
            r["state"] = json!(st);
        }
        // 判定说明
        if pid_q.is_none() && name_q.is_none() && matched.len() > 60 {
            matched.truncate(60);
        }
        let note = if let Some(q) = pid_q {
            let hit = matched
                .iter()
                .find(|r| r.get("pid").and_then(|v| v.as_i64()) == Some(q));
            match hit {
                None => format!("进程 {q} 不存在或已退出（exited）——长任务提前消失=异常终止，查引擎输出/日志定位"),
                Some(r) => {
                    let st = r.get("state").and_then(|v| v.as_str()).unwrap_or("idle");
                    if st == "active" {
                        format!("进程 {q} 存活且在干活（CPU 在动）——正常推进，等它自然退出")
                    } else {
                        format!("进程 {q} 存活但 CPU 未动（idle）——可能挂起/等待 I/O，查日志确认；无进展再考虑终止")
                    }
                }
            }
        } else if matched.is_empty() {
            "无匹配进程（已退出或名字不对）".to_string()
        } else {
            let act = matched
                .iter()
                .filter(|r| r.get("state").and_then(|v| v.as_str()) == Some("active"))
                .count();
            format!(
                "共 {} 个匹配进程，其中 {} 个 CPU 在动（active）、{} 个 idle（按内存降序前 60 展示）",
                matched.len(),
                act,
                matched.len() - act
            )
        };
        Ok(json!({
            "ok": true, "kind": "proc_status_result",
            "data": {"observed": matched, "note": note}
        }).to_string())
    }
}

#[cfg(test)]
#[path = "proc_status_tests.rs"]
mod proc_status_tests;
