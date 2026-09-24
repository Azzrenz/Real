//! agent/memory/journal.rs 的测试外置（部门纪律：核心文件测试移出）

use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    /// 任务目录名样例（统一时间戳形态，见 docs/20260915-数据落盘与命名规范.md §二）
    const TK: &str = "20260912T145519+0800";

    /// 测试清理：删**数据根**下这个 workspace 对应的整个项目目录。
    fn cleanup(ws: &str) {
        let proj = crate::path::data_root::data_root()
            .join("projects")
            .join(project_key(ws));
        std::fs::remove_dir_all(&proj).ok();
    }

    /// 造一个探针工作区：规范化 + **先清上次残留**（重跑/并行都不互相污染）。
    fn probe(tag: &str) -> String {
        let raw = std::env::temp_dir().join(format!("real_journal_{tag}"));
        let ws = crate::path::normalize_workspace(&raw.to_string_lossy());
        cleanup(&ws);
        ws
    }

    /// 路径必须落在**数据根**的 `projects/<项目标识>/` —— **用户的项目树里零痕迹**。
    #[test]
    fn paths_live_in_data_root_not_project() {
        let root = project_dir("D:\\Proj");
        let base = crate::path::data_root::data_root();
        assert!(
            root.starts_with(&base),
            "记忆必须落在数据根下：{root:?} 不在 {base:?} 内"
        );
        assert!(
            !root.to_string_lossy().contains(".real"),
            "项目树里不得再出现 .real：{root:?}"
        );
        assert!(root.to_string_lossy().contains("projects"), "{root:?}");
        assert!(root.ends_with(&project_key("D:\\Proj")), "{root:?}");
        assert!(
            !root.ends_with("memory"),
            "项目目录下不再套 memory/ 这一层（任务目录本身就是工作空间）：{root:?}"
        );
        // 任务目录：…/tasks/<创建时刻>/，日志与记忆都在里面（任务级隔离）
        let t = task_dir("D:\\Proj", TK);
        assert!(t.to_string_lossy().contains("tasks"), "{t:?}");
        assert!(t.ends_with(TK), "{t:?}");
        assert!(task_journal_path("D:\\Proj", TK, "2026-09-12")
            .to_string_lossy()
            .ends_with("worklog-2026-09-12.md"));
        assert!(task_memory_path("D:\\Proj", TK)
            .to_string_lossy()
            .ends_with("MEMORY.md"));
        // 长期知识：…/longterm/<类名>.md —— **在任务目录之外** ⇒ 删任务动不到它
        let l = longterm_path("D:\\Proj", LONGTERM_KINDS[0].0);
        assert!(l.to_string_lossy().contains("longterm"), "{l:?}");
        assert!(!l.starts_with(&t), "长期知识不得位于任务目录内：{l:?}");

        for p in [&t, &l, &task_memory_path("D:\\Proj", TK)] {
            assert!(
                p.to_string_lossy().is_ascii(),
                "任务工作空间的路径不得含非 ASCII 字符：{p:?}"
            );
        }
        for (kind, _, _) in LONGTERM_KINDS {
            assert!(kind.is_ascii(), "长期知识文件名必须 ASCII：{kind}");
        }
    }

    /// 项目标识：**可读 + 唯一 + 稳定**（三者缺一都会让记忆分裂）。
    #[test]
    fn project_key_is_readable_unique_and_stable() {
        // 可读：末段名在前面（人一眼知道是哪个项目）
        let k = project_key("D:\\Workspace\\My Suite\\MyPlugin");
        assert!(k.starts_with("myplugin-"), "{k}");

        // 通用末段名要补父级：`D:\project-b\server` 只有 `server` 的话，人认不出是哪个项目
        let e = project_key("D:/project-b/server");
        assert!(e.starts_with("project-b-server-"), "{e}");

        // 唯一：不同盘的同名目录必须分开（只取末段名会撞车）
        assert_ne!(
            project_key("D:\\a\\app"),
            project_key("D:\\b\\app"),
            "同名不同盘必须落到不同目录"
        );

        // 稳定：同一路径的不同写法（大小写 / 斜杠）必须归到同一个目录
        assert_eq!(
            project_key("D:\\proj"),
            project_key("d:/proj"),
            "Windows 路径不区分大小写，两种写法必须同目录"
        );

        // 不空手：空路径/纯符号也要给出合法目录名（空名会让路径塌成父目录）
        for weird in ["", "D:\\", "\\\\?\\", "C:\\..."] {
            let k = project_key(weird);
            assert!(!k.is_empty(), "标识不得为空：{weird:?}");
            assert!(!k.contains(['/', '\\', ':']), "标识不得含路径分隔符：{k}");
        }
    }

    /// 产物分类表（TASK_SUBDIRS）：**它是唯一事实源** —— 目录名重复、或两条判据一字不差，
    #[test]
    fn task_subdirs_are_unique_and_meaningful() {
        let mut seen: Vec<&str> = Vec::new();
        for (sub, what) in TASK_SUBDIRS {
            assert!(!sub.is_empty() && !what.is_empty(), "目录名与判据都不得为空");
            assert!(!seen.contains(&sub), "分类目录重复：{sub}");
            assert!(
                !seen.contains(&what),
                "两条判据一字不差 ⇒ 模型分不清该放哪：{what}"
            );
            seen.push(sub);
            seen.push(what);
        }
        for must in ["attachments", "scripts", "artifacts"] {
            assert!(
                TASK_SUBDIRS.iter().any(|(d, _)| *d == must),
                "分类表里缺 `{must}/` —— 用户明确点名过这一类"
            );
        }
        let cache = TASK_SUBDIRS
            .iter()
            .find(|(d, _)| *d == "cache")
            .map(|(_, w)| *w);
        assert!(
            cache.map(|w| w.contains("用完可丢")).unwrap_or(false),
            "「cache」的判据必须点明「用完可丢」：{cache:?}"
        );
        // 目录名全 ASCII —— 与 `task_subdirs_are_unique_and_meaningful` 同一不变量，这里再钉口径
        for (sub, _) in TASK_SUBDIRS {
            assert!(sub.is_ascii(), "分类目录名必须 ASCII：{sub}");
        }
    }

    /// 上传侧引用的分类名必须在表里登记 —— 否则附件会落到一个不存在的类里（两处漂移）。
    #[test]
    fn attach_subdir_is_registered() {
        assert!(
            TASK_SUBDIRS.iter().any(|(d, _)| *d == ATTACH_SUBDIR),
            "{ATTACH_SUBDIR} 未登记进 TASK_SUBDIRS —— 上传侧引用了它，两处会漂移"
        );
    }

    /// 任务目录名由**会话创建时刻**派生（北京时区）；异常输入兜底成当前时刻，**绝不空手**。
    #[test]
    fn task_key_derives_from_created_at() {
        // 输入是 UTC（+00:00），任务目录名用北京时间（+8）
        assert_eq!(
            task_key_from_created_at("2026-09-10T04:55:25.680739600+00:00"),
            "20260910T125525+0800"
        );
        // 解析不了的输入 → 兜底成当前时刻；**长度与形态仍合规**，绝不是空串
        let k = task_key_from_created_at("");
        assert_eq!(k.chars().count(), 20, "YYYYMMDDTHHMMSS+0800 共 20 字符: {k}");
        assert!(k.ends_with("+0800"), "带数字时区偏移: {k}");
        assert_eq!(k.matches('T').count(), 1, "日期与时刻之间一个 T: {k}");
    }

    /// 没有工作区时也要给出机制（相对表述），**绝不返回空**——否则模型不知道自己有地方可记。
    #[test]
    fn prompt_survives_missing_workspace() {
        let s = build_journal_prompt("", TK, true);
        assert!(s.contains("数据根"), "要说清落点在数据根下：{s}");
        assert!(s.contains("tasks/"), "要给出任务工作空间的位置：{s}");
        assert!(s.contains("MEMORY.md"), "{s}");
        assert!(s.contains(&MEMORY_MAX_CHARS.to_string()), "上限要写清：{s}");
        for (kind, _, _) in LONGTERM_KINDS {
            assert!(s.contains(kind), "三类长期知识都要点名：{s}");
        }
        for (sub, _) in TASK_SUBDIRS {
            assert!(s.contains(sub), "没有工作区时也要把产物分类讲清（{sub}/）：{s}");
        }
    }

    /// 文件还不存在（新会话第一次干活）——仍要给出绝对路径 + 「今天还没记」+ 上限 + 任务目录。
    #[test]
    fn prompt_reports_paths_when_files_absent() {
        let ws = probe("probe_absent");
        let s = build_journal_prompt(&ws, TK, true);
        assert!(!s.is_empty(), "永不返回空");
        assert!(s.contains("MEMORY.md"), "{s}");
        assert!(s.contains(TK), "任务目录名要出现在注入里：{s}");
        assert!(s.contains("今天还没记"), "没文件要明说，别让模型猜：{s}");
        assert!(
            s.contains(&MEMORY_MAX_CHARS.to_string()),
            "上限必须出现：{s}"
        );
        assert!(s.contains("不要猜、不要问"), "回忆路径要说清：{s}");
        // 分类目录要**真的建出来**：模型只会 write 文件，不会先 mkdir（缺目录会白跑一轮）
        assert!(task_dir(&ws, TK).is_dir(), "任务目录应已创建");
        for (sub, what) in TASK_SUBDIRS {
            assert!(task_dir(&ws, TK).join(sub).is_dir(), "{sub}/ 应已创建");
            assert!(s.contains(sub), "注入里要点名 {sub}/：{s}");
            assert!(
                s.contains(what),
                "注入里要给 {sub}/ 的判据（否则模型不知道该往里放什么）：{s}"
            );
        }
        // 且**必须建在数据根**，不能在 workspace 里（那就是"往用户项目里塞目录"）
        assert!(
            !std::path::Path::new(&ws).join(".real").exists(),
            "工作区里不得被建出 .real"
        );
        cleanup(&ws);
    }

    /// 有内容时：条数、最近一条、体积都要如实报出；超限当场亮。
    #[test]
    fn prompt_reports_entries_and_memory_size() {
        let ws = probe("probe_present");
        std::fs::create_dir_all(task_dir(&ws, TK)).unwrap();
        std::fs::write(
            task_journal_path(&ws, TK, &today()),
            "# 工作日志\n- 09:00 做了 A → 落在 B\n- 09:30 做了 C → 落在 D\n",
        )
        .unwrap();
        std::fs::write(task_memory_path(&ws, TK), "约定：X").unwrap();

        let s = build_journal_prompt(&ws, TK, true);
        assert!(s.contains("今天已有 2 条"), "{s}");
        assert!(s.contains("09:30 做了 C"), "最近一条要带上：{s}");
        assert!(s.contains("4 / 3000"), "本任务记忆体积要带上：{s}");
        assert!(!s.contains("已超上限"), "4 字不该报超限：{s}");

        // 超限要亮出来（否则模型会一直往上写，记忆变成垃圾场）
        std::fs::write(task_memory_path(&ws, TK), "字".repeat(MEMORY_MAX_CHARS + 1)).unwrap();
        let s2 = build_journal_prompt(&ws, TK, true);
        assert!(s2.contains("已超上限"), "超限必须当场提示：{s2}");

        cleanup(&ws);
    }

    /// 长期知识分类：每类**独立**上报体积（混一个文件就没法判断该往哪写）；超限单独亮。
    #[test]
    fn longterm_kinds_reported_separately() {
        let ws = probe("lt_probe");
        std::fs::create_dir_all(longterm_dir(&ws)).unwrap();
        std::fs::write(longterm_path(&ws, "rules"), "以后都用中文").unwrap();
        std::fs::write(
            longterm_path(&ws, "project"),
            "字".repeat(LONGTERM_MAX_CHARS + 1),
        )
        .unwrap();
        let s = build_journal_prompt(&ws, TK, true);
        assert!(s.contains("rules"), "{s}");
        assert!(s.contains("project"), "{s}");
        assert!(s.contains("已超上限"), "超限的那一类要单独亮：{s}");
        cleanup(&ws);
    }

    #[test]
    fn legacy_memory_seeded_once() {
        let ws = probe("seed_probe");
        std::fs::create_dir_all(project_dir(&ws)).unwrap();
        std::fs::write(project_dir(&ws).join("MEMORY.md"), "旧积累：架构是 X").unwrap();

        let _ = build_journal_prompt(&ws, TK, true);
        let seeded = std::fs::read_to_string(longterm_path(&ws, LONGTERM_KINDS[0].0)).unwrap();
        assert!(seeded.contains("旧积累：架构是 X"), "旧内容应接续过来：{seeded}");

        std::fs::write(longterm_path(&ws, LONGTERM_KINDS[0].0), "用户改过的内容").unwrap();
        let _ = build_journal_prompt(&ws, TK, true);
        let again = std::fs::read_to_string(longterm_path(&ws, LONGTERM_KINDS[0].0)).unwrap();
        assert_eq!(again, "用户改过的内容", "已存在则绝不覆盖（幂等）");
        cleanup(&ws);
        std::fs::remove_dir_all(&ws).ok();
    }

    /// 老位（`<workspace>/.real/memory/`）的记忆要**一次性搬进数据根**，一个文件都不能丢，
    #[test]
    fn legacy_workspace_memory_migrates_into_data_root() {
        let ws = probe("migrate_probe");
        let old = std::path::Path::new(&ws).join(".real").join("memory");
        // 老位按**旧中文名**造 —— 那才是磁盘上的真实历史形态
        std::fs::create_dir_all(old.join("任务").join(TK)).unwrap();
        std::fs::create_dir_all(old.join("长期")).unwrap();
        std::fs::write(old.join("任务").join(TK).join("日志-2026-09-12.md"), "- 09:00 干了 A").unwrap();
        std::fs::write(old.join("长期").join("项目事实.md"), "架构是 X").unwrap();
        std::fs::write(old.join("MEMORY.md"), "旧单文件记忆").unwrap();

        let _ = build_journal_prompt(&ws, TK, true);

        let new = project_dir(&ws);
        // **正名后**的落点：longterm/project.md、tasks/<TK>/worklog-*.md
        assert!(
            new.join("longterm").join("project.md").is_file(),
            "长期知识要搬过来并正名（长期/项目事实.md → longterm/project.md）"
        );
        assert!(
            new.join("tasks").join(TK).join("worklog-2026-09-12.md").is_file(),
            "任务日志要搬过来并正名（任务/日志-*.md → tasks/worklog-*.md）"
        );
        // 旧中文名不得残留 —— 两套并存会让模型不知道哪种口径算数
        assert!(
            !new.join("长期").exists() && !new.join("任务").exists(),
            "迁移后不得残留旧的中文目录名"
        );
        assert!(new.join("MEMORY.md").is_file(), "旧单文件要搬过来");
        assert_eq!(
            std::fs::read_to_string(new.join("longterm").join("project.md")).unwrap(),
            "架构是 X",
            "内容不得被改写"
        );
        assert!(old.is_dir(), "迁移不得删除老位");
        cleanup(&ws);
        std::fs::remove_dir_all(&ws).ok();
    }

    /// 日期按北京时间切分（固定 +8，不随系统时区漂）。
    #[test]
    fn today_is_utc_plus_8_and_well_formed() {
        let d = today();
        assert_eq!(d.chars().count(), 10, "yyyy-MM-dd：{d}");
        assert!(d.starts_with("20"), "{d}");
        assert_eq!(d.matches('-').count(), 2, "{d}");
    }

    #[test]
    fn longterm_can_be_off_but_workspace_stays() {
        let ws = probe("scope_probe");
        let with_lt = build_journal_prompt(&ws, TK, true);
        let without = build_journal_prompt(&ws, TK, false);

        assert!(with_lt.contains("longterm"), "共享模式要给长期知识目录：{with_lt}");
        assert!(
            !without.contains("跨任务长期知识"),
            "关共享时不得出现长期知识：{without}"
        );
        // **两者都必须给工作空间** —— 这是本次修正的核心
        let tdir = task_dir(&ws, TK).display().to_string();
        for s in [&with_lt, &without] {
            assert!(s.contains(&tdir), "工作空间路径必须无条件给出：{s}");
            assert!(s.contains("artifacts"), "分类目录必须无条件给出：{s}");
        }
        cleanup(&ws);
    }
}
