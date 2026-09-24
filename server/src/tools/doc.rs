//! 文档读取模块（移植 doc_reader）

use std::io::Read;
use std::path::Path;

/// 读取文档内容，根据扩展名自动选择解析方式
pub fn read_document(path: &str) -> Result<String, String> {
    let p = Path::new(path);
    if !p.exists() {
        return Err(format!("文件不存在: {path}"));
    }

    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    match ext.as_str() {
        "txt" | "md" | "markdown" | "json" | "yaml" | "yml" | "toml" | "xml" | "svg" | "csv"
        | "log"
        | "rs" | "ts" | "tsx" | "js" | "jsx" | "css" | "html" | "sql" | "py" | "java" | "go"
        | "c" | "cpp" | "h" | "hpp" | "sh" | "bat" | "ps1" => read_text_file(path),
        "docx" => read_docx(path),
        "pptx" => read_pptx(path),
        "xlsx" => read_xlsx(path),
        "pdf" => read_pdf(path),
        _ => read_text_file(path).or_else(|_| Err(format!("不支持的文件格式: .{ext}"))),
    }
}

/// 该扩展名是否由 doc_reader 支持（供调用方判断"文本可读"）
pub fn is_supported_ext(ext: &str) -> bool {
    matches!(
        ext.to_lowercase().as_str(),
        "txt"
            | "md"
            | "markdown"
            | "json"
            | "yaml"
            | "yml"
            | "toml"
            | "xml"
            | "svg"
            | "csv"
            | "log"
            | "rs"
            | "ts"
            | "tsx"
            | "js"
            | "jsx"
            | "css"
            | "html"
            | "sql"
            | "py"
            | "java"
            | "go"
            | "c"
            | "cpp"
            | "h"
            | "hpp"
            | "sh"
            | "bat"
            | "ps1"
            | "docx"
            | "pptx"
            | "xlsx"
            | "pdf"
    )
}

/// 文档结构化预览（附件策略升级·替代纯开头截断 truncate_text）
pub fn document_preview(text: &str, name: &str, path: &str) -> String {
    const FULL_THRESHOLD: usize = 4000;
    const HEAD_CHARS: usize = 2000;
    const TAIL_CHARS: usize = 1000;

    if text.len() <= FULL_THRESHOLD {
        return format!("### 📄 {name}\n```\n{text}\n```");
    }
    // （L1 文档读全量·升级）：大文档不再只给"头 2000 + 尾 1000"——
    let paragraphs: Vec<String> = text
        .split("\n\n")
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    let total_chars = text.chars().count();
    let total_lines = text.lines().count();
    // 首行预览（每段第一行截断 80 字符）
    let para_preview: Vec<(usize, String)> = paragraphs
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let first_line = p.lines().next().unwrap_or("").trim();
            let head: String = first_line.chars().take(80).collect();
            let suffix = if first_line.chars().count() > 80 {
                "…"
            } else {
                ""
            };
            (i, format!("{head}{suffix}"))
        })
        .collect();

    let mut out = format!(
        "### 📄 {name}（原文 {} 字符 / {} 行 / {} 段，较长→段落目录预览）\n",
        total_chars,
        total_lines,
        paragraphs.len()
    );
    // 段落目录（全部列，不截断——这是模型定位全文的地图）
    out.push_str(&format!(
        "【段落目录】共 {} 段，每段首行如下：\n",
        paragraphs.len()
    ));
    for (i, preview) in para_preview.iter().take(60) {
        out.push_str(&format!("- 段{}: {preview}\n", i + 1));
    }
    if paragraphs.len() > 60 {
        out.push_str(&format!("- …（其余 {} 段见全文）\n", paragraphs.len() - 60));
    }
    // 开头（安全字节边界）
    let head_end = byte_boundary(text, HEAD_CHARS.min(text.len()));
    let head = &text[..head_end];
    // 结尾（安全字节边界）
    let tail_start = byte_boundary(text, text.len().saturating_sub(TAIL_CHARS));
    let tail = &text[tail_start..];
    out.push_str(&format!(
        "【开头 {} 字符】\n```\n{head}\n```\n【结尾 {} 字符】\n```\n{tail}\n```\n",
        HEAD_CHARS, TAIL_CHARS
    ));
    out.push_str(&format!(
        "【全文】路径 `{path}`——需要细节时用 read 工具读取。**分段读法**：先读段落目录定位目标段，\
         再用 read 带 start_line/end_line 读该段（每段约 {} 行，文件共 {} 行）。不要凭预览猜内容。\n",
        (total_lines as f64 / paragraphs.len().max(1) as f64).ceil() as usize,
        total_lines
    ));
    out
}

