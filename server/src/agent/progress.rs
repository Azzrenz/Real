//! 任务进度上报 —— 让右侧栏「进度」页看见 **agent 自己**在跑什么。
//!
//! ## 为什么是文件，不是接口
//!
//! 与面板的设计前提**完全一致**（见 `ProgressPanel.tsx` 顶部说明）：长活是**别的进程**
//! 在跑，一份 JSON 落在盘上，谁写都行、谁读都行。agent 自己就是一个"长活"——它跑的时候
//! 把进度写进**同一份快照**，于是面板**零改动**就能显示；Real 服务挂了也不影响读。
//!
//! ## 契约（与 `progress.py` / `ProgressPanel.tsx` 逐字对应）
//!
//! - 快照：`<工作区>/.real/progress.json`
//! - 索引：`%APPDATA%/real-agent/progress_index.json`（登记本目录，面板据此自动发现）
//!
//! ## 并发安全
//!
//! 同一目录下可能**同时有别的活在写**同一份快照（渲染 / 转写 / 批处理）⇒ 一律
//! **读-改-写**，只动自己那条 tag，其余原样带过。写盘走"临时文件 + rename"，
//! 避免面板 2 秒轮询读到半截 JSON。
//!
//! ## 为什么进度是"不定态"
//!
//! agent 任务的**总步数不可知**（模型自主决定跑几轮），假装给个百分比等于骗人。
//! 面板为这种情况备了不定态条纹（`.prog2-mini i.indet`）——这里就报 `n_total = 0`、
//! `pct = null`，让面板画成"在跑、还不知道多久"。**阶段与当前动作**则由 `what` 如实给出。

use std::io;
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

/// 快照在工作区下的约定路径（与 `ProgressPanel.tsx` 的 `REL_PATH = '.real/progress.json'` 对应）。
///
/// 拆成两段是为了能直接 `join` —— 不在这里手拼斜杠，免得哪天路径风格各写一半。
pub const REL_DIR: &str = ".real";
pub const REL_FILE: &str = "progress.json";

/// 快照里同时保留的最大主条数 —— 只留最近的，免得一份文件被历史任务堆肿。
const MAX_ITEMS: usize = 8;

/// 已完结条目在快照里的**留存时长**（秒）：留一会儿让用户看见结果，之后让新任务挤掉。
const FINISHED_KEEP_SECS: i64 = 3600;

/// 一条活的 tag：面板只把 `title` 画给人看，tag 是它认"是不是同一条活"的键。
///
/// 用会话 id 前 8 位 —— 同目录下同一个会话重跑即**覆盖**自己那条，不会越堆越多。
pub fn tag_for(session_id: &str) -> String {
    let head: String = session_id.chars().take(8).collect();
    format!("agent:{head}")
}

/// 一次上报的内容。
pub struct Patch<'a> {
    /// 任务目标（一句话）。**只在起手那次传 Some**，后续沿用快照里的值——
    /// 免得每轮都把长目标重复写一遍盘。
    pub title: Option<&'a str>,
    /// `running` / `done` / `failed`（面板 `STATE_ZH` 认这三个词）。
    pub state: &'a str,
    /// 已跑轮次。
    pub n_done: usize,
    /// 当前在干什么（一句话，面板显示在卡片右下）。
    pub what: &'a str,
}

/// 上报一条活的当前状态。**失败绝不影响主流程**——这是旁路信息，丢了顶多少画一条。
pub fn report(workspace: &Path, tag: &str, patch: Patch<'_>) {
    if let Err(e) = report_inner(workspace, tag, &patch) {
        tracing::debug!(error = %e, tag, "进度上报失败（旁路，不影响任务）");
    }
}

