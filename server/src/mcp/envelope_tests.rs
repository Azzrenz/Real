//! mcp/envelope.rs 的测试外置（部门盘查：测试全部移出核心文件）
use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_text_skips_truncation() {
        let big = "x".repeat(5000);
        let env = ToolEnvelope::success("c1", "read", vec![ContentPart::full_text(big.clone())], 0)
            .with_truncate(100);
        assert_eq!(
            env.content[0].text.as_deref(),
            Some(big.as_str()),
            "full_text 不应被截断"
        );
        assert!(!env.truncated);
    }

    #[test]
    fn normal_text_is_truncated() {
        let big = "x".repeat(5000);
        let env =
            ToolEnvelope::success("c1", "read", vec![ContentPart::text(big)], 0).with_truncate(100);
        assert!(env.truncated);
        assert!(env.content[0].text.as_deref().unwrap().contains("已截断"));
    }
}

#[cfg(test)]
mod abs_limit_tests {
    use super::*;

    #[test]
    fn full_text_over_absolute_limit_truncated() {
        let big = "x".repeat(300_000);
        let env = ToolEnvelope::success("c1", "read", vec![ContentPart::full_text(big)], 0)
            .with_truncate(100);
        assert!(env.truncated, "超过绝对上限必须标记截断");
        let t = env.content[0].text.as_deref().unwrap();
        assert!(t.contains("已截断"), "超限必须截断");
    }
}

#[cfg(test)]
mod inline_image_history_tests {
    use super::*;

    fn env_with_img(uri: &str) -> ToolEnvelope {
        ToolEnvelope::success(
            "c1",
            "read",
            vec![
                ContentPart::text("📎 shot.png（image/png）"),
                ContentPart::image_ref(uri, "image/png"),
            ],
            0,
        )
    }

    #[test]
    fn inner_base64_is_stripped_before_history() {
        let uri = format!("data:image/png;base64,{}", "A".repeat(200_000));
        let mut env = env_with_img(&uri);
        let stripped = env.strip_inline_image_bodies();
        assert_eq!(stripped, uri.len(), "返回剥离字节数（可观测，便于记账）");
        let after = env.content[1].uri.as_deref().unwrap();
        assert!(!after.contains("AAAA"), "base64 不得留在进历史的信封里");
        assert!(after.contains("未随历史留存"), "要留一句可解释的说明: {after}");
        assert_eq!(
            env.content[0].text.as_deref(),
            Some("📎 shot.png（image/png）"),
            "说明文字（含原路径）不动 —— 它正是下一轮回取的入口"
        );
    }

    #[test]
    fn http_image_survives_and_repeat_strip_is_noop() {
        let mut env = env_with_img("https://x/y.png");
        assert_eq!(
            env.strip_inline_image_bodies(),
            0,
            "http(s) 图不剥：不占载荷体积，且留在历史里依然可达"
        );
        assert_eq!(env.content[1].uri.as_deref(), Some("https://x/y.png"));

        let inline = "data:image/png;base64,BBBB";
        let mut env2 = env_with_img(inline);
        assert_eq!(env2.strip_inline_image_bodies(), inline.len());
        assert_eq!(env2.strip_inline_image_bodies(), 0, "已剥过的不再重复处理");
    }
}
