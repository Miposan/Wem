//! Agent — AI Agent 子系统
//!
//! 与 block_system 并列的平级子系统，提供 AI Agent 能力。
//! 模块结构：
//! - provider: LLM 接入层（Provider trait + Anthropic 实现）
//! - tools: 工具能力层（Tool trait + Registry + 各工具实现）
//! - permission: 权限拦截
//! - prompt: Prompt 组装
//! - context: 上下文压缩管理
//! - session: 会话管理
//! - loop_runner: Agentic Loop 编排核心
//! - handler: Axum HTTP handler

pub mod provider;
pub mod tools;
pub mod permission;
pub mod prompt;
pub mod context;
pub mod session;
pub mod loop_runner;
pub mod runtime;
pub mod handler;
pub mod mcp;
