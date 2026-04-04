//! 操作日志（Oplog）与快照（Snapshot）数据模型
//!
//! Oplog 是 source of truth。每次 Block 操作都记录一条 Operation。
//! Snapshot 是某个版本的全量内容，减少回放步数。
//!
//! 简化版（单用户，无协同）：
//! - 去掉 author / batch_id / doc_id（单用户场景可从 block 推导）
//! - 只保留核心：谁在什么时候对哪个 Block 做了什么
//!
//! 参考 05-oplog.md §1~§6

use serde::{Deserialize, Serialize};

// ─── 解析警告 ─────────────────────────────────────────────────

/// 解析过程中产生的警告
///
/// 定义在 model 层，供 parser 产出、api 层引用，避免 api→parser 层级依赖。
#[derive(Debug, Clone, Serialize)]
pub struct ParseWarning {
    /// 行号（1-based，0 表示未知）
    pub line: usize,
    /// 警告类型标识
    pub warning_type: String,
    /// 人类可读描述
    pub message: String,
    /// 采取的操作（如 `"auto_fixed"`）
    pub action: String,
}

// ─── Action 枚举 ──────────────────────────────────────────────

/// 操作类型
///
/// 每种 Action 对应一个 Block 操作端点。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum Action {
    /// 新建 Block
    Create,
    /// 更新 Block 内容或属性
    Update,
    /// 软删除 Block（含级联）
    Delete,
    /// 移动 Block（改变 parent_id 或 position）
    Move,
    /// 恢复已删除的 Block
    Restore,
}

impl Action {
    /// 转为数据库存储的字符串
    pub fn as_str(&self) -> &'static str {
        match self {
            Action::Create => "create",
            Action::Update => "update",
            Action::Delete => "delete",
            Action::Move => "move",
            Action::Restore => "restore",
        }
    }

    /// 从数据库字符串解析
    pub fn from_str_lossy(s: &str) -> Option<Self> {
        match s {
            "create" => Some(Action::Create),
            "update" => Some(Action::Update),
            "delete" => Some(Action::Delete),
            "move" => Some(Action::Move),
            "restore" => Some(Action::Restore),
            _ => None,
        }
    }
}

// ─── Operation 数据结构 ───────────────────────────────────────

/// 一条操作日志
///
/// 对应 oplog 表的一行。op_id 由 SQLite AUTOINCREMENT 自动分配。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Operation {
    /// 全局单调递增 ID（SQLite AUTOINCREMENT）
    pub op_id: i64,
    /// 操作目标 Block ID
    pub block_id: String,
    /// 操作类型
    pub action: Action,
    /// 操作数据（JSON，格式取决于 action）
    pub data: String,
    /// 操作前的 block version
    pub prev_version: u64,
    /// 操作后的 block version
    pub new_version: u64,
    /// 操作时间（ISO 8601）
    pub timestamp: String,
}

// ─── OperationData 各 Action 的 JSON 结构 ─────────────────────

/// Create 操作的数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateOpData {
    pub block_type: String,
    pub content_type: String,
    pub content: String,
    pub properties: String,
    pub parent_id: String,
    pub position: String,
}

/// Update 操作的数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateOpData {
    /// 更新后的完整 Block 内容（冗余但可靠，保证回放链不断裂）
    pub content: String,
    pub properties: String,
    /// 是否为回滚操作
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_rollback: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rollback_to: Option<u64>,
}

/// Delete 操作的数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteOpData {
    /// 级联删除的子块数量（不含自身）
    pub cascade_count: u32,
    /// 被删除 Block 的内容快照摘要
    pub snapshot: DeleteSnapshot,
}

/// Delete 操作中附带的快照摘要
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteSnapshot {
    pub block_type: String,
    pub content: String,
    pub properties: String,
}

/// Move 操作的数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoveOpData {
    pub old_parent_id: String,
    pub old_position: String,
    pub new_parent_id: String,
    pub new_position: String,
}

/// Restore 操作的数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreOpData {
    /// 级联恢复的子块数量（含自身）
    pub cascade_count: u32,
}

// ─── Snapshot 数据结构 ────────────────────────────────────────

/// Block 快照
///
/// 记录某个 Block 在某个 version 的完整内容。
/// 配合 oplog 实现高效的历史回溯。
///
/// 对应 snapshots 表的一行。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub block_id: String,
    pub version: u64,
    pub block_type: String,
    pub content_type: String,
    pub content: Vec<u8>,
    pub properties: String,
    pub parent_id: String,
    pub position: String,
    pub timestamp: String,
}

/// 快照触发原因
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotReason {
    /// 距上次快照的操作数达到阈值
    OpCountThreshold,
    /// 距上次快照的时间达到阈值
    TimeThreshold,
    /// 用户手动创建
    Manual,
}

impl SnapshotReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            SnapshotReason::OpCountThreshold => "op_count_threshold",
            SnapshotReason::TimeThreshold => "time_threshold",
            SnapshotReason::Manual => "manual",
        }
    }
}

// ─── API 响应类型 ──────────────────────────────────────────────
// 这些类型被 service 层构造、被 api/response.rs 引用，
// 放在 model 层确保依赖方向正确：api → model ← service

/// Block 变更历史条目（API 返回用）
#[derive(Debug, Clone, Serialize)]
pub struct HistoryEntry {
    pub op_id: i64,
    pub block_id: String,
    pub action: String,
    pub data: serde_json::Value,
    pub prev_version: u64,
    pub new_version: u64,
    pub timestamp: String,
}

/// 版本内容（API 返回用）
#[derive(Debug, Clone, Serialize)]
pub struct VersionContent {
    /// 目标版本号
    pub version: u64,
    /// 该版本的完整 Block 内容
    pub block: super::Block,
    /// 重建来源（快照 + 回放了 N 步 oplog）
    pub source: String,
}

/// 回滚结果（API 返回用）
#[derive(Debug, Clone, Serialize)]
pub struct RollbackResult {
    pub id: String,
    /// 回滚前的版本号
    pub prev_version: u64,
    /// 回滚后的新版本号
    pub new_version: u64,
    /// 回滚到哪个版本的内容
    pub rollback_to_version: u64,
}

/// 快照创建结果（API 返回用）
#[derive(Debug, Clone, Serialize)]
pub struct SnapshotResult {
    pub block_id: String,
    pub version: u64,
    pub reason: String,
}
