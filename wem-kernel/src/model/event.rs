//! Block 变更事件模型
//!
//! 所有 Block mutation（创建/更新/删除/移动/恢复）完成后，产出一个结构化事件。
//! EventBus 广播事件 → SSE 端点推送给前端 → 前端实时更新 UI。
//!
//! **设计理念**：
//! - 后端是唯一的数据真相源
//! - 所有状态变更通过事件通知
//! - 前端只是一个"投影"
//!
//! 无论变更来自前端 REST 调用还是 Agent 直接操作，都走同一条事件通道。
//! 前端不需要区分变更来源——收到事件就更新 UI。

use serde::Serialize;

use crate::model::Block;

// ─── BlockEvent 枚举 ─────────────────────────────────────────

/// Block 变更事件
///
/// 每种 mutation 对应一种事件变体，携带完整的 Block 数据供前端直接更新 UI。
/// 所有事件包含 `document_id` 字段，SSE 端点按文档过滤推送。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BlockEvent {
    /// Block 创建（create_block / create_document）
    BlockCreated {
        document_id: String,
        /// 编辑者标识：前端会话级 UUID，用于 SSE 回声去重
        #[serde(skip_serializing_if = "Option::is_none")]
        editor_id: Option<String>,
        #[serde(flatten)]
        block: Block,
    },
    /// Block 更新（update_block — 内容/属性变更）
    BlockUpdated {
        document_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        editor_id: Option<String>,
        #[serde(flatten)]
        block: Block,
    },
    /// Block 删除（软删除，可能级联）
    BlockDeleted {
        document_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        editor_id: Option<String>,
        block_id: String,
        cascade_count: u32,
    },
    /// Block 移动（改变父块和/或位置）
    BlockMoved {
        document_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        editor_id: Option<String>,
        #[serde(flatten)]
        block: Block,
    },
    /// Block 恢复（从已删除状态恢复）
    BlockRestored {
        document_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        editor_id: Option<String>,
        #[serde(flatten)]
        block: Block,
    },
    /// 批量操作完成（前端应 refetch 整个文档）
    BlocksBatchChanged {
        document_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        editor_id: Option<String>,
    },
}

impl BlockEvent {
    /// 获取事件所属的文档 ID
    pub fn document_id(&self) -> &str {
        match self {
            Self::BlockCreated { document_id, .. } => document_id,
            Self::BlockUpdated { document_id, .. } => document_id,
            Self::BlockDeleted { document_id, .. } => document_id,
            Self::BlockMoved { document_id, .. } => document_id,
            Self::BlockRestored { document_id, .. } => document_id,
            Self::BlocksBatchChanged { document_id, .. } => document_id,
        }
    }

    /// 获取 SSE `event:` 字段值
    pub fn event_type(&self) -> &'static str {
        match self {
            Self::BlockCreated { .. } => "block_created",
            Self::BlockUpdated { .. } => "block_updated",
            Self::BlockDeleted { .. } => "block_deleted",
            Self::BlockMoved { .. } => "block_moved",
            Self::BlockRestored { .. } => "block_restored",
            Self::BlocksBatchChanged { .. } => "blocks_batch_changed",
        }
    }
}
