//! 规范按需加载层

macro_rules! rule {
    ($name:literal) => {
        include_str!(concat!("../../specs/rules/", $name, ".md"))
    };
}

macro_rules! summary {
    ($id:literal) => {
        include_str!(concat!("../../specs/summaries/", $id, ".md"))
    };
}

/// 装填预算（字符）——命中条目超预算的只列摘要，不展开正文。
const BUDGET_CHARS: usize = 2_400;

/// 一条规范：id + 一行摘要（L1）+ 正文（L2）+ 触发词
struct Spec {
    id: &'static str,
    summary: &'static str,
    rules: &'static str,
    /// 命中判据：goal 小写化后含任一触发词。
    triggers: &'static [&'static str],
}

/// 规范注册表（编译期嵌入，避免运行时扫描文件系统）
static REGISTRY: [Spec; 19] = [
        Spec {
            id: "SP-SHELL",
            summary: summary!("SP-SHELL"),
            rules: rule!("shell_dispatch"),
            triggers: &[
                "命令", "cmd", "bash", "shell", "grep", "findstr", "管道", "引号", "转义",
                "报错", "乱码", "编码", "执行",
            ],
        },
        Spec {
            id: "SP-LOCATE",
            summary: summary!("SP-LOCATE"),
            rules: rule!("locate_anchor"),
            triggers: &[
                "搜索", "检索", "定位", "找到", "锚点", "源码", "grep", "哪个文件", "行号",
            ],
        },
        Spec {
            id: "SP-REFACTOR",
            summary: summary!("SP-REFACTOR"),
            rules: rule!("refactor_order"),
            triggers: &[
                "重构", "拆分", "搬移", "批量替换", "改名", "重命名", "迁移", "抽出来",
                "批量", "多处",
            ],
        },
        Spec {
            id: "SP-VERDICT",
            summary: summary!("SP-VERDICT"),
            rules: rule!("verdict_signals"),
            triggers: &[
                "后台", "进程", "端口", "服务", "启动", "编译", "构建", "build", "测试",
                "test", "server", "部署", "跑起来", "起服务", "超时",
            ],
        },
        Spec {
            id: "SP-OUTPUT",
            summary: summary!("SP-OUTPUT"),
            rules: rule!("output_budget"),
            triggers: &["日志", "输出", "截断", "太长", "刷屏", "全量打印"],
        },
        Spec {
            id: "SP-SCRIPT",
            summary: summary!("SP-SCRIPT"),
            rules: rule!("script_library"),
            triggers: &[
                "脚本", "python", "统计", "解析", "批量", "json", "数据", "复现", "扫一遍",
            ],
        },
        Spec {
            id: "SP-NET",
            summary: summary!("SP-NET"),
            rules: rule!("net_fetch"),
            triggers: &[
                "curl", "http", "网页", "下载", "api", "联网", "抓取", "github", "pypi",
                "npm", "接口",
            ],
        },
        Spec {
            id: "SP-GLOB",
            summary: summary!("SP-GLOB"),
            rules: rule!("glob_rules"),
            triggers: &["通配符", "批量", "目录下", "文件名", "所有的"],
        },
        Spec {
            id: "SP-PYENV",
            summary: summary!("SP-PYENV"),
            rules: rule!("python_env"),
            triggers: &[
                "python", "pip", "venv", "虚拟环境", "tomllib", ".py", "pytest", "依赖",
                "装包", "import",
            ],
        },
        Spec {
            id: "SP-RUSTC",
            summary: summary!("SP-RUSTC"),
            rules: rule!("rust_build"),
            triggers: &[
                "cargo", "rust", "编译", "build", "target", "crate", ".rs", "二进制",
                "重启", "链接",
            ],
        },
        Spec {
            id: "SP-NODE",
            summary: summary!("SP-NODE"),
            rules: rule!("node_pkg"),
            triggers: &[
                "npm", "node", "javascript", "typescript", "package.json", "pnpm", "yarn",
                "前端", "node_modules", "vite",
            ],
        },
        Spec {
            id: "SP-WINENC",
            summary: summary!("SP-WINENC"),
            rules: rule!("win_encoding"),
            triggers: &[
                "乱码", "编码", "gbk", "utf-8", "findstr", "空格", "bat", "chcp",
                "windows", "路径",
            ],
        },
        Spec {
            id: "SP-GIT",
            summary: summary!("SP-GIT"),
            rules: rule!("git_ops"),
            triggers: &[
                "git", "提交", "commit", "分支", "branch", "回滚", "rollback", "diff",
                "版本控制", "stash",
            ],
        },
        Spec {
            id: "SP-TEST",
            summary: summary!("SP-TEST"),
            rules: rule!("test_discipline"),
            triggers: &[
                "测试", "test", "断言", "assert", "回归", "用例", "失败", "fail", "跑通",
            ],
        },
        Spec {
            id: "SP-BACKUP",
            summary: summary!("SP-BACKUP"),
            rules: rule!("backup_first"),
            triggers: &["备份", "留底", "恢复", "还原", "回退", "覆盖", "改前", "存档"],
        },
        Spec {
            id: "SP-READBIG",
            summary: summary!("SP-READBIG"),
            rules: rule!("read_big"),
            triggers: &[
                "大文件", "几千行", "全文", "全部内容", "遍历", "太大", "读不完", "全部读",
            ],
        },
        Spec {
            id: "SP-EVIDENCE",
            summary: summary!("SP-EVIDENCE"),
            rules: rule!("evidence_layers"),
            triggers: &[
                "挖出", "挖出来", "逆向", "解包", "反编译", "资源包", "皮肤", "提取资源",
            ],
        },
        // Appended rather than inserted: the registry order IS the fill order (first come, first
        Spec {
            id: "SP-VISUAL",
            summary: summary!("SP-VISUAL"),
            rules: rule!("visual_artifact"),
            triggers: &[
                "html", "网页", "页面", "可视化", "图表", "看板", "dashboard", "报告",
                "导出", "单页", "仪表盘", "图谱", "画个图", "展示",
            ],
        },
        Spec {
            id: "SP-EDITSAFE",
            summary: summary!("SP-EDITSAFE"),
            rules: rule!("edit_safety"),
            triggers: &[
                "改", "修", "补", "调整", "替换", "编辑", "覆写", "重写", "行号", "转义",
                "加个", "加一个", "find", "old",
            ],
        },
];

