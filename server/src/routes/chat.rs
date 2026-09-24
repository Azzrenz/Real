//! 对话入口：启动编排（每会话独立 tokio task，坑位 C6）

use crate::sse::EV_CANCELLED;
use crate::db::repos;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Debug, Deserialize)]
pub struct ChatReq {
    pub content: String,
    /// （移植拖拽附件）：{name, data_url} 列表（图片 → 视觉模型）
    #[serde(default)]
    pub attachments: Option<Vec<AttachmentInput>>,
}

#[derive(Debug, Deserialize)]
pub struct AttachmentInput {
    pub name: String,
    /// data:image/png;base64,xxx 或纯 base64
    pub data_url: String,
}

/// （入口脏活前置·控制字符清洗）：用户消息里的 C0 控制字符（\x00-\x1f，
pub(crate) fn sanitize_user_input(input: &str) -> String {
    input
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect()
}

pub(crate) fn compress_image_large(bytes: &[u8]) -> Option<Vec<u8>> {
    let img = image::load_from_memory(bytes).ok()?;
    let (w, h) = (img.width(), img.height());
    let max_side = w.max(h);
    let img = if max_side > 1280 {
        let scale = 1280.0 / max_side as f32;
        img.resize(
            ((w as f32) * scale).round().max(1.0) as u32,
            ((h as f32) * scale).round().max(1.0) as u32,
            image::imageops::FilterType::Lanczos3,
        )
    } else {
        img
    };
    let rgb = img.to_rgb8();
    let mut buf = Vec::new();
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 85);
    enc.encode_image(&rgb).ok()?;
    if buf.len() >= bytes.len() {
        return None;
    }
    Some(buf)
}

