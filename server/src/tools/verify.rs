use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// 一条语言/栈的探测与验证规则（表驱动）。
struct Rule {
    kind: &'static str,
    /// 探测标志文件（相对工程根）；空 = 单文件检查，无需工程
    marker: &'static str,
    /// 适用扩展名
    exts: &'static [&'static str],
    /// 命令模板：{root}=工程根，{file}=改动文件
    cmd: &'static str,
    /// 依赖的外部 CLI（空 = 不需外部工具）
    tool: &'static str,
}

/// 规则表（顺序 = 优先级；新增语言只在这里加一行）
const RULES: &[Rule] = &[
    // ── 编译型 ──
    Rule { kind: "rust", marker: "Cargo.toml", exts: &["rs"], cmd: "cargo check --manifest-path \"{root}/Cargo.toml\"", tool: "cargo" },
    Rule { kind: "cpp", marker: "compile_commands.json", exts: &["c", "cc", "cpp", "cxx", "h", "hpp"], cmd: "", tool: "" },
    Rule { kind: "csharp", marker: ".sln", exts: &["cs"], cmd: "dotnet build --no-restore -v q", tool: "dotnet" },
    Rule { kind: "csharp-project", marker: ".csproj", exts: &["cs"], cmd: "dotnet build --no-restore -v q", tool: "dotnet" },
    Rule { kind: "java-maven", marker: "pom.xml", exts: &["java"], cmd: "mvn -q -o compile", tool: "mvn" },
    Rule { kind: "java-gradle", marker: "build.gradle", exts: &["java"], cmd: "gradle compileJava -q", tool: "gradle" },
    Rule { kind: "go", marker: "go.mod", exts: &["go"], cmd: "go vet ./...", tool: "go" },
    // ── 前端 / 桌面壳（Electron=V8 的 JS、Tauri=前端+src-tauri/Cargo.toml 的 Rust）──
    Rule { kind: "typescript", marker: "tsconfig.json", exts: &["ts", "tsx"], cmd: "npx tsc --noEmit -p \"{root}/tsconfig.json\"", tool: "npx" },
    Rule { kind: "vue", marker: "package.json", exts: &["vue"], cmd: "npx vue-tsc --noEmit", tool: "npx" },
    Rule { kind: "svelte", marker: "package.json", exts: &["svelte"], cmd: "npx svelte-check", tool: "npx" },
    Rule { kind: "javascript", marker: "", exts: &["js", "mjs", "cjs"], cmd: "node --check \"{file}\"", tool: "node" },
    // ── 脚本语言（单文件、只读检查）──
    Rule { kind: "python", marker: "", exts: &["py"], cmd: "python -c \"import sys;compile(open(sys.argv[1],encoding='utf-8').read(),sys.argv[1],'exec')\" \"{file}\"", tool: "python" },
    Rule { kind: "php", marker: "", exts: &["php"], cmd: "php -l \"{file}\"", tool: "php" },
    Rule { kind: "ruby", marker: "", exts: &["rb"], cmd: "ruby -c \"{file}\"", tool: "ruby" },
    Rule { kind: "shell", marker: "", exts: &["sh", "bash"], cmd: "bash -n \"{file}\"", tool: "bash" },
    // JSON 没有"静默校验"的标准库入口（`json.tool` 会把全文打到 stdout，大文件会灌爆上下文），
    Rule { kind: "json", marker: "", exts: &["json"], cmd: "python -c \"import json,sys;json.load(open(sys.argv[1],encoding='utf-8'))\" \"{file}\"", tool: "python" },
];

/// 验证计划
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyPlan {
    pub kind: &'static str,
    pub root: String,
    pub command: String,
}

/// 5 分钟内已自动验证过**同一内容**的文件（防重复烧时间）。
fn recently_verified(file: &str) -> bool {
    static CACHE: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let key = match std::fs::metadata(file) {
        Ok(m) => {
            let mtime = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            format!("{}|{}|{}", file.to_lowercase().replace('\\', "/"), m.len(), mtime)
        }
        Err(_) => return false,
    };
    if let Ok(mut map) = cache.lock() {
        if let Some(t) = map.get(&key) {
            if t.elapsed() < Duration::from_secs(300) {
                return true;
            }
        }
        if map.len() > 512 {
            map.clear();
        }
        map.insert(key, Instant::now());
    }
    false
}

