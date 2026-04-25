//! SQLite Schema 定义
//!
//! 所有建表 DDL、索引、PRAGMA 配置集中管理。
//!
//! ## 表结构
//!
//! | 表 | 用途 | 生命周期 |
//! |----|------|----------|
//! | `blocks` | Block 主表（文档内容块） | 永久 |
//! | `operations` | 操作记录（undo/redo 单元） | 短期，GC 可清理 |
//! | `changes` | 块级变更记录（before/after 快照） | 随 operation 清理 |
//! | `snapshots` | 文档级快照（完整存档） | 长期，用户管理 |

// ─── PRAGMA 配置 ────────────────────────────────────────────────

/// SQLite 连接时执行的 PRAGMA 语句
///
/// - `journal_mode = WAL`：写前日志，支持并发读写
/// - `foreign_keys = ON`：启用外键约束
/// - `busy_timeout = 5000`：锁等待 5 秒
/// - `cache_size = -64000`：64MB 缓存
/// - `synchronous = NORMAL`：WAL 模式下足够安全
pub const PRAGMAS: &str = r#"
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;
PRAGMA busy_timeout = 5000;
PRAGMA cache_size = -64000;
PRAGMA synchronous = NORMAL;
"#;

// ─── blocks 表 ──────────────────────────────────────────────────

/// blocks 表建表语句
///
/// 15 个字段 + 外键约束 + UNIQUE 约束
pub const CREATE_BLOCKS_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS blocks (
    id              TEXT PRIMARY KEY,                -- 20 位 Block ID
    parent_id       TEXT NOT NULL,                   -- 父块 ID（Document 根节点指向自身）
    document_id     TEXT NOT NULL,                   -- 所属文档 ID（文档块指向自身）
    position        TEXT NOT NULL,                   -- Fractional Index（字符串，字典序排序）
    block_type      TEXT NOT NULL,                   -- JSON: {"type":"heading","level":2}
    content         BLOB DEFAULT X'',                -- 块内容（空值用 X''）
    properties      TEXT DEFAULT '{}',               -- JSON 属性
    version         INTEGER NOT NULL DEFAULT 1,      -- 乐观锁版本号
    status          TEXT NOT NULL DEFAULT 'normal',   -- normal / deleted
    schema_version  INTEGER NOT NULL DEFAULT 1,      -- 格式迁移版本
    author          TEXT NOT NULL DEFAULT 'system',   -- 创建者（不可变）
    owner_id        TEXT,                            -- 当前所有者 user_id（可变）
    encrypted       INTEGER NOT NULL DEFAULT 0,      -- 0=未加密, 1=已加密
    created         TEXT NOT NULL,                   -- ISO 8601 创建时间
    modified        TEXT NOT NULL,                   -- ISO 8601 修改时间
    FOREIGN KEY (parent_id) REFERENCES blocks(id) ON DELETE RESTRICT
);
"#;

/// blocks 表索引
///
/// 8 个索引覆盖所有主要查询场景
pub const CREATE_BLOCKS_INDEXES: &[&str] = &[
    // 唯一约束：同一父块下 position 不能重复（排除已软删除的块）
    "CREATE UNIQUE INDEX IF NOT EXISTS idx_blocks_parent_pos ON blocks(parent_id, position) WHERE status != 'deleted';",
    // 按父块找子块（最频繁，含 status 过滤 + position 排序）
    "CREATE INDEX IF NOT EXISTS idx_blocks_parent ON blocks(parent_id, status, position);",
    // 状态过滤（软删除过滤）
    "CREATE INDEX IF NOT EXISTS idx_blocks_status ON blocks(status);",
    // 类型查询（JSON 函数提取 type 值）
    "CREATE INDEX IF NOT EXISTS idx_blocks_type ON blocks(json_extract(block_type, '$.type'));",
    // 时间范围查询
    "CREATE INDEX IF NOT EXISTS idx_blocks_modified ON blocks(modified);",
    // 按 author 查询（Agent 操作审计）
    "CREATE INDEX IF NOT EXISTS idx_blocks_author ON blocks(author);",
    // 按 document_id 查询（替代递归 CTE，O(n) 等值查询）
    "CREATE INDEX IF NOT EXISTS idx_blocks_document_id ON blocks(document_id, status, position);",
    // 加密块过滤
    "CREATE INDEX IF NOT EXISTS idx_blocks_encrypted ON blocks(encrypted) WHERE encrypted = 1;",
    // 同层文档名唯一：同一 parent 下 document 类型不可重名（content 即标题）
    // 排除根块（id = parent_id）
    "CREATE UNIQUE INDEX IF NOT EXISTS idx_blocks_parent_doc_content ON blocks(parent_id, content) WHERE status != 'deleted' AND JSON_EXTRACT(block_type, '$.type') = 'document' AND id != parent_id;"
];