pub(crate) fn sanitize_attachment_name(raw: &str, index: usize) -> String {
    let mut cleaned: String = raw
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect();
    // 路径穿越片段 `..`（如 "..\..\evil.md"→".._.._evil.md" 仍有 ..）→ 点也清洗，
    let mut out = String::with_capacity(cleaned.len());
    let chars: Vec<char> = cleaned.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '.' {
            let mut j = i;
            while j < chars.len() && chars[j] == '.' {
                j += 1;
            }
            let dots = j - i;
            if dots >= 2 {
                for _ in 0..dots {
                    out.push('_');
                }
            } else {
                out.push('.');
            }
            i = j;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    cleaned = out;
    // 清洗后只剩下划线/空白/点（原始名全非法）→ fallback attachment_N（防空名/歧义名）
    let meaningful = cleaned
        .chars()
        .any(|c| c != '_' && !c.is_whitespace() && c != '.');
    if !meaningful {
        format!("attachment_{index}")
    } else {
        cleaned
    }
}

/// 位图扩展名判定（**不含 `svg`** —— 它是文本 XML，见 `svg_prompt`）。
fn is_raster_image(ext: &str) -> bool {
    matches!(ext, "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp")
}

/// SVG 附件的注入文案：**明说它是文本、可以 read**，并把源码直接交给模型。
fn svg_prompt(text: &str, raw_name: &str, path: &str) -> String {
    const MAX: usize = 6000;
    let max = MAX;
    let total = text.chars().count();
    let head: String = text.chars().take(MAX).collect();
    let more = if total > MAX {
        format!("（源码共 {total} 字符，下方为前 {max} —— **需要看全时用 read 读 `{path}`**）")
    } else {
        String::new()
    };
    format!(
        "📎 {raw_name}（**SVG 矢量图源码**，纯文本 XML，已保存：{path}）——\
这是**文本**，不是像素图：图形由下面的 XML 描述，读懂它就等于看见了图（形状/文字/坐标/颜色都在里面）。\
**可以也应该用 read 读它的全文**（不存在「读图片出乱码」的问题）。{more}\n{head}"
    )
}

/// 附件的落盘目录：**本任务工作空间**的 `uploads/`。
async fn attachment_dir(state: &AppState, session_id: &str) -> Option<std::path::PathBuf> {
    let ws = repos::get_session_workspace(&state.pool, session_id)
        .await
        .ok()
        .flatten()?;
    if ws.trim().is_empty() {
        return None;
    }
    let created = repos::get_session_created_at(&state.pool, session_id).await.ok()?;
    let key = crate::agent::memory::journal::task_key_from_created_at(&created);
    Some(crate::agent::memory::journal::task_subdir(
        &ws,
        &key,
        crate::agent::memory::journal::ATTACH_SUBDIR,
    ))
}

pub async fn chat(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(req): Json<ChatReq>,
) -> AppResult<Json<Value>> {
    // 先清洗控制字符再 trim（顺序：剥离后 trim 首尾空白）
    let content = sanitize_user_input(&req.content).trim().to_string();
    if content.is_empty() {
        return Err(AppError::Validation("消息内容不能为空".into()));
    }

    // ===== 附件处理（移植 doc_reader）=====
    let mut attach_meta: Vec<Value> = Vec::new();
    let mut image_blocks: Vec<Value> = Vec::new();
    let mut attach_ctx: Vec<String> = Vec::new();
    if let Some(atts) = &req.attachments {
        if !atts.is_empty() {
            // 附件落盘到**本任务工作空间**的 `uploads/`（模型可读回，前端也有据可查）；
            let dir = match attachment_dir(&state, &session_id).await {
                Some(d) => d,
                None => crate::path::data_root::attachments_dir().join(&session_id),
            };
            if let Err(e) = std::fs::create_dir_all(&dir) {
                tracing::warn!(session = %session_id, error = %e, "附件目录创建失败");
            }
            for (i, att) in atts.iter().enumerate().take(10) {
                let raw_name = att.name.trim().to_string();
                let mut data_url = att.data_url.trim().to_string();
                if raw_name.is_empty() || data_url.is_empty() {
                    continue;
                }
                // 文件名清洗（防路径穿越/非法字符）—— （拔种子：中文名被洗成
                let fname = sanitize_attachment_name(&raw_name, i);
                // base64 解码（兼容 data: 前缀）
                let b64 = data_url
                    .split_once(',')
                    .map(|(_, b)| b)
                    .unwrap_or(&data_url);
                let bytes = match crate::tools::base::base64_decode(b64) {
                    Some(b) if !b.is_empty() => b,
                    _ => continue,
                };
                let path = dir.join(&fname);
                if let Err(e) = std::fs::write(&path, &bytes) {
                    tracing::warn!(session = %session_id, error = %e, "附件写盘失败: {fname}");
                    continue;
                }
                let p = path.display().to_string();
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.to_lowercase())
                    .unwrap_or_default();
                let is_image = is_raster_image(&ext);
                if is_image {
                    // 收图即压缩：1.7MP 截图 base64 ≈1.2MB 原样进历史 →
                    if matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "webp" | "bmp")
                        && bytes.len() > 400 * 1024
                        && data_url.starts_with("data:image/")
                    {
                        if let Some(comp) = compress_image_large(&bytes) {
                            if comp.len() < bytes.len() {
                                data_url = format!(
                                    "data:image/jpeg;base64,{}",
                                    crate::tools::base::base64_encode(&comp)
                                );
                                // 落盘同步为压缩版（磁盘不囤原图，前端/模型读的是同一份）
                                let _ = std::fs::write(&path, &comp);
                            }
                        }
                    }
                    if data_url.starts_with("data:image/") {
                        image_blocks.push(json!({
                            "type": "input_image",
                            "image_url": data_url,
                            "detail": "low",
                        }));
                    }
                    // 图片**不走文本上下文**（像素进不了文本），但必须在文本里留一条"别去 read 它"
                    attach_ctx.push(format!(
                        "📎 {raw_name}（图片，已保存：{p}）—— **不要用 read 读图片**：read 是文本读取器，只会把像素解成乱码。要再次查看它，请把它作为附件重新发送。"
                    ));
                    attach_meta.push(json!({ "name": raw_name, "kind": "image", "preview": data_url, "data_url": data_url }));
                } else if ext == "svg" {
                    match crate::tools::doc::read_document(&p) {
                        Ok(text) => attach_ctx.push(svg_prompt(&text, &raw_name, &p)),
                        Err(e) => attach_ctx.push(format!(
                            "📎 {raw_name}（SVG 矢量图，已保存：{p}）—— 按文本读取失败（{e}）。\
它是**文本 XML**，请直接用 read 读它的源码（不要当成像素图去处理）。"
                        )),
                    }
                    attach_meta.push(json!({ "name": raw_name, "kind": "image", "preview": data_url, "data_url": data_url }));
                } else if crate::tools::doc::is_supported_ext(&ext) {
                    // 文档类：读内容注入模型上下文（截断 6000 字符，防上下文爆炸）
                    match crate::tools::doc::read_document(&p) {
                        Ok(text) => {
                            // （附件策略升级）：结构化预览替代纯开头截断——
                            attach_ctx
                                .push(crate::tools::doc::document_preview(&text, &raw_name, &p));
                        }
                        Err(_) => {
                            // 解析失败 → 回退给路径，模型可用 read 工具读
                            attach_ctx.push(format!(
                                "📎 {raw_name}（已保存，路径 {p}；需要时用 read 读取）"
                            ));
                        }
                    }
                    attach_meta.push(json!({ "name": raw_name, "kind": "file", "preview": null, "data_url": data_url }));
                } else {
                    // 其余二进制：给路径，模型自行决定读
                    attach_ctx.push(format!(
                        "📎 {raw_name}（已保存，路径 {p}；需要时用 read 读取）"
                    ));
                    attach_meta.push(json!({ "name": raw_name, "kind": "file", "preview": null, "data_url": data_url }));
                }
            }
        }
    }

    // 会话存在性校验
    let session = repos::get_session(&state.pool, &session_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("会话 {session_id} 不存在")))?;
    if session.status == "executing"
        || session.status == "planning"
        || session.status == "solving"
        || session.status == "reflecting"
    {
        return Err(AppError::Validation(format!(
            "会话 {session_id} 正在运行中，请等待完成或先取消"
        )));
    }

    // 用户消息落库（item_json 原样，供历史重建）。
    let vision_model_name = state.effective_model(&session.id).await;
    let is_vision_model = crate::model::catalog::model_vision(&vision_model_name);
    let mut blocks = vec![json!({"type": "input_text", "text": content.clone()})];
    if is_vision_model {
        for b in &image_blocks {
            blocks.push(b.clone());
        }
    } else if !image_blocks.is_empty() {
        attach_ctx.push(format!(
            "⚠️ 用户本轮附了 {} 张图片，但当前模型（{}）不支持图片输入——这些图片你看不到，\
也**不要用 read 去读**（read 是文本读取器，只会把像素读成乱码）。\
请明确告诉用户：图片没有被读取，需要切换到支持看图的模型，或请用户用文字描述图中内容。",
            image_blocks.len(),
            vision_model_name
        ));
    }
    let mut item_json = json!({
        "type": "message",
        "role": "user",
        "content": blocks,
    });
    if !attach_meta.is_empty() {
        item_json["attachments"] = json!(attach_meta);
    }
    repos::insert_message(
        &state.pool,
        &session_id,
        "user",
        &content,
        Some(&serde_json::to_string(&item_json).unwrap()),
    )
    .await?;

    // 独立 task 运行编排（会话级隔离）
    let ctx = state.clone();
    let sid = session_id.clone();
    let stale_inj = crate::db::repos::cancel_pending_interjections(&state.pool, &session_id)
        .await
        .unwrap_or(0);
    // （附件策略升级）：文档类附件内容（attach_ctx）拼进模型输入文本，
    let mut agent_content = if attach_ctx.is_empty() {
        content.clone()
    } else {
        format!("{}\n\n{}\n", content, attach_ctx.join("\n\n"))
    };
    if stale_inj > 0 {
        agent_content = format!(
            "（系统提示：上一轮有 {} 条插话因任务当时已结束而未生效，已作废；本轮任务以用户这条新消息为准）\n\n{}",
            stale_inj, agent_content
        );
    }
    // run 身份 + 取消令牌在此创建（编排层只消费）：终态事件由此获得"属于哪一轮"的坐标
    let run = state.open_run(&session_id);
    let run_id = run.run_id.clone();
    let run_task = run.clone();
    tokio::spawn(async move {
        let cancel_token = run_task.cancel.clone();
        let task_run_id = run_task.run_id.clone();
        if let Err(e) = crate::agent::orchestration::run_agent(
            &ctx,
            &sid,
            &agent_content,
            if is_vision_model {
                image_blocks.clone()
            } else {
                Vec::new()
            },
            run_task,
        )
        .await
        {
            tracing::error!(session = %sid, error = %e, "编排运行失败");
            // B4 修复：用户取消时**本次 run 自己的令牌**已置位——取消判据归令牌，
            if cancel_token.is_cancelled() {
                // 审计 A2：取消也留痕——对话滚存 + 中断断点（"接着修"有据可依）
                let _ = crate::agent::memory::store_conversation_roll(
                    &ctx.pool,
                    &sid,
                    &content,
                    "任务被用户取消",
                )
                .await;
                // 断点带真实进度（已执行工具 + 计划步数 + 已确认结论）——"继续"时模型
                let files = crate::db::repos::recall_read_paths(&ctx.pool, &sid, 6)
                    .await
                    .unwrap_or_default();
                let plan_exists = sqlx::query_scalar::<_, i64>(
                    "SELECT COUNT(*) FROM plans WHERE session_id = ?1",
                )
                .bind(&sid)
                .fetch_one(&ctx.pool)
                .await
                .map(|n| n > 0)
                .unwrap_or(false);
                let (round, summary) =
                    crate::db::repos::build_checkpoint_summary(&ctx.pool, &sid).await;
                let _ = crate::db::repos::checkpoint_save(
                    &ctx.pool,
                    &sid,
                    round,
                    &summary,
                    plan_exists,
                    &files.join(","),
                )
                .await;
                // （P0-1 取消闭环）：取消必须发 cancelled 事件——前端取消分支
                let payload = json!({"message": "任务已被取消", "run_id": task_run_id});
                if let Ok(seq) = repos::insert_event(&ctx.pool, &sid, EV_CANCELLED, &payload).await
                {
                    ctx.hub.publish(&sid, seq, EV_CANCELLED, payload);
                }
                return;
            }
            // 审计 A2：错误路径同样写对话滚存 + 断点（不丢轮次脉络，下轮可续）
            let _ = crate::agent::memory::store_conversation_roll(
                &ctx.pool,
                &sid,
                &content,
                &format!(
                    "任务失败: {}",
                    crate::agent::plan::truncate(&e.to_string(), 200)
                ),
            )
            .await;
            let _ = crate::db::repos::checkpoint_save(
                &ctx.pool,
                &sid,
                0,
                &format!(
                    "任务失败: {}",
                    crate::agent::plan::truncate(&e.to_string(), 300)
                ),
                false,
                "",
            )
            .await;
            let _ = repos::update_session_status(&ctx.pool, &sid, "error").await;
            // 双发根治：workflow 层 Err 分支已发 EV_ERROR（{message}），这里
            let _ = e;
        }
    });

    Ok(Json(json!({
        "accepted": true,
        "session_id": session_id,
        "run_id": run_id,
        "message": "编排已启动，请订阅 SSE 事件流查看进度",
    })))
}