/// 外部 CLI 是否在 PATH（只查路径，不执行）
fn tool_available(bin: &str) -> bool {
    if bin.is_empty() {
        return true;
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path) {
        if dir.join(bin).is_file() {
            return true;
        }
        for ext in ["exe", "cmd", "bat", "ps1"] {
            if dir.join(format!("{bin}.{ext}")).is_file() {
                return true;
            }
        }
    }
    false
}

/// 从改动文件列表产出验证计划；判不出 → None。
pub fn plan_for(changed_files: &[String]) -> Option<VerifyPlan> {
    for f in changed_files {
        let p = f.replace('\\', "/");
        let ext = p.rsplit('.').next().unwrap_or("").to_lowercase();
        // ① 工程**声明式**验证（吸取平台做法：读声明、不猜语言、不堆表）
        if let Some(plan) = project_declared_plan(&p) {
            return Some(plan);
        }
        // ② 用户学习规则（先于内置：用户为某扩展名自定义过就以用户为准）
        if let Some(plan) = user_rule_plan(&p, &ext) {
            return Some(plan);
        }
        // ③ 内置规则表（兜底）
        for rule in RULES {
            if !rule.exts.contains(&ext.as_str()) {
                continue;
            }
            if recently_verified(&p) {
                return None;
            }
            if !rule.tool.is_empty() && !tool_available(rule.tool) {
                continue;
            }
            if rule.kind == "cpp" {
                if let Some((root, cmd)) = cpp_plan(&p) {
                    return Some(VerifyPlan { kind: "cpp", root, command: cmd });
                }
                continue;
            }
            if rule.marker.is_empty() {
                return Some(VerifyPlan {
                    kind: rule.kind,
                    root: String::new(),
                    command: rule.cmd.replace("{file}", &p),
                });
            }
            if let Some(root) = nearest_marker_root(&p, rule.marker) {
                let cmd = rule.cmd.replace("{root}", &root).replace("{file}", &p);
                return Some(VerifyPlan { kind: rule.kind, root, command: cmd });
            }
        }
    }
    None
}

// 学习层（四级阶梯的第 3/4 级）：用户确认过的验证规则落库，下次直接命中

/// 用户规则（一个扩展名一条）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UserRule {
    pub ext: String,
    /// 命令模板：可用 {root} / {file}
    pub cmd: String,
    /// 依赖的 CLI（可空）
    #[serde(default)]
    pub tool: String,
    /// 依据（官方文档 URL 或用户备注）——留痕，便于审计
    #[serde(default)]
    pub source: String,
}

fn user_rules_path() -> PathBuf {
    let base = std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    base.join("real-agent").join("verify_rules.json")
}

/// 读取用户学习规则（文件不存在/损坏 → 空表，不 panic）
pub fn load_user_rules() -> Vec<UserRule> {
    std::fs::read_to_string(user_rules_path())
        .ok()
        .and_then(|t| serde_json::from_str::<Vec<UserRule>>(&t).ok())
        .unwrap_or_default()
}

/// **工程声明式验证**（读声明 > 猜语言 > 堆表）
fn project_declared_plan(file: &str) -> Option<VerifyPlan> {
    let ext = file.rsplit('.').next().unwrap_or("").to_lowercase();
    let root = nearest_marker_root(file, ".real/verify.json")
        .or_else(|| nearest_marker_root(file, "verify.json"))?;
    let dotted = std::path::Path::new(&root).join(".real/verify.json");
    let path = if dotted.is_file() {
        dotted
    } else {
        std::path::Path::new(&root).join("verify.json")
    };
    let txt = std::fs::read_to_string(&path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&txt).ok()?;
    for r in v.get("rules")?.as_array()? {
        let rext = r.get("ext")?.as_str()?.to_lowercase();
        if rext != ext {
            continue;
        }
        let cmd = r.get("cmd")?.as_str()?;
        let tool = r.get("tool").and_then(|t| t.as_str()).unwrap_or("");
        if !tool.is_empty() && !tool_available(tool) {
            return None;
        }
        if !command_is_safe(cmd) {
            tracing::warn!(file = %file, cmd = %cmd, "工程声明的验证命令未过安全门，已忽略");
            return None;
        }
        return Some(VerifyPlan {
            kind: "declared",
            root: root.clone(),
            command: cmd.replace("{root}", &root).replace("{file}", file),
        });
    }
    None
}

