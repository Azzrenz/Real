//! 编排域：基座工作流 + 五个职责单一的子域。

pub mod contract;
pub mod inject;
// 编排（本域根）：`run_agent` 与循环本体
pub mod stall;
pub mod text;
pub mod tooling;
pub mod workflow;
pub use workflow::run_agent;