/// 规范注册表出口（`static` 而非函数返回临时数组——临时值被 `Vec<&Spec>` 借走会悬垂）
fn registry() -> &'static [Spec] {
    &REGISTRY
}

/// 命中判据：goal 小写化后含任一触发词。
fn hits(spec: &Spec, goal_lower: &str) -> bool {
    spec.triggers.iter().any(|t| goal_lower.contains(t))
}

/// 基线组：**一个都没命中时**才注入，保证基础方法论可达。
const BASELINE: &[&str] = &["SP-SHELL", "SP-LOCATE"];

fn baseline() -> Vec<&'static Spec> {
    BASELINE
        .iter()
        .filter_map(|id| registry().iter().find(|s| s.id == *id))
        .collect()
}

/// 本次任务命中的规范（预算内）。
pub fn match_specs(goal: &str) -> String {
    let goal_lower = goal.to_lowercase();
    let mut hit: Vec<&Spec> = registry().iter().filter(|s| hits(s, &goal_lower)).collect();
    if hit.is_empty() {
        hit = baseline();
    }
    if hit.is_empty() {
        return String::new();
    }

    let mut out = String::from("## 工作规范（与本次任务相关）\n\n");
    let mut used = 0usize;
    let mut dropped: Vec<&Spec> = Vec::new();

    for s in hit {
        let block = format!("### {}\n{}\n\n", s.id, s.rules.trim());
        if used + block.chars().count() > BUDGET_CHARS {
            dropped.push(s);
            continue;
        }
        used += block.chars().count();
        out.push_str(&block);
    }

    // 超预算的只留「id + 摘要」——让模型知道还有哪条可用，而不是静默消失
    if !dropped.is_empty() {
        out.push_str("（以下规范与本次任务相关但未展开，需要时按 id 索取）：\n");
        for s in &dropped {
            out.push_str(&format!("- [{}] {}\n", s.id, s.summary));
        }
    }
    out
}

#[cfg(test)]
mod specs_tests {
    use super::{match_specs, registry, BUDGET_CHARS};

    /// 触发词必须已小写——否则 `goal.to_lowercase()` 永远匹配不上（静默失效）
    #[test]
    fn triggers_are_lowercase() {
        for s in registry() {
            for t in s.triggers {
                assert_eq!(
                    *t,
                    t.to_lowercase(),
                    "规范 {} 的触发词 {:?} 含大写：小写化后永远不命中",
                    s.id,
                    t
                );
            }
        }
    }

    /// id 与摘要必须成对、非空（少一个 = 摘要行显示成空白）
    #[test]
    fn ids_and_summaries_are_paired() {
        for s in registry() {
            assert!(!s.id.is_empty(), "规范 id 不能为空");
            assert!(
                !s.summary.trim().is_empty(),
                "规范 {} 的摘要为空（summaries/ 缺文件或写空了）",
                s.id
            );
            assert!(
                !s.rules.trim().is_empty(),
                "规范 {} 的正文为空（rules/ 缺文件或写空了）",
                s.id
            );
        }
    }

    /// 一个都没命中 → 回落基线（保证基础方法论可达），不是空手
    #[test]
    fn baseline_fills_when_nothing_matches() {
        let s = match_specs("你好");
        assert!(s.contains("SP-SHELL"), "无命中时应回落基线 SP-SHELL：{s}");
        assert!(s.contains("SP-LOCATE"), "无命中时应回落基线 SP-LOCATE：{s}");
    }

    /// 有命中就**不再塞基线** —— 否则等于把基线做成常驻，白做分层
    #[test]
    fn baseline_is_not_added_when_something_matches() {
        let s = match_specs("抓网页");
        assert!(s.contains("SP-NET"), "抓网页应命中 SP-NET：{s}");
        assert!(
            !s.contains("SP-LOCATE"),
            "已命中时不得再叠加基线（那会让分层失效）：{s}"
        );
    }

    #[test]
    fn shell_spec_hits_on_command_words() {
        let s = match_specs("把这段命令改一下，引号被吃了");
        assert!(s.contains("SP-SHELL"), "命令/引号应命中 SP-SHELL：{s}");
    }

    #[test]
    fn verdict_spec_hits_on_service_words() {
        let s = match_specs("后台起服务，等端口就绪");
        assert!(s.contains("SP-VERDICT"), "服务/端口应命中 SP-VERDICT：{s}");
    }

    /// 预算闸门：超预算的条目只列摘要，不展开正文
    #[test]
    fn budget_caps_expanded_rules() {
        let s = match_specs("命令 搜索 服务 日志 脚本 网页 通配符");
        assert!(
            s.chars().count() <= BUDGET_CHARS + 400,
            "注入体应受预算约束（含摘要尾巴），实测 {} 字符",
            s.chars().count()
        );
    }
}