/// **命令安全门**：自动（或来自声明的）验证命令必须先过这道门才可能被执行。
pub fn command_is_safe(cmd: &str) -> bool {
    let low = cmd.to_lowercase();
    const BANNED: [&str; 20] = [
        "rm ", "rm-", "del ", "erase ", "rmdir", "rd ", "format ", "mkfs", "shutdown",
        "reg delete", "curl", "wget", "invoke-webrequest", "iwr ", "git push", "git reset",
        "npm install", "npm i ", "pip install", "cargo install",
    ];
    if BANNED.iter().any(|b| low.contains(b)) {
        return false;
    }
    const PY_BANNED: [&str; 11] = [
        "remove(", "unlink(", "rmtree", "os.system", "subprocess", "shutil", "os.rename",
        "write(", "requests", "urllib", "socket",
    ];
    // 只读 open 放行；写模式（'w'/'a'/'x'）一律拒
    const PY_WRITE_MODES: [&str; 6] = [",'w'", ",\"w\"", ",'a'", ",\"a\"", ",'x'", ",\"x\""];
    let first = low
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches('"')
        .trim_end_matches(".exe")
        .to_string();
    match first.as_str() {
        "cargo" => low.contains("check") || low.contains("clippy"),
        "tsc" => low.contains("--noemit"),
        "npx" => low.contains("tsc") || low.contains("vue-tsc") || low.contains("svelte-check"),
        "node" => low.contains("--check"),
        // python 检查器只允许**只读**形态：必须同时满足「`-c` 内联」与「出现只读校验入口」。
        "python" | "python3" => {
            let c_form = low.contains(" -c ");
            let readonly_entry = low.contains("compile(") || low.contains("json.load(");
            c_form
                && readonly_entry
                && !PY_BANNED.iter().any(|k| low.contains(k))
                && !PY_WRITE_MODES.iter().any(|k| low.contains(k))
        }
        "cmake" => low.contains("--build"),
        "go" => low.contains("vet") || low.contains("build"),
        "dotnet" => low.contains("build"),
        "mvn" => low.contains("compile"),
        "gradle" => low.contains("compile"),
        "php" => low.contains("-l"),
        "ruby" => low.contains("-c"),
        "bash" => low.contains("-n"),
        "g++" | "gcc" | "clang++" | "clang" | "cl" => low.contains("-fsyntax-only"),
        _ => false,
    }
}

pub fn declared_reject_hint(changed_files: &[String]) -> Option<String> {
    for f in changed_files {
        let p = f.replace('\\', "/");
        let ext = p.rsplit('.').next().unwrap_or("").to_lowercase();
        let root = nearest_marker_root(&p, ".real/verify.json")
            .or_else(|| nearest_marker_root(&p, "verify.json"))?;
        let dotted = std::path::Path::new(&root).join(".real/verify.json");
        let path = if dotted.is_file() {
            dotted
        } else {
            std::path::Path::new(&root).join("verify.json")
        };
        let txt = std::fs::read_to_string(&path).ok()?;
        let v: serde_json::Value = serde_json::from_str(&txt).ok()?;
        for r in v.get("rules")?.as_array()? {
            if r.get("ext")?.as_str()?.to_lowercase() != ext {
                continue;
            }
            let cmd = r.get("cmd").and_then(|c| c.as_str()).unwrap_or("");
            let tool = r.get("tool").and_then(|t| t.as_str()).unwrap_or("");
            if !tool.is_empty() && !tool_available(tool) {
                return Some(format!(
                    "〔未自动验证〕工程声明的验证需 `{tool}`，但本机 PATH 里找不到它——本轮未验证该改动。"
                ));
            }
            if !command_is_safe(cmd) {
                return Some(format!(
                    "〔未自动验证〕工程声明的验证命令未过安全门（只允许只读检查类命令），本轮未验证该改动。原命令：{cmd}"
                ));
            }
        }
    }
    None
}

