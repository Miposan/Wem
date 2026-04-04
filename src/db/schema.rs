//! SQLite Schema 定义
//!
//! 所有建表 DDL、索引、PRAGMA 配置集中管理。
//! MVP 阶段只建 blocks 表（oplog/snapshots 等后续 Phase 添加）。
//!
//! 参考 06-storage.md §1~§2

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
/// 16 个字段 + 外键约束 + UNIQUE 约束
/// 参考 06-storage.md §2.1
pub const CREATE_BLOCKS_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS blocks (
    id              TEXT PRIMARY KEY,                -- 20 位 Block ID
    parent_id       TEXT NOT NULL,                   -- 父块 ID（Document 根节点指向自身）
    position        TEXT NOT NULL,                   -- Fractional Index（字符串，字典序排序）
    block_type      TEXT NOT NULL,                   -- JSON: {"type":"heading","level":2}
    content_type    TEXT NOT NULL,                   -- markdown / empty / query
    content         BLOB DEFAULT X'',                -- 块内容（空值用 X''）
    properties      TEXT DEFAULT '{}',               -- JSON 属性
    version         INTEGER NOT NULL DEFAULT 1,      -- 乐观锁版本号
    status          TEXT NOT NULL DEFAULT 'normal',   -- normal / draft / deleted
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
/// 7 个索引覆盖所有主要查询场景
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
    // 加密块过滤
    "CREATE INDEX IF NOT EXISTS idx_blocks_encrypted ON blocks(encrypted) WHERE encrypted = 1;",
];
