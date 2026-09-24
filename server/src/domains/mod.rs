//! 域插件注册表（生态章程：Real 是环境面，域能力 U 盘式接入）

use crate::mcp::registry::BuiltinTool;
use std::sync::Arc;

/// 域插件接口（U 盘插口协议）
pub trait Domain: Send + Sync {
    /// 域 id（小写英文，目录名 = 域名，工具名前缀 = "<id>_"）
    #[allow(dead_code)]
    fn id(&self) -> &'static str;
    /// 本域工具清单（域内 tools.rs 装配，工具名必须带 <id>_ 前缀）
    fn tools(&self) -> Vec<Arc<dyn BuiltinTool>>;
    /// 域分类：goal 是否属本域（关键词互斥——域间不得有交集，测试断言）
    fn classify(&self, goal: &str) -> bool;
    /// 域工作流话术（prompt.md，include_str! 编译期嵌入）；无则 None
    fn system_prompt(&self) -> Option<&'static str>;
    /// 域工具信封超时地板（秒）：命中返回上浮值（如长任务工具 24h）
    fn tool_timeout_floor(&self, tool: &str) -> Option<u64>;
}

/// 域注册表（唯一权威）——新域在此加一行
pub fn registered() -> Vec<&'static dyn Domain> {
    vec![]
}
