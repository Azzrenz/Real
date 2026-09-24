//! routes/chat.rs 的测试外置（部门盘查：测试全部移出核心文件）
use super::*;

#[cfg(test)]
mod chat_tests {
    use super::*;

    #[test]
    fn attachment_name_keeps_chinese() {
        // 中文名 → 保留原文（变成 "Real____-___.md" 则无法辨认）
        let name = sanitize_attachment_name("Real工具操作指南-终极整合版.md", 0);
        assert_eq!(
            name, "Real工具操作指南-终极整合版.md",
            "中文必须保留: {name}"
        );
    }

    #[test]
    fn attachment_name_blocks_path_traversal() {
        // 路径穿越/分隔符 → 替换成下划线（安全威胁必须清洗）
        let name = sanitize_attachment_name(r"..\..\evil.md", 0);
        assert!(!name.contains('\\'), "反斜杠必须清洗: {name}");
        assert!(!name.contains('/'), "斜杠必须清洗: {name}");
        assert!(!name.contains(".."), "路径穿越必须清洗: {name}");
        let name2 = sanitize_attachment_name(r"C:\Windows\system32\pwd.dll", 1);
        assert!(!name2.contains(':'), "盘符冒号必须清洗: {name2}");
    }

    #[test]
    fn attachment_name_falls_back_when_all_illegal() {
        // 全非法字符 → fallback attachment_N（不产生空名/危险名）
        assert_eq!(sanitize_attachment_name("///***???", 3), "attachment_3");
        // 空名同样 fallback
        assert_eq!(sanitize_attachment_name("   ", 0), "attachment_0");
    }

    #[test]
    fn attachment_name_keeps_ascii_and_dots() {
        // 常规英文名 + 点 → 原样保留（兼容既有用例）
        assert_eq!(
            sanitize_attachment_name("report.v2_final.md", 0),
            "report.v2_final.md"
        );
    }
}

#[cfg(test)]
mod compress_image_tests {
    use super::*;

    /// 行为级回归（收图压缩）：1.7MP 纯白图压缩后应显著变小且仍是合法 JPEG；
    #[test]
    fn large_image_compressed_smaller() {
        let mut img = image::RgbImage::new(1109, 1518);
        for (x, y, p) in img.enumerate_pixels_mut() {
            let v = ((x + y) % 256) as u8;
            *p = image::Rgb([v, v, (v / 2) + 60]);
        }
        let mut raw = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut raw), image::ImageFormat::Png)
            .unwrap();
        assert!(raw.len() > 400 * 1024, "测试图应 >400KB: {}", raw.len());
        let comp = compress_image_large(&raw).expect("大图应能压缩");
        assert!(comp.len() < raw.len(), "压缩后应更小: {} vs {}", comp.len(), raw.len());
        // 输出必须是合法 JPEG，且已降采样到最长边 ≤1280
        let dec = image::load_from_memory(&comp).expect("压缩产物应可解码");
        assert!(dec.width().max(dec.height()) <= 1280, "应降采样到 ≤1280: {}x{}", dec.width(), dec.height());
        assert!(dec.width() > 500, "不应过度缩小: {}x{}", dec.width(), dec.height());
    }

    #[test]
    fn small_image_not_compressed() {
        let mut img = image::RgbImage::new(100, 100);
        for p in img.pixels_mut() {
            *p = image::Rgb([10, 20, 30]);
        }
        let mut raw = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut raw), image::ImageFormat::Png)
            .unwrap();
        assert!(compress_image_large(&raw).is_none(), "小图不应压缩");
    }

    #[test]
    fn svg_is_text_not_raster() {
        assert!(!is_raster_image("svg"), "svg 不是位图，不得进 image_blocks");
        assert!(
            crate::tools::doc::is_supported_ext("svg"),
            "svg 必须走文本读取通道（它是纯文本 XML）"
        );
        // 位图照旧走视觉通道
        assert!(is_raster_image("png"));
        assert!(is_raster_image("jpeg"));
        assert!(is_raster_image("webp"));
        assert!(!is_raster_image("pdf"), "文档不是位图");
        // 注入文案必须**鼓励** read（与位图那句"不要 read"正好相反）
        let s = svg_prompt("<svg><text>hi</text></svg>", "a.svg", "D:/x/a.svg");
        assert!(s.contains("用 read 读"), "必须明确告知可以 read: {s}");
        assert!(s.contains("纯文本"), "必须说明是文本而非像素图: {s}");
        assert!(s.contains("a.svg"), "应带原始文件名: {s}");
    }

    #[test]
    fn garbage_bytes_safe_none() {
        assert!(compress_image_large(b"not an image at all").is_none(), "损坏输入应安全降级");
    }
}
