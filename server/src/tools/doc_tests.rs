//! tools/doc.rs 的测试外置（部门盘查：测试全部移出核心文件）
use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_small_doc_full_text() {
        let out = document_preview("短文档内容", "a.md", "D:/x/a.md");
        assert!(out.contains("短文档内容"), "小文档应全文注入");
    }

    #[test]
    fn preview_large_doc_has_toc_head_tail_locator() {
        // 构造 >4000 字符的带标题长文档
        let mut text = String::from("# 总标题\n\n");
        for i in 0..200 {
            text.push_str(&format!("## 章节 {i}\n这是第 {i} 章的内容，用来填充长度，让文档超过四千字符阈值以便触发结构化预览分支。\n"));
        }
        let out = document_preview(&text, "big.md", "D:/x/big.md");
        assert!(out.contains("【段落目录】"), "应有段落目录: {out}");
        assert!(out.contains("章节 0"), "段落目录应含首行预览");
        assert!(out.contains("【开头"), "应有开头");
        assert!(out.contains("【结尾"), "应有结尾");
        assert!(out.contains("D:/x/big.md"), "应有全文 locator: {out}");
        assert!(out.contains("start_line/end_line"), "应引导分段读法: {out}");
        assert!(out.contains("段1"), "段落目录应从段1 开始编号: {out}");
    }

    // （L1 文档读全量）：非 md 格式（纯文本段落）也能出段落目录——
    #[test]
    fn preview_large_plain_text_has_paragraph_index() {
        let mut text = String::new();
        for i in 0..100 {
            text.push_str(&format!(
                "第{i}段的核心内容是一段普通文字，没有标题标记。\n\n"
            ));
        }
        let out = document_preview(&text, "plain.txt", "D:/x/plain.txt");
        assert!(out.contains("【段落目录】"), "纯文本也应出段落目录: {out}");
        assert!(out.contains("第0段的核心内容"), "段落首行应可见: {out}");
        assert!(
            out.contains("100 段") || out.contains("段100"),
            "应显示段落总数: {out}"
        );
        assert!(out.contains("分段读法"), "应有分段读法引导: {out}");
    }

    // ── 真实格式解析（防纸面接线）：用 tests/fixtures 下最小真实 docx/xlsx/pdf 验证
    #[test]
    fn read_document_parses_real_docx() {
        let p = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/sample.docx");
        let text = read_document(p).expect("docx 应解析成功");
        assert!(text.contains("HelloRealDocx"), "应提取 docx 正文: {text}");
    }

    #[test]
    fn read_document_parses_real_xlsx() {
        let p = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/sample.xlsx");
        let text = read_document(p).expect("xlsx 应解析成功");
        assert!(text.contains("RealCellA1"), "应提取 xlsx 单元格: {text}");
    }

    #[test]
    fn read_document_parses_real_pdf() {
        let p = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/sample.pdf");
        let text = read_document(p).expect("pdf 应解析成功");
        assert!(text.contains("RealPdfText"), "应提取 pdf 文本: {text}");
    }

    #[test]
    fn is_supported_ext_recognizes_office() {
        assert!(is_supported_ext("docx"));
        assert!(is_supported_ext("xlsx"));
        assert!(is_supported_ext("pptx"));
        assert!(is_supported_ext("pdf"));
        // 源码类也算文档附件（可注入模型上下文）
        assert!(is_supported_ext("rs"));
        assert!(is_supported_ext("py"));
        // 图片走 input_image 块，不算文档
        assert!(!is_supported_ext("png"));
        assert!(!is_supported_ext("jpg"));
    }
}
