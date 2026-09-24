//! routes/sessions.rs 的测试外置（部门盘查：测试全部移出核心文件）
use super::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::repos::EventRow;

    fn reasoning(id: i64, text: &str) -> EventRow {
        EventRow {
            id,
            session_id: "s".into(),
            kind: "reasoning".into(),
            payload_json: serde_json::json!({"text": text}).to_string(),
            created_at: "t".into(),
        }
    }

    fn other(id: i64, kind: &str) -> EventRow {
        EventRow {
            id,
            session_id: "s".into(),
            kind: kind.into(),
            payload_json: "{}".into(),
            created_at: "t".into(),
        }
    }

    #[test]
    fn reasoning_same_round_keeps_only_last_snapshot() {
        // 累计全文协议：同一轮 3 条快照，后一条包含前一条全文 → 只保留最后一条。
        let events = vec![
            reasoning(1, "思考开头"),
            reasoning(2, "思考开头，继续想"),
            reasoning(3, "思考开头，继续想，最终结论"),
            other(4, "thinking"),
            other(5, "tool"),
        ];
        let out = sample_reasoning_events(events);
        let reasoning_ids: Vec<i64> = out
            .iter()
            .filter(|e| e.kind == "reasoning")
            .map(|e| e.id)
            .collect();
        assert_eq!(reasoning_ids, vec![3], "同轮只保留最后一条完整思考");
        assert_eq!(out.len(), 3, "非 reasoning 事件原样保留");
    }

    #[test]
    fn reasoning_across_rounds_keeps_each_round() {
        // 跨轮：thinking reset 清空 acc，新轮文本不包含旧轮全文 → 每轮各保留一条。
        let events = vec![
            reasoning(1, "第一轮思考：定位问题"),
            reasoning(2, "第一轮思考：定位问题，确认根因"),
            other(3, "tool"),
            reasoning(4, "第二轮思考：开始修复"),
            reasoning(5, "第二轮思考：开始修复，检查结果"),
        ];
        let out = sample_reasoning_events(events);
        let reasoning_ids: Vec<i64> = out
            .iter()
            .filter(|e| e.kind == "reasoning")
            .map(|e| e.id)
            .collect();
        assert_eq!(reasoning_ids, vec![2, 5], "每轮各保留最后一条");
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn reasoning_incremental_fragments_merge_into_full_text() {
        // 增量碎片协议（起）：相邻无 thinking 隔开的 reasoning 是同一轮碎片，
        let events = vec![reasoning(1, "第一段思考"), reasoning(2, "第二段思考")];
        let out = sample_reasoning_events(events);
        assert_eq!(out.len(), 1, "相邻碎片合并成一条");
        let text = serde_json::from_str::<serde_json::Value>(&out[0].payload_json)
            .ok()
            .and_then(|v| v.get("text").and_then(|t| t.as_str()).map(str::to_string))
            .unwrap_or_default();
        assert_eq!(text, "第一段思考第二段思考", "拼接还原全文");
        assert_eq!(out[0].id, 1, "保留首条碎片的 seq（拼接不新建行）");
    }

    #[test]
    fn reasoning_repeated_text_fragment_keeps_both() {
        // 重复文本边界：增量碎片文本恰好等于/以已有文本为前缀 → 必须拼接不丢字
        let events = vec![reasoning(1, "哈"), reasoning(2, "哈"), reasoning(3, "哈哈")];
        let out = sample_reasoning_events(events);
        let text = serde_json::from_str::<serde_json::Value>(&out[0].payload_json)
            .ok()
            .and_then(|v| v.get("text").and_then(|t| t.as_str()).map(str::to_string))
            .unwrap_or_default();
        assert_eq!(text, "哈哈哈哈", "重复文本碎片全部保留（1+1+2=4 个哈）");
    }

    #[test]
    fn reasoning_thinking_event_separates_rounds() {
        // 轮边界由 thinking 事件标记：thinking 隔开的相邻 reasoning 属于不同轮 → 不合并。
        let events = vec![
            reasoning(1, "第一轮碎片一"),
            reasoning(2, "第一轮碎片二"),
            other(3, "thinking"),
            reasoning(4, "第二轮碎片一"),
            reasoning(5, "第二轮碎片二"),
        ];
        let out = sample_reasoning_events(events);
        let reasoning_ids: Vec<i64> = out
            .iter()
            .filter(|e| e.kind == "reasoning")
            .map(|e| e.id)
            .collect();
        assert_eq!(reasoning_ids, vec![1, 4], "thinking 隔开的两轮各自合并（保留各轮首条碎片 seq）");
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn spanned_tracks_fragment_ranges() {
        // 骨架回放契约：每个聚合轮带碎片首末 seq（from, to），前端按区间懒加载全文。
        let events = vec![
            reasoning(1, "第一轮碎片一"),
            reasoning(2, "第一轮碎片二"),
            reasoning(3, "第一轮碎片三"),
            other(4, "thinking"),
            reasoning(5, "第二轮碎片一"),
        ];
        let out = sample_reasoning_spanned(events);
        let spans: Vec<(i64, i64, i64)> = out
            .iter()
            .filter(|(e, _, _)| e.kind == "reasoning")
            .map(|(e, f, t)| (e.id, *f, *t))
            .collect();
        assert_eq!(
            spans,
            vec![(1, 1, 3), (5, 5, 5)],
            "第一轮聚合行 seq=1、区间 1..3；第二轮 seq=5、区间 5..5"
        );
    }

    #[test]
    fn spanned_single_event_range_is_itself() {
        // 单条碎片成轮：from == to == 自身 seq（无碎片合并时区间即自身）。
        let events = vec![reasoning(7, "独轮思考"), other(8, "tool")];
        let out = sample_reasoning_spanned(events);
        assert_eq!(out[0].1, 7);
        assert_eq!(out[0].2, 7);
    }
}