fn report_inner(workspace: &Path, tag: &str, patch: &Patch<'_>) -> io::Result<()> {
    let dir = workspace.join(REL_DIR);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(REL_FILE);

    // ── 读：把现有内容原样取出来（别的活的条目一条都不能丢）──
    let mut root = read_object(&path);
    let mut items = take_items(&mut root);

    let now = unix_now();
    // 自己那条的"起点"沿用旧值 —— 秒表要连着重跑的时间算，不能每轮归零。
    let started = items
        .iter()
        .find(|it| it.get("tag").and_then(Value::as_str) == Some(tag))
        .and_then(|it| it.get("started").and_then(Value::as_i64))
        .unwrap_or(now);
    let title = patch
        .title
        .map(str::to_string)
        .or_else(|| {
            items
                .iter()
                .find(|it| it.get("tag").and_then(Value::as_str) == Some(tag))
                .and_then(|it| it.get("title").and_then(Value::as_str))
                .map(str::to_string)
        })
        .unwrap_or_else(|| "任务".to_string());

    let mine = json!({
        "tag": tag,
        "title": title,
        "state": patch.state,
        // 总步数不可知 ⇒ 交不定态给面板画（见模块头说明）。
        "n_done": patch.n_done,
        "n_total": 0,
        "pct": Value::Null,
        "what": patch.what,
        "pid": std::process::id(),
        // 只有 running 才算"还活着"；面板据此把点画成蓝色还是灰/黄。
        "alive": patch.state == "running",
        "started": started,
        "beat": now,
        "stuck": false,
        "state_note": Value::Null,
    });

    // ── 改：同 tag 就地替换，没有则插到最前 ──
    match items
        .iter()
        .position(|it| it.get("tag").and_then(Value::as_str) == Some(tag))
    {
        Some(i) => items[i] = mine,
        None => items.insert(0, mine),
    }

    // ── 修剪：完结太久的不再占位，再按条数封顶 ──
    items.retain(|it| {
        let finished = it.get("state").and_then(Value::as_str) != Some("running");
        let beat = it.get("beat").and_then(Value::as_i64).unwrap_or(now);
        !(finished && now - beat > FINISHED_KEEP_SECS)
    });
    items.truncate(MAX_ITEMS);

    // 顶层汇总（面板 `Snapshot` 的三个必填数字位）：主条等权，与面板 `overallOf` 同口径。
    let total = items.len() as i64;
    let done = items
        .iter()
        .filter(|it| it.get("state").and_then(Value::as_str) != Some("running"))
        .count() as i64;
    root.insert("time".into(), json!(now));
    root.insert("items".into(), Value::Array(items));
    root.insert("done".into(), json!(done));
    root.insert("total".into(), json!(total));
    root.entry("total_mem_mb".to_string()).or_insert(json!(0));

    write_atomic(&path, &Value::Object(root))?;
    // 登记索引：面板据此**自动发现**这份快照，不用用户手填路径。
    register_index(&path);
    Ok(())
}

/// 收尾：把这条活标成 `done` / `failed` 并留一份"产出"说明（可选）。
pub fn finish(workspace: &Path, tag: &str, ok: bool, n_done: usize, what: &str) {
    report(
        workspace,
        tag,
        Patch {
            title: None,
            state: if ok { "done" } else { "failed" },
            n_done,
            what,
        },
    );
}

/// 读 JSON 对象；文件不在 / 不是对象 / 坏了 —— 一律当空的重来（宁可少画，不能崩）。
fn read_object(path: &Path) -> Map<String, Value> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

/// 取主条数组。兼容旧键名 `episodes`（`watch_render.py` 早期输出）。
fn take_items(root: &mut Map<String, Value>) -> Vec<Value> {
    let by_new = root.remove("items").and_then(|v| v.as_array().cloned());
    match by_new {
        Some(v) if !v.is_empty() => v,
        _ => root
            .remove("episodes")
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default(),
    }
}

/// 登记到全局索引（`%APPDATA%/real-agent/progress_index.json`）。
///
/// 索引形状与 `progress_index` 命令的读取端逐字对应：`{ "<快照绝对路径>": {dir,snapshot,mtime} }`。
/// 失败只记 debug —— 索引丢了，用户还能手填路径；不该因此打断任务。
fn register_index(snapshot: &Path) {
    let Some(p) = index_path() else { return };
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut obj = read_object(&p);
    let key = snapshot.to_string_lossy().to_string();
    obj.insert(
        key.clone(),
        json!({
            "dir": snapshot
                .parent()
                .map(|d| d.to_string_lossy().to_string())
                .unwrap_or_default(),
            "snapshot": key,
            "mtime": unix_now(),
        }),
    );
    // 索引也会长：只留最近 20 个目录，老的自动退场。
    if obj.len() > 20 {
        let mut rows: Vec<(String, i64)> = obj
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    v.get("mtime").and_then(Value::as_i64).unwrap_or(0),
                )
            })
            .collect();
        rows.sort_by_key(|(_, m)| std::cmp::Reverse(*m));
        for (k, _) in rows.into_iter().skip(20) {
            obj.remove(&k);
        }
    }
    if let Err(e) = write_atomic(&p, &Value::Object(obj)) {
        tracing::debug!(error = %e, "进度索引登记失败（旁路）");
    }
}