// ─── operations 表 ─────────────────────────────────────────────

/// operations 表建表语句
///
/// 每次用户操作产生一个 Operation（全局唯一 id）。
/// Operation 内记录所有受影响 Block 的 before/after 快照（changes 表）。
/// undo = 标记 undone=1 + 恢复 before；redo = 标记 undone=0 + 恢复 after。
pub const CREATE_OPERATIONS_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS operations (
    id              TEXT PRIMARY KEY,                -- 操作 ID（时间有序唯一 ID）
    document_id     TEXT NOT NULL,                   -- 所属文档 ID（undo/redo 按文档隔离）
    action          TEXT NOT NULL,                   -- create/update/delete/move/restore/split/merge/batch_ops/import
    description     TEXT,                            -- 操作描述（可选）
    timestamp       TEXT NOT NULL,                   -- ISO 8601 操作时间
    undone          INTEGER NOT NULL DEFAULT 0,      -- 0=未撤销, 1=已撤销
    editor_id       TEXT                             -- 编辑者标识（前端会话级 UUID，用于 SSE 回声去重）
);
"#;

/// operations 表索引
pub const CREATE_OPERATIONS_INDEXES: &[&str] = &[
    // 按文档查操作（undo/redo 查找最近的 operation）
    "CREATE INDEX IF NOT EXISTS idx_operations_document ON operations(document_id, timestamp DESC);",
    // 按时间排序（GC 清理旧 operation）
    "CREATE INDEX IF NOT EXISTS idx_operations_timestamp ON operations(timestamp DESC);",
];

// ─── changes 表 ─────────────────────────────────────────────────

/// changes 表建表语句
///
/// 一个 Block 在某次 Operation 中的变更记录。
/// before_data / after_data 存储 BlockSnapshot 的 JSON。
pub const CREATE_CHANGES_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS changes (
    id              INTEGER PRIMARY KEY AUTOINCREMENT, -- 自增 ID
    operation_id    TEXT NOT NULL,                     -- 所属操作 ID
    block_id        TEXT NOT NULL,                     -- 受影响的 Block ID
    change_type     TEXT NOT NULL,                     -- created/updated/deleted/moved/restored/reparented
    before_data     TEXT,                              -- 变更前快照 JSON（create 时为 NULL）
    after_data      TEXT,                              -- 变更后快照 JSON（delete 时为 NULL）
    FOREIGN KEY (operation_id) REFERENCES operations(id) ON DELETE CASCADE
);
"#;

/// changes 表索引
pub const CREATE_CHANGES_INDEXES: &[&str] = &[
    // 按操作查变更（undo/redo 核心操作）
    "CREATE INDEX IF NOT EXISTS idx_changes_operation ON changes(operation_id);",
    // 按 Block 查变更历史
    "CREATE INDEX IF NOT EXISTS idx_changes_block ON changes(block_id, id DESC);",
];

// ─── snapshots 表 ──────────────────────────────────────────────

/// snapshots 表建表语句
///
/// 文档级快照：某一时刻整篇文档所有 Block 的完整状态。
/// 用于版本存档和整档恢复，与 Operation（细粒度）互补。
pub const CREATE_SNAPSHOTS_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS snapshots (
    id              TEXT PRIMARY KEY,                -- 快照 ID（UUID v7）
    document_id     TEXT NOT NULL,                   -- 文档 ID
    name            TEXT NOT NULL,                   -- 快照名称（用户可编辑）
    timestamp       TEXT NOT NULL,                   -- 创建时间（ISO 8601）
    block_count     INTEGER NOT NULL DEFAULT 0,      -- Block 数量（冗余字段）
    data            TEXT NOT NULL DEFAULT '[]'       -- 完整快照数据（JSON 数组）
);
"#;

/// snapshots 表索引
pub const CREATE_SNAPSHOTS_INDEXES: &[&str] = &[
    // 按文档查快照列表
    "CREATE INDEX IF NOT EXISTS idx_snapshots_document ON snapshots(document_id, timestamp DESC);",
];
