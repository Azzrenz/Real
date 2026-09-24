//! 输出语言：思考 / 旁白 / 工具卡片文案的语言，**唯一权威**。
//!
//! 值住在 `settings` 表 `output_lang`（`zh` | `en`），由前端界面语言开关（`useLang`）同步写入。
//! 缺省 `zh`（保持现存用户行为不变）。这里只负责「读 + 渲染对应话术片段」——语言值只进
//! dynamic 区（`#frame`、语言锚），**绝不进固定系统前缀**（前缀缓存关键区，见 `context.rs`）。

use crate::error::AppResult;

/// settings 表键名。
pub const SETTING_KEY: &str = "output_lang";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputLang {
    Zh,
    En,
}

impl OutputLang {
    /// 落库值 → 语言（未知/缺失一律回退 `zh`）。
    pub fn from_setting(v: Option<&str>) -> Self {
        match v.map(str::trim) {
            Some("en") => OutputLang::En,
            _ => OutputLang::Zh,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            OutputLang::Zh => "zh",
            OutputLang::En => "en",
        }
    }

    /// `#frame` 里那一行输出语言声明（整行，含标签）。
    pub fn frame_line(self) -> &'static str {
        match self {
            OutputLang::Zh => "【输出语言】中文 —— 代码、路径、命令、错误信息等实体一律保留原文。",
            OutputLang::En => {
                "[Output language] English — keep code, paths, commands, and error messages verbatim."
            }
        }
    }

    /// 每 5 轮的语言锚（长任务里语言会被上下文稀释，主动维持）。
    pub fn anchor(self) -> &'static str {
        match self {
            OutputLang::Zh => "〔语言锚〕继续用简体中文思考与叙述。",
            OutputLang::En => "[Language anchor] Keep thinking and narrating in English.",
        }
    }

    /// 会话标题字数上限（中文按字数、英文按词约算，给足余量）。
    pub fn title_max_chars(self) -> usize {
        match self {
            OutputLang::Zh => 20,
            OutputLang::En => 40,
        }
    }
}

/// 读当前输出语言（DB 读失败按缺省 `zh`，不阻断主流程）。
pub async fn load(pool: &sqlx::SqlitePool) -> OutputLang {
    let raw = crate::db::repos::get_setting(pool, SETTING_KEY)
        .await
        .ok()
        .flatten();
    OutputLang::from_setting(raw.as_deref())
}

/// 校验来自接口的语言值（`put` 用）。
pub fn validate(v: &str) -> AppResult<OutputLang> {
    match v.trim() {
        "zh" => Ok(OutputLang::Zh),
        "en" => Ok(OutputLang::En),
        _ => Err(crate::error::AppError::Validation(
            "output_lang 必须是 zh 或 en".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_setting_defaults_to_zh() {
        assert_eq!(OutputLang::from_setting(None), OutputLang::Zh);
        assert_eq!(OutputLang::from_setting(Some("")), OutputLang::Zh);
        assert_eq!(OutputLang::from_setting(Some("zh")), OutputLang::Zh);
        assert_eq!(OutputLang::from_setting(Some("en")), OutputLang::En);
        assert_eq!(OutputLang::from_setting(Some(" en ")), OutputLang::En);
        // 未知值不猜：回退缺省，避免把乱值当英文
        assert_eq!(OutputLang::from_setting(Some("fr")), OutputLang::Zh);
    }

    #[test]
    fn zh_anchor_text_is_unchanged() {
        // 语言锚中文文案是既有行为，改语言参数化时不能顺手动它
        assert_eq!(OutputLang::Zh.anchor(), "〔语言锚〕继续用简体中文思考与叙述。");
    }

    #[test]
    fn frame_line_states_language() {
        assert!(OutputLang::Zh.frame_line().contains("中文"));
        assert!(OutputLang::En.frame_line().contains("English"));
    }
}
