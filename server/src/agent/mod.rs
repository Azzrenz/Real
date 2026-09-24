//! Agent 编排模块出口（模块化拆包为五域）

pub mod context;
pub mod evolution;
pub mod naming;
pub mod output_lang;
pub mod execution;
pub mod history;
pub mod memory;
pub mod orchestration;
pub mod session_title;
pub mod planning;
pub mod specs;

// ── 旧路径兼容层 ──
pub use memory::project_profile;
pub use memory::workspace;
pub use planning::plan;