/// 索引文件位置 —— 与 `lib.rs` 的 `progress_index` 命令**同一处**，不许各拼一份。
fn index_path() -> Option<PathBuf> {
    // 调试与测试可用 `REAL_PROGRESS_INDEX` 指到别处 —— 免得验证时写进真实用户目录。
    if let Ok(p) = std::env::var("REAL_PROGRESS_INDEX") {
        if !p.trim().is_empty() {
            return Some(PathBuf::from(p));
        }
    }
    std::env::var("APPDATA")
        .ok()
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(".config"))
        })
        .map(|b| b.join("real-agent").join("progress_index.json"))
}

/// 原子写：先落临时文件再改名。
///
/// Windows 上 `rename` **目标已存在会失败**（与 Unix 不同），故先删旧的；
/// 删与改名之间若被占用而失败，退回直接覆写 —— 面板 2 秒轮询容忍一次半截读，
/// 但绝不能因此**丢掉整份进度**。
fn write_atomic(path: &Path, v: &Value) -> io::Result<()> {
    let txt = serde_json::to_string(v)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, txt.as_bytes())?;
    let _ = std::fs::remove_file(path);
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(_) => std::fs::write(path, txt.as_bytes()),
    }
}

/// Unix 秒。面板全程用秒（`ago()` / `beat` / `started`），不是毫秒。
fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 快照路径是**面板契约**的一部分：改了就再也看不见进度了。
    #[test]
    fn snapshot_path_is_the_panel_contract() {
        assert_eq!(
            format!("{REL_DIR}/{REL_FILE}"),
            ".real/progress.json",
            "与 ProgressPanel.tsx 的 REL_PATH 必须逐字一致"
        );
    }

    /// 索引登记：面板靠它**自动发现**快照，不用用户手填路径。
    ///
    /// 用 `REAL_PROGRESS_INDEX` 把索引指进沙盒，**不碰真实用户目录**。
    /// 键必须是快照的绝对路径、`dir` 是它所在的 `.real` —— 与 `progress_index` 命令的读取端对齐。
    #[test]
    fn index_registration_points_the_panel_at_the_snapshot() {
        let ws = sandbox("index");
        let idx = ws.join("idx.json");
        std::env::set_var("REAL_PROGRESS_INDEX", &idx);

        report(
            &ws,
            &tag_for("s3"),
            Patch { title: Some("t"), state: "running", n_done: 1, what: "跑" },
        );
        std::env::remove_var("REAL_PROGRESS_INDEX");

        let txt = std::fs::read_to_string(&idx).expect("索引应被写出");
        let obj: Value = serde_json::from_str(&txt).unwrap();
        let snap_path = ws.join(REL_DIR).join(REL_FILE);
        let key = snap_path.to_string_lossy().to_string();
        let row = obj.get(&key).expect("快照路径必须是索引的键");
        assert_eq!(row["snapshot"], key);
        assert_eq!(row["dir"], ws.join(REL_DIR).to_string_lossy().to_string());
        assert!(row["mtime"].as_i64().unwrap_or(0) > 0, "mtime 供面板排序");
    }

    fn sandbox(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("real-progress-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn read_snapshot(ws: &Path) -> Value {
        let txt = std::fs::read_to_string(ws.join(".real").join("progress.json")).unwrap();
        serde_json::from_str(&txt).unwrap()
    }

    /// 起手 → 每轮 → 收尾：**一条活自始至终只占一条主条**，且字段按契约落盘。
    #[test]
    fn one_task_occupies_exactly_one_item() {
        let ws = sandbox("one-item");
        let tag = tag_for("abcdef1234567890");
        report(
            &ws,
            &tag,
            Patch { title: Some("把 X 改成 Y"), state: "running", n_done: 0, what: "起手" },
        );
        report(
            &ws,
            &tag,
            Patch { title: None, state: "running", n_done: 3, what: "正在编辑 a.rs" },
        );
        finish(&ws, &tag, true, 4, "完成");

        let snap = read_snapshot(&ws);
        let items = snap["items"].as_array().unwrap();
        assert_eq!(items.len(), 1, "同一条活必须就地替换，不能越写越多");
        let it = &items[0];
        assert_eq!(it["tag"], tag);
        assert_eq!(it["state"], "done");
        assert_eq!(it["n_done"], 4);
        // 总步数不可知 ⇒ 必须是不定态，而不是编一个百分比出来
        assert_eq!(it["n_total"], 0);
        assert!(it["pct"].is_null());
        assert_eq!(it["alive"], false);
        assert_eq!(it["title"], "把 X 改成 Y", "title 由起手那次写入，后续沿用");
        assert_eq!(snap["done"], 1);
        assert_eq!(snap["total"], 1);
    }

    /// **别的活的条目一条都不能丢** —— agent 写盘时只能动自己那条。
    #[test]
    fn other_actors_items_survive_untouched() {
        let ws = sandbox("coexist");
        std::fs::create_dir_all(ws.join(".real")).unwrap();
        let preexisting = json!({
            "time": 1,
            "items": [{ "tag": "render:abc", "title": "渲染", "state": "running", "n_done": 5, "n_total": 10 }],
            "done": 0, "total": 1, "total_mem_mb": 42
        });
        std::fs::write(
            ws.join(".real").join("progress.json"),
            serde_json::to_string(&preexisting).unwrap(),
        )
        .unwrap();

        report(
            &ws,
            &tag_for("session-xyz"),
            Patch { title: Some("我的任务"), state: "running", n_done: 1, what: "读码" },
        );

        let snap = read_snapshot(&ws);
        let items = snap["items"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert!(
            items.iter().any(|i| i["tag"] == "render:abc" && i["n_done"] == 5),
            "外部的活必须原样保留"
        );
        assert!(
            items.iter().any(|i| i["tag"] == tag_for("session-xyz")),
            "自己的活也要在"
        );
        // total_mem_mb 是别人写的字段，不该被我们抹掉
        assert_eq!(snap["total_mem_mb"], 42);
    }

    /// `n_total = 0` + `pct = null` 是**有意为之**：面板据此画不定态条纹。
    #[test]
    fn running_state_is_indeterminate_not_a_fake_percentage() {
        let ws = sandbox("indet");
        report(
            &ws,
            &tag_for("s1"),
            Patch { title: Some("t"), state: "running", n_done: 7, what: "验证" },
        );
        let snap = read_snapshot(&ws);
        let it = &snap["items"][0];
        assert_eq!(it["alive"], true);
        assert_eq!(it["n_total"], 0);
        assert!(it["pct"].is_null(), "不许编造百分比");
    }

    /// 坏文件 / 空文件都要能起来：宁可少画，不能崩。
    #[test]
    fn corrupted_snapshot_is_rebuilt_not_fatal() {
        let ws = sandbox("corrupt");
        std::fs::create_dir_all(ws.join(".real")).unwrap();
        std::fs::write(ws.join(".real").join("progress.json"), b"{ not json").unwrap();
        report(
            &ws,
            &tag_for("s2"),
            Patch { title: Some("t"), state: "running", n_done: 0, what: "起手" },
        );
        let snap = read_snapshot(&ws);
        assert_eq!(snap["items"].as_array().unwrap().len(), 1);
    }

    /// 标签只用会话前 8 位：同一会话重跑**覆盖**自己那条，不堆积。
    #[test]
    fn repeated_runs_under_same_session_overwrite() {
        let ws = sandbox("rerun");
        let tag = tag_for("same-session-id-0001");
        for _ in 0..5 {
            report(
                &ws,
                &tag,
                Patch { title: Some("t"), state: "running", n_done: 0, what: "起手" },
            );
        }
        assert_eq!(read_snapshot(&ws)["items"].as_array().unwrap().len(), 1);
    }
}
