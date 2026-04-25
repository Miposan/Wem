//! 操作日志（Oplog）数据模型
//!
//! 基于 Operation-based 操作日志，为文档提供 undo/redo 能力。
//!
//! ## 双层存档架构
//!
//! | 层级 | 粒度 | 用途 | 生命周期 |
//! |------|------|------|----------|
//! | **Operation + Change** | 细粒度（单次操作） | undo/redo | 短期，被快照压缩后可清理 |
//! | **Snapshot** | 粗粒度（整篇文档） | 版本存档、恢复 | 长期，用户手动管理 |
//!
//! ### Operation（细粒度）
//! - 每次用户操作产生一个 Operation（全局唯一 id）
//! - Operation 内记录所有受影响 Block 的 before/after 快照
//! - undo = 恢复 before 快照；redo = 恢复 after 快照
//!
//! ### Snapshot（粗粒度）
//! - 某一时刻整篇文档所有 Block 的完整状态
//! - 触发方式：手动保存 / 每 N 个 Operation 自动 / 导入前自动
//! - 恢复 = 将文档内所有 Block 回滚到快照时的状态
//! - 快照之间的 Operation 可用于 undo/redo；快照之前的可被 GC 清理

use serde::{Deserialize, Serialize};

// ─── Operation ─────────────────────────────────────────────────

/// 一次用户操作产生的变更记录
///
/// 对应 `operations` 表的一行。id 由服务端生成（时间有序）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Operation {
    /// 操作 ID（时间有序唯一 ID）
    pub id: String,
    /// 所属文档 ID（undo/redo 按文档作用域隔离）
    pub document_id: String,
    /// 操作类型
    pub action: Action,
    /// 操作描述（可选，如 "split paragraph"）
    pub description: Option<String>,
    /// 操作时间（ISO 8601）
    pub timestamp: String,
    /// 是否已被撤销
    pub undone: bool,
    /// 编辑者标识（前端会话级 UUID，用于 SSE 回声去重）
    pub editor_id: Option<String>,
}

// ─── Action ────────────────────────────────────────────────────

/// 操作类型
///
/// 与 Block CRUD 端点一一对应，加上组合操作。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Create,
    Update,
    Delete,
    Move,
    Restore,
    Split,
    Merge,
    BatchOps,
    Import,
}

impl Action {
    pub fn as_str(&self) -> &'static str {
        match self {
            Action::Create => "create",
            Action::Update => "update",
            Action::Delete => "delete",
            Action::Move => "move",
            Action::Restore => "restore",
            Action::Split => "split",
            Action::Merge => "merge",
            Action::BatchOps => "batch_ops",
            Action::Import => "import",
        }
    }

    pub fn from_str_lossy(s: &str) -> Option<Self> {
        match s {
            "create" => Some(Action::Create),
            "update" => Some(Action::Update),
            "delete" => Some(Action::Delete),
            "move" => Some(Action::Move),
            "restore" => Some(Action::Restore),
            "split" => Some(Action::Split),
            "merge" => Some(Action::Merge),
            "batch_ops" => Some(Action::BatchOps),
            "import" => Some(Action::Import),
            _ => None,
        }
    }
}

// ─── Change ────────────────────────────────────────────────────

/// 一个 Block 在某次 Operation 中的变更记录
///
/// 对应 `changes` 表的一行。before/after 存储 Block 的完整快照。
/// - create: before = None, after = 完整 Block
/// - delete: before = 完整 Block, after = None
/// - update/move: before = 变更前, after = 变更后
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Change {
    /// 自增 ID
    pub id: i64,
    /// 所属操作 ID
    pub operation_id: String,
    /// 受影响的 Block ID
    pub block_id: String,
    /// 变更类型（create / update / delete / reparent）
    pub change_type: ChangeType,
    /// 变更前的 Block 快照（JSON，create 时为 None）
    pub before: Option<BlockSnapshot>,
    /// 变更后的 Block 快照（JSON，delete 时为 None）
    pub after: Option<BlockSnapshot>,
}

/// 变更类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ChangeType {
    /// 新建 Block
    Created,
    /// 修改 Block 内容/属性
    Updated,
    /// 软删除 Block
    Deleted,
    /// 移动 Block（改变 parent_id / position）
    Moved,
    /// 恢复 Block
    Restored,
    /// Block 被重新挂载（merge 时子块 reparent）
    Reparented,
}

impl ChangeType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ChangeType::Created => "created",
            ChangeType::Updated => "updated",
            ChangeType::Deleted => "deleted",
            ChangeType::Moved => "moved",
            ChangeType::Restored => "restored",
            ChangeType::Reparented => "reparented",
        }
    }

    pub fn from_str_lossy(s: &str) -> Option<Self> {
        match s {
            "created" => Some(ChangeType::Created),
            "updated" => Some(ChangeType::Updated),
            "deleted" => Some(ChangeType::Deleted),
            "moved" => Some(ChangeType::Moved),
            "restored" => Some(ChangeType::Restored),
            "reparented" => Some(ChangeType::Reparented),
            _ => None,
        }
    }
}

// ─── BlockSnapshot ─────────────────────────────────────────────

/// Block 快照（用于 before/after 记录）
///
/// 只存储 undo/redo 必要的字段，不含 id（id 在 Change 层面）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockSnapshot {
    pub parent_id: String,
    pub document_id: String,
    pub position: String,
    pub block_type: String,
    pub content: Vec<u8>,
    pub properties: String,
    pub status: String,
}

impl BlockSnapshot {
    /// 从 Block 模型创建快照
    pub fn from_block(block: &super::Block) -> Self {
        Self {
            parent_id: block.parent_id.clone(),
            document_id: block.document_id.clone(),
            position: block.position.clone(),
            block_type: serde_json::to_string(&block.block_type).unwrap_or_default(),
            content: block.content.clone(),
            properties: serde_json::to_string(&block.properties).unwrap_or_default(),
            status: block.status.as_str().to_string(),
        }
    }
}

// ─── API 响应类型 ──────────────────────────────────────────────

/// Undo/Redo 结果
#[derive(Debug, Clone, Serialize)]
pub struct UndoRedoResult {
    /// 恢复的操作 ID
    pub operation_id: String,
    /// 受影响的 Block ID 列表
    pub affected_block_ids: Vec<String>,
    /// 受影响的 document_id 集合（用于 SSE 广播）
    pub affected_document_ids: Vec<String>,
    /// 对应的操作类型
    pub action: String,
}

/// 历史条目（API 返回）
#[derive(Debug, Clone, Serialize)]
pub struct HistoryEntry {
    pub operation_id: String,
    pub action: String,
    pub description: Option<String>,
    pub timestamp: String,
    pub undone: bool,
    pub changes: Vec<ChangeSummary>,
}

/// 变更摘要（API 返回）
#[derive(Debug, Clone, Serialize)]
pub struct ChangeSummary {
    pub block_id: String,
    pub change_type: String,
}


