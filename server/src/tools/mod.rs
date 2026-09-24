//! 工具总线：契约驱动、后端兜底

pub mod base;
pub mod cmd_bash;
pub mod cmd_tools;
pub mod cmd_translate;
pub mod verify;
#[allow(dead_code)]
pub mod proc_status;
pub mod contract;
pub mod doc;
pub mod fs_common;
pub mod fs_read;
pub mod fs_write;
// fs_write 的符号均以 `fs_write::X` 显式路径引用（WriteTool/EditTool 等）；
pub use fs_read::*;
// err_text 从 fs_write 下沉到 contract（解 fs_read↔fs_write 循环依赖）——
pub use contract::err_text;
pub mod audit;
pub mod modify;
pub mod ask;
pub mod summary_render;
pub mod web_tools;

use crate::mcp::registry::BuiltinTool;
use std::sync::Arc;

pub fn builtins() -> Vec<Arc<dyn BuiltinTool>> {
    let mut v: Vec<Arc<dyn BuiltinTool>> = vec![
        // 文件读 / 写 / 意图式改（模型只说改什么，后端 read→edit→verify）
        Arc::new(fs_read::ReadTool),
        Arc::new(fs_write::WriteTool),
        Arc::new(modify::ModifyTool),
        // 命令执行：默认通道——一切命令 + 管道能完成的都走它
        Arc::new(cmd_tools::RunTool),
    ];
    // ── 裁剪出工具面（实现保留，恢复即加回；加回须同步话术与契约测试）──
    for d in crate::domains::registered() {
        v.extend(d.tools());
    }
    v
}