/// 取 ≤limit 的最大字符边界（防切片 panic）
fn byte_boundary(text: &str, limit: usize) -> usize {
    if text.len() <= limit {
        return text.len();
    }
    let mut b = limit;
    while b > 0 && !text.is_char_boundary(b) {
        b -= 1;
    }
    b
}

// 纯文本

fn read_text_file(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("读取文件失败: {e}"))
}

// .docx — Word

fn read_docx(path: &str) -> Result<String, String> {
    let xml_bytes = read_zip_entry(path, "word/document.xml")?;
    extract_xml_text(&xml_bytes, "w:t")
}

// .pptx — PowerPoint

fn read_pptx(path: &str) -> Result<String, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("无法打开文件: {e}"))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("无法解析 .pptx (ZIP): {e}"))?;

    // 收集所有 ppt/slides/slideN.xml 的文本
    let mut slide_texts: Vec<(usize, String)> = Vec::new();
    for i in 0..archive.len() {
        let name = archive
            .name_for_index(i)
            .map(|n| n.to_lowercase().replace('\\', "/"))
            .unwrap_or_default();
        if name.starts_with("ppt/slides/slide") && name.ends_with(".xml") {
            // 提取 slide 序号
            let num: usize = name
                .trim_start_matches("ppt/slides/slide")
                .trim_end_matches(".xml")
                .parse()
                .unwrap_or(0);
            let mut entry = archive
                .by_index(i)
                .map_err(|e| format!("读取 ZIP 条目失败: {e}"))?;
            let mut bytes = Vec::new();
            entry
                .read_to_end(&mut bytes)
                .map_err(|e| format!("读取 {name} 失败: {e}"))?;
            let slide_text = extract_xml_text(&bytes, "a:t").unwrap_or_default();
            slide_texts.push((num, slide_text));
        }
    }

    slide_texts.sort_by_key(|(n, _)| *n);
    let mut result = String::new();
    for (i, text) in &slide_texts {
        if !text.trim().is_empty() {
            result.push_str(&format!("\n--- 幻灯片 {i} ---\n{}\n", text.trim()));
        }
    }

    if result.is_empty() {
        Err("未从 PPT 中提取到文本内容".to_string())
    } else {
        Ok(result.trim().to_string())
    }
}

// .xlsx — Excel

fn read_xlsx(path: &str) -> Result<String, String> {
    // （端到端验证发现）：真实 Excel 导出常用 inlineStr（单元格内联字符串），
    let mut strings: String = String::new();
    if let Ok(xml) = read_zip_entry(path, "xl/sharedStrings.xml") {
        strings.push_str(&extract_xml_text(&xml, "t").unwrap_or_default());
    }
    // 回退/补充：扫所有 worksheet 的 inlineStr
    if let Ok(sheets) = list_zip_entries(path) {
        for name in sheets {
            if name.starts_with("xl/worksheets/sheet") && name.ends_with(".xml") {
                if let Ok(xml) = read_zip_entry(path, &name) {
                    strings.push_str(&extract_xml_text(&xml, "t").unwrap_or_default());
                }
            }
        }
    }
    let lines: Vec<&str> = strings
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();
    if lines.is_empty() {
        Err("未从 Excel 中提取到文本内容".to_string())
    } else {
        Ok(lines.join("\n"))
    }
}

