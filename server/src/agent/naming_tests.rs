use super::*;

#[test]
fn placeholder_is_recognised_with_surrounding_space() {
    for t in ["", "   ", "新任务", " 新任务 ", "New task"] {
        assert!(is_placeholder(t), "应识别为占位: {t:?}");
    }
}

#[test]
fn user_named_titles_are_never_treated_as_placeholder() {
    for t in ["新任务清单", "我的任务", "Cubase 安装目录", "New tasks"] {
        assert!(!is_placeholder(t), "不该识别为占位: {t:?}");
    }
}

#[test]
fn condense_flattens_whitespace() {
    assert_eq!(condense("  帮我   看看  Cubase  ").as_deref(), Some("帮我 看看 Cubase"));
    assert_eq!(condense("第一行\n第二行").as_deref(), Some("第一行 第二行"));
}

#[test]
fn condense_drops_skill_prefix_segments() {
    assert_eq!(condense("/find-skills 查一下技能").as_deref(), Some("查一下技能"));
    assert_eq!(condense("/a /b 正文开始").as_deref(), Some("正文开始"));
}

#[test]
fn condense_truncates_by_char_not_byte() {
    let long = "一".repeat(40);
    let got = condense(&long).expect("长文本应当有结果");
    assert_eq!(got.chars().count(), MAX_CHARS + 1, "应保留 20 个字符 + 省略号");
    assert!(got.ends_with('…'));
}

#[test]
fn condense_returns_none_when_nothing_is_left() {
    assert!(condense("   ").is_none());
    assert!(condense("/only-skill").is_none());
}

#[test]
fn condense_keeps_short_input_untouched() {
    assert_eq!(condense("看看 Cubase 目录").as_deref(), Some("看看 Cubase 目录"));
}

#[test]
fn condense_drops_a_leading_path_run() {
    // 用户常把话说成「路径 + 正文」，路径不该变成任务名。
    let got = condense(r"C:\Program Files\Cubase 15把它UI的界面挖出来").expect("应当有结果");
    assert!(!got.contains(r"C:"), "路径不该进任务名: {got:?}");
    assert!(got.starts_with('把'), "应从正文起头: {got:?}");
    assert_eq!(condense(r"D:/proj 帮我把这个面板做矮").as_deref(), Some("帮我把这个面板做矮"));
}

#[test]
fn condense_gives_no_name_to_a_bare_path() {
    // 整句就是路径 —— 不猜，交给兜底（占位标题由模型命名）。
    assert!(condense(r"D:\Workspace\My Suite\MyPlugin").is_none());
    assert!(condense(r"\\server\share\thing").is_none());
}

fn paths(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn area_matches_the_most_specific_rule() {
    // features/chat 比 client/src 具体，必须赢。
    let p = paths(&[r"D:\proj\client\src\features\chat\components\Composer.tsx"]);
    assert_eq!(area_from_paths(&p).as_deref(), Some("聊天面板"));
}

#[test]
fn area_follows_the_majority_not_the_first_file() {
    let p = paths(&[
        r"D:\proj\client\src\features\chat\ChatView.tsx",
        r"D:\proj\server\src\routes\chat.rs",
        r"D:\proj\server\src\routes\settings.rs",
        r"D:\proj\server\src\agent\mod.rs",
    ]);
    // server 那三条压过 client 那一条。
    assert_eq!(area_from_paths(&p).as_deref(), Some("后端接口"));
}

#[test]
fn area_handles_forward_slashes_too() {
    let p = paths(&["D:/proj/client/src/styles/chat.css"]);
    assert_eq!(area_from_paths(&p).as_deref(), Some("样式"));
}

#[test]
fn area_is_none_when_nothing_matches() {
    let p = paths(&[r"E:\elsewhere\some\file.txt"]);
    assert!(area_from_paths(&p).is_none(), "没命中就不要猜，返回 None");
    assert!(area_from_paths(&[]).is_none());
}

