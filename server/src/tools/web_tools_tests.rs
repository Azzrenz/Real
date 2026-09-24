//! tools/web_tools.rs 的测试外置（部门盘查：测试全部移出核心文件）
use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_html_removes_tags_scripts_entities() {
        let html = r#"<html><head><style>.a{color:red}</style><script>var x="<body>";</script></head>
<body><h1>标题一</h1><p>正文 &amp; 更多 &lt;标签&gt; &quot;引&quot; &#39;单&#39;&nbsp;尾</p><!-- 注释 --></body></html>"#;
        let t = strip_html(html);
        assert!(t.contains("标题一"), "标题应保留: {t}");
        assert!(t.contains("正文 & 更多 <标签> \"引\" '单'"), "实体应还原: {t}");
        assert!(!t.contains("color:red"), "style 块应剥除: {t}");
        assert!(!t.contains("var x"), "script 块应剥除: {t}");
        assert!(!t.contains("注释"), "注释应剥除: {t}");
        assert!(!t.contains("  "), "空白应压缩: {t}");
    }

    #[test]
    fn search_without_key_teaches_config() {
        // 未配置 key：确定性教学（不瞎跑网络）
        set_search_key(None);
        assert!(!has_search_key(), "清空后 has_search_key 应为 false");
        let rt = tokio::runtime::Runtime::new().unwrap();
        let err = rt
            .block_on(bigmodel_search("test", 3))
            .expect_err("无 key 应报错");
        assert!(err.contains("搜索凭据"), "教学应指向配置: {err}");
        assert!(err.contains("WEB_SEARCH_NO_KEY"), "应带确定性错误码: {err}");
    }

    #[test]
    fn search_key_is_independent_of_model() {
        set_search_key(Some("sk-any".into()));
        assert!(has_search_key(), "写入后应为 true");
        assert_eq!(search_key().as_deref(), Some("sk-any"));
        // 空白串等同未配置（防止「设了个空格」被误判为已配 → 挂出工具又必然失败）
        set_search_key(Some("   ".into()));
        assert!(!has_search_key(), "空白串应视为未配置");
        set_search_key(None);
    }
}