/// 命中用户规则 → 产出计划
fn user_rule_plan(file: &str, ext: &str) -> Option<VerifyPlan> {
    for r in load_user_rules() {
        if r.ext.to_lowercase() != ext {
            continue;
        }
        if !r.tool.is_empty() && !tool_available(&r.tool) {
            return None;
        }
        if !command_is_safe(&r.cmd) {
            tracing::warn!(ext = %ext, cmd = %r.cmd, "用户验证规则未通过安全门，已忽略");
            return None;
        }
        // 找工程根（沿用内置的逐级上溯）；找不到则按单文件处理
        let root = nearest_marker_root(file, "Cargo.toml")
            .or_else(|| nearest_marker_root(file, "package.json"))
            .or_else(|| nearest_marker_root(file, "CMakeLists.txt"))
            .unwrap_or_default();
        let cmd = r.cmd.replace("{root}", &root).replace("{file}", file);
        return Some(VerifyPlan {
            kind: "user-rule",
            root,
            command: cmd,
        });
    }
    None
}

/// 逐级向上找含标志文件的目录（最多 8 层）。marker 形如 ".sln"/".csproj" 时按扩展名匹配。
fn nearest_marker_root(file: &str, marker: &str) -> Option<String> {
    let ext_marker = marker.starts_with('.')
        && marker.len() > 1
        && marker[1..].chars().all(|c| c.is_ascii_alphabetic());
    let want = marker.trim_start_matches('.').to_lowercase();
    let mut cur: PathBuf = Path::new(file).parent()?.to_path_buf();
    for _ in 0..8 {
        let hit = if ext_marker {
            std::fs::read_dir(&cur)
                .ok()
                .map(|rd| {
                    rd.flatten().any(|e| {
                        e.path()
                            .extension()
                            .map(|x| x.to_string_lossy().to_lowercase() == want)
                            .unwrap_or(false)
                    })
                })
                .unwrap_or(false)
        } else {
            cur.join(marker).is_file()
        };
        if hit {
            return Some(cur.display().to_string().replace('\\', "/"));
        }
        match cur.parent() {
            Some(p) => cur = p.to_path_buf(),
            None => break,
        }
    }
    None
}

/// C/C++ 三级链：编译数据库 → CMake 增量构建 → 不验
fn cpp_plan(file: &str) -> Option<(String, String)> {
    if let Some((root, cmd)) = cpp_from_compdb(file) {
        return Some((root, cmd));
    }
    cpp_incremental_build(file)
}

fn cpp_from_compdb(file: &str) -> Option<(String, String)> {
    let root = nearest_marker_root(file, "compile_commands.json")?;
    let txt = std::fs::read_to_string(Path::new(&root).join("compile_commands.json")).ok()?;
    let arr: serde_json::Value = serde_json::from_str(&txt).ok()?;
    let base = file.replace('\\', "/").to_lowercase();
    let base = base.rsplit('/').next()?.to_string();
    for entry in arr.as_array()? {
        let f = entry.get("file")?.as_str()?;
        if f.replace('\\', "/").to_lowercase().ends_with(&base) {
            let cmd = entry.get("command").and_then(|c| c.as_str()).unwrap_or("");
            if !cmd.is_empty() {
                let mut parts: Vec<String> = cmd
                    .split_whitespace()
                    .filter(|t| !t.starts_with("-o") && *t != "-o" && !t.ends_with(".o"))
                    .map(|t| t.to_string())
                    .collect();
                parts.push("-fsyntax-only".into());
                return Some((root, parts.join(" ")));
            }
        }
    }
    None
}

fn cpp_incremental_build(file: &str) -> Option<(String, String)> {
    let proj_root = nearest_marker_root(file, "CMakeLists.txt")?;
    for cand in ["build", "out/build", "cmake-build-debug", "cmake-build-release"] {
        let dir = Path::new(&proj_root).join(cand);
        if dir.join("CMakeCache.txt").is_file() {
            let d = dir.display().to_string().replace('\\', "/");
            return Some((proj_root, format!("cmake --build \"{d}\" -j 4")));
        }
    }
    None
}

#[cfg(test)]
#[path = "verify_tests.rs"]
mod verify_tests;