// .pdf

fn read_pdf(path: &str) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("无法读取 PDF 文件: {e}"))?;
    let doc = lopdf::Document::load_mem(&bytes).map_err(|e| format!("无法解析 PDF: {e}"))?;

    let mut text = String::new();
    let pages = doc.get_pages();
    // 按页码提取文本
    let mut page_nums: Vec<u32> = pages.keys().copied().collect();
    page_nums.sort();

    for pn in page_nums {
        if let Ok(page_text) = doc.extract_text(&[pn]) {
            let trimmed = page_text.trim();
            if !trimmed.is_empty() {
                text.push_str(&format!("\n--- 第 {pn} 页 ---\n{trimmed}\n"));
            }
        }
    }

    if text.is_empty() {
        Err("未从 PDF 中提取到文本内容（可能为扫描件）".to_string())
    } else {
        Ok(text.trim().to_string())
    }
}

// 工具函数

/// 从 ZIP 存档中读取指定路径条目的内容
fn read_zip_entry(path: &str, entry_name: &str) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("无法打开文件: {e}"))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("无法解析 ZIP 文件: {e}"))?;

    let normalized = entry_name.to_lowercase().replace('\\', "/");
    for i in 0..archive.len() {
        let name = archive
            .name_for_index(i)
            .map(|n| n.to_lowercase().replace('\\', "/"))
            .unwrap_or_default();
        if name == normalized || name.ends_with(&format!("/{normalized}")) {
            let mut entry = archive
                .by_index(i)
                .map_err(|e| format!("读取 ZIP 条目失败: {e}"))?;
            let mut buf = Vec::new();
            entry
                .read_to_end(&mut buf)
                .map_err(|e| format!("读取数据失败: {e}"))?;
            return Ok(buf);
        }
    }
    Err(format!("ZIP 中未找到 {entry_name}"))
}

/// 列出 ZIP 内所有条目名（小写、正斜杠），供遍历 worksheet 等用
fn list_zip_entries(path: &str) -> Result<Vec<String>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("无法打开文件: {e}"))?;
    let archive = zip::ZipArchive::new(file).map_err(|e| format!("无法解析 ZIP 文件: {e}"))?;
    let mut names = Vec::new();
    for i in 0..archive.len() {
        if let Some(n) = archive.name_for_index(i) {
            names.push(n.to_lowercase().replace('\\', "/"));
        }
    }
    Ok(names)
}

/// 用 quick-xml 从 XML 中提取指定标签的文本内容
fn extract_xml_text(xml_data: &[u8], tag_name: &str) -> Result<String, String> {
    use quick_xml::events::Event;
    use quick_xml::Reader;

    let mut reader = Reader::from_reader(xml_data);
    reader.config_mut().trim_text(true);

    let tag_local = tag_name.split(':').last().unwrap_or(tag_name);

    let mut in_tag = false;
    let mut text = String::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let local = std::str::from_utf8(e.local_name().as_ref())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                in_tag = local == tag_local;
            }
            Ok(Event::Text(ref e)) => {
                if in_tag {
                    if let Ok(t) = e.unescape() {
                        text.push_str(&t);
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                let local = std::str::from_utf8(e.local_name().as_ref())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if local == tag_local {
                    in_tag = false;
                    text.push(' ');
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("XML 解析错误: {e}")),
            _ => {}
        }
        buf.clear();
    }

    let cleaned: String = text.split_whitespace().collect::<Vec<&str>>().join(" ");
    if cleaned.is_empty() {
        Err("XML 中未提取到文本内容".to_string())
    } else {
        Ok(cleaned)
    }
}

#[cfg(test)]
#[path = "doc_tests.rs"]
mod doc_tests;