pub async fn cancel(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> AppResult<Json<Value>> {
    // 返回被叫停的 run_id（None = 当时没有活跃 run，无事发生）——前端/日志据此对账
    let cancelled = state.cancel_session(&session_id).await;
    Ok(Json(
        json!({"cancelled": cancelled.is_some(), "run_id": cancelled, "session_id": session_id}),
    ))
}

/// 危险操作确认回传（前端 ConfirmModal 提交）
pub async fn confirm(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    body: Json<Value>,
) -> AppResult<Json<Value>> {
    let request_id = body
        .get("request_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let approved = body
        .get("approved")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let _trust_session = body
        .get("trust_session")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    // 选定值：select_workspace 是路径，ask 是选项 label。
    let selected = body
        .get("selected")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from);
    // 补充说明：ask 专用——用户自己写的一句话。他可以不选任何选项、只写它。
    let note = body
        .get("note")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from);
    // （ConfirmGate v2 重建）：把响应写入确认门挂起表——waiting 的
    let _ = state.confirm_gate.resolve(
        &request_id,
        &session_id,
        approved,
        selected.clone(),
        note.clone(),
    );
    let _ = crate::sse::emit(
        &state,
        &session_id,
        "confirm.resolved",
        json!({
            "request_id": request_id,
            "approved": approved,
            "selected": selected,
            "note": note,
        }),
    )
    .await;
    Ok(Json(
        json!({"responded": true, "request_id": request_id, "approved": approved}),
    ))
}

#[cfg(test)]
#[path = "chat_tests.rs"]
mod chat_tests;
