# Block System 架构重构方案

## 目标

对外 API 不变（handler 层保持 blocks/* 和 documents/* 分组），
重构内部模块结构，解决三个核心问题：

1. **消除循环依赖** — block.rs ↔ heading.rs/document.rs/paragraph.rs
2. **统一事务管理** — 事务基础设施从 block.rs 拔出来
3. **职责分离** — repo 层纯持久化，巨型文件拆分

## 设计原则

1. **业务只走 Service** — CLI/Handler 不直连 Repo
2. **Repo 只做持久化** — 不含任何业务策略
3. **事务边界统一** — 不分散在各模块手写
4. **初始化与运行时分离** — init 和数据访问彻底分开
5. **巨型文件切片** — 按职责拆分，依赖单向

---

## 分层架构

```
Layer 0 — repo 层（纯持久化，零业务逻辑）
┌─────────────────────────────────────────────────┐
│  repo/init.rs          repo/mod.rs               │
│  init_db()             Db 类型别名 + lock_db()   │
│  init_memory_db()                                │
│  ensure_root_block()                             │
│                                                   │
│  repo/block_repo.rs      repo/oplog_repo.rs      │
│  blocks 表 CRUD          operations/changes 表   │
│  接收 &Connection        接收 &Connection        │
│  返回 rusqlite::Error    返回 rusqlite::Error    │
└────────────────────────┬────────────────────────┘
                         │ 被所有 service 模块调用
                         ↓
Layer 1 — 基础设施（无业务语义，无模块间依赖）
┌─────────────────────────────────────────────────┐
│  traits.rs                  helpers.rs           │
│  ┌─────────────────────┐   ┌──────────────────┐ │
│  │ MoveContext struct  │   │ run_in_transaction│ │
│  │ ExportDepth enum    │   │ finish_tx         │ │
│  │ BlockTypeOps trait  │   │ derive_document_id│ │
│  │ TreeMoveOps trait   │   │ derive_doc_id_    │ │
│  │                     │   │   from_parent     │ │
│  │ 只定义接口，         │   │ resolve_target_   │ │
│  │ 不引用任何 impl      │   │   parent          │ │
│  │                     │   │ validate_no_cycle │ │
│  │                     │   │ reparent_children │ │
│  │                     │   │ to_json           │ │
│  │                     │   │ merge_properties  │ │
│  └─────────────────────┘   └──────────────────┘ │
└─────────────────────────────────────────────────┘
                         ↑
                         │ 仅依赖 L0 + L1
          ┌──────────────┼──────────────┐
          ↓              ↓              ↓
Layer 2 — 类型特化（互相独立，互不引用）
┌──────────────┐ ┌──────────────┐ ┌──────────────┐ ┌──────────┐
│ heading.rs   │ │ document.rs  │ │ paragraph.rs │ │ batch.rs │
│ impl         │ │ impl         │ │ impl         │ │ batch_   │
│  BlockTypeOps│ │  BlockTypeOps│ │  BlockTypeOps│ │ operations│
│ build_flat_  │ │ move_tree    │ │ split_block  │ │ batch_   │
│  list        │ │ (TreeMoveOps)│ │ merge_block  │ │ create   │
│ reconstruct_ │ │ deduplicate_ │ │              │ │ update   │
│  tree        │ │  doc_name    │ │              │ │ delete   │
│ move_heading │ │ find_doc_by_ │ │              │ │ move     │
│  _flat/_tree │ │  name        │ │              │ │          │
└──────────────┘ └──────────────┘ └──────────────┘ └──────────┘
          ↑              ↑              ↑               ↑
          └──────────────┼──────────────┼───────────────┘
                         ↓              ↓
Layer 3 — 整合层
┌─────────────────────────────────────────────────┐
│  block.rs                                        │
│  CRUD + move_block + export + 分派函数            │
│  re-export: move_heading_tree, split_block, ...   │
└──────────────────────┬──────────────────────────┘
                       │
                       ↓
Layer 4 — 入口层（保持不变）
┌─────────────────────────────────────────────────┐
│  handler/block.rs     handler/document.rs        │
│  handler/oplog.rs     handler/event.rs           │
│  wem_cli.rs（走 service，不直连 repo）            │
└─────────────────────────────────────────────────┘
```

### 对外 API 入口（不变）

handler 和 CLI 继续按现有方式调用：

```rust
// handler/block.rs → /api/v1/blocks/*
use crate::service::block_system::block;
block::create_block(db, req)
block::move_block(db, id, req)
block::split_block(db, id, req)        // re-export 自 paragraph.rs
block::move_heading_tree(db, req)      // re-export 自 heading.rs
block::batch_operations(db, req)       // re-export 自 batch.rs
block::export_block(db, id, fmt, depth)

// handler/document.rs → /api/v1/documents/*
use crate::service::block_system::document;
document::create_document(db, title, ...)
document::get_document_content(db, doc_id)
document::move_document_tree(db, req)
document::export_text(db, doc_id, fmt)
document::import_text(db, req)
```

---

## 各层详细设计

### Layer 0 — repo 层

**职责**：纯 SQL 映射，零业务判断。

#### `repo/mod.rs`（~20 行）

运行时基础设施，不含初始化逻辑。

- `Db` 类型别名（`Arc<Mutex<Connection>>`）
- `lock_db()` — 获取数据库锁（含 poison recovery）

#### `repo/init.rs`（~100 行）

启动初始化，与运行时数据访问彻底分离。

- `init_db(path)` — 创建文件数据库（建表、索引、迁移、根块种子）
- `init_memory_db()` — 创建内存数据库（CLI 和测试用）
- `ensure_root_block(conn)` — 确保全局根块 "/" 存在（幂等）

#### `repo/block_repo.rs`（~900 行）

blocks 表的全部 CRUD 操作。

- 接收 `&Connection`（加锁由 service 层负责）
- 返回 `Result<T, rusqlite::Error>`（错误转换由 service 层负责）
- 函数命名：`find_xxx` / `insert_xxx` / `update_xxx`
- **不包含** `deduplicate_doc_name`（业务策略，搬至 service 层）

#### `repo/oplog_repo.rs`（不变）

operations / changes / snapshots 表的 CRUD。

---

### Layer 1 — 基础设施

**职责**：定义接口和共享工具，不含具体业务逻辑，不含对 L2+ 模块的任何引用。

#### `traits.rs`（~120 行）

类型定义和 trait 接口。零依赖（只引用 model / error）。

| 导出项           | 类型    | 说明                                      |
|------------------|---------|-------------------------------------------|
| `MoveContext`    | struct  | 移动操作的上下文，传递给类型钩子          |
| `ExportDepth`    | enum    | 导出深度控制（Children / Descendants）    |
| `BlockTypeOps`   | trait   | 类型行为钩子接口（validate / on_moved / on_type_changed / adjust_content） |
| `TreeMoveOps`    | trait   | 子树移动的类型特化接口（validate / resolve / pre / execute / post / build_changes） |

BlockTypeOps 钩子一览：

- `use_tree_move() -> bool` — 是否需要子树移动（Document = true）
- `use_flat_list_move() -> bool` — 是否走 flat-list 移动（Heading = true）
- `validate_on_create(block_type)` — 创建时校验（Heading 校验 level 1-6）
- `on_moved(conn, ctx)` — 移动后置处理（Heading 跨文档重建树）
- `adjust_content_on_update(conn, block, content)` — 更新时调整内容（Document 同步 title）
- `on_type_changed(conn, block_id, old_block, new_type)` — 类型变更后处理（Heading 重建树）

#### `helpers.rs`（~200 行）

共享工具函数。只依赖 repo 层（L0），不依赖任何 L2+ 模块。

| 函数                           | 说明                                      |
|--------------------------------|-------------------------------------------|
| `run_in_transaction(conn, f)`  | 事务包装：BEGIN IMMEDIATE → 执行 → COMMIT/ROLLBACK |
| `finish_tx(conn, result)`      | 事务提交或回滚                            |
| `derive_document_id(parent)`   | 从父块推断 document_id（Document → 自身 id，其他 → 继承） |
| `derive_document_id_from_parent(conn, parent_id)` | 从 parent_id 推断 document_id（不加载完整 Block） |
| `resolve_target_parent(conn, before_id, after_id, current_parent)` | 从 before/after 推导目标父块 |
| `validate_no_cycle(conn, id, target_parent, current_parent)` | 循环引用检测 |
| `reparent_children_to(conn, anchor, new_parent, update_doc_id)` | 子块 reparent + 位置重算 |
| `to_json(val)`                 | 安全 JSON 序列化                          |
| `merge_properties(current, new, mode)` | 属性合并或替换                    |

---

### Layer 2 — 类型特化

**职责**：各 BlockType 变体的具体行为实现。模块间互相不可见。

#### `heading.rs`（~350 行）

Heading 类型的完整行为。

- `impl BlockTypeOps for HeadingOps` — 钩子实现
  - `validate_on_create` — level 1-6 校验
  - `on_moved` — 跨文档移动后 flat-list 重建
  - `on_type_changed` — heading 变更后 flat-list 重建
- Flat-list 树操作
  - `build_flat_list(conn, doc_id)` — 前序遍历构建 flat list
  - `reconstruct_tree(conn, doc_id, flat)` — 栈算法重建 heading 层级树
  - `find_subtree_end(flat, start_idx)` — 定位子树边界
  - `move_heading_flat(conn, heading, before, after)` — 展开状态移动
  - `move_heading_tree(db, req)` — 折叠状态移动（含事务 + oplog + 事件）

#### `document.rs`（~450 行）

Document 类型的完整行为。对外公共 API 入口（handler/document.rs 直接调用）。

- `impl BlockTypeOps for DocumentOps` — 钩子实现
  - `adjust_content_on_update` — content 同步到 properties.title
  - `on_moved` — 跨文档移动后 document_id 级联更新
- `impl TreeMoveOps for DocumentTreeMove` — 子树移动
- 公共 API
  - `create_document(db, title, parent_id, after_id, editor_id)`
  - `list_root_documents(db)`
  - `get_document_content(db, doc_id)`
  - `get_document_children(db, doc_id)`
  - `move_document_tree(db, req)`
  - `export_text(db, doc_id, format)`
  - `import_text(db, req)`
  - `deduplicate_doc_name(conn, parent_id, title)` — 从 repo 搬来的业务策略
  - `find_doc_by_name(db, parent_id, name)` — 新增，供 CLI 使用

#### `paragraph.rs`（~240 行）

Paragraph 类型的完整行为。

- `impl BlockTypeOps for ParagraphOps` — 钩子实现（全部默认空操作）
- 公共 API
  - `split_block(db, id, req)` — 原子拆分
  - `merge_block(db, id, req)` — 原子合并

#### `batch.rs`（~350 行）

批量操作。

- `batch_operations(db, req)` — 最多 50 条操作，同一事务内执行
- 内部 helper：`batch_create_block` / `batch_update_block` / `batch_delete_block` / `batch_move_block`

---

### Layer 3 — 整合层

#### `block.rs`（~700 行）

系统的统一入口。唯一整合者——通过分派函数连接 L2 的各 trait impl。
对外提供 blocks/* 相关的公共 API（handler/block.rs 调用）。

**公共 API**：

| 函数            | 说明                  |
|-----------------|-----------------------|
| `create_block`  | 创建 Block            |
| `get_block`     | 获取单个 Block        |
| `update_block`  | 更新内容和/或属性     |
| `delete_block`  | 删除（子块提升）      |
| `delete_tree`   | 级联删除              |
| `restore_block` | 恢复软删除            |
| `move_block`    | 移动（含类型分派）    |
| `export_block`  | 导出子树              |

**re-export**（转发 L2 的公共 API）：

| 函数                | 来源          |
|---------------------|---------------|
| `move_heading_tree` | heading.rs    |
| `split_block`       | paragraph.rs  |
| `merge_block`       | paragraph.rs  |
| `batch_operations`  | batch.rs      |

**分派函数**（`match BlockType` → 路由到 L2 的 trait impl）：

| 函数                         | 说明                          |
|------------------------------|-------------------------------|
| `use_tree_move`              | 判断是否走子树移动            |
| `use_flat_list_move`         | 判断是否走 flat-list 移动     |
| `dispatch_tree_move`         | 路由到 DocumentTreeMove       |
| `dispatch_flat_list_move`    | 路由到 HeadingOps             |
| `validate_on_create`         | 路由到 HeadingOps             |
| `on_moved`                   | 路由到各类型 on_moved         |
| `adjust_content_on_update`   | 路由到 DocumentOps            |
| `on_type_changed`            | 路由到 HeadingOps             |

---

### Layer 4 — 入口层（不变）

#### `handler/*.rs`

HTTP 处理层，保持现有分组不变。

- `handler/block.rs` — `/api/v1/blocks/*` 路由
- `handler/document.rs` — `/api/v1/documents/*` 路由
- `handler/oplog.rs` — `/api/v1/documents/history|undo|redo`
- `handler/event.rs` — `/api/v1/health` + SSE

#### `wem_cli.rs`

命令行接口，只调用 service 层公共 API，不直连 repo。

---

## 依赖规则

| 层   | 可以依赖         | 不可以依赖               |
|------|------------------|--------------------------|
| L0   | rusqlite, model  | service 层任何模块       |
| L1   | L0 (repo)        | L2, L3, L4              |
| L2   | L0, L1           | 其他 L2 模块, L3        |
| L3   | L0, L1, L2       | L4                      |
| L4   | L2(document), L3(block) | L0 (repo), L1     |

说明：handler/document.rs 直接调 document.rs（L2），这是有意为之——
document.rs 是 documents/* 路由的 service 入口，不经过 block.rs 整合。
handler/block.rs 调 block.rs（L3），block.rs 内部再分派到 L2。

核心约束：**L2 模块之间互相不可见**。heading.rs 不知道 document.rs 的存在。
只有 block.rs（L3）通过分派函数连接它们。

---

## 循环依赖的消除

### 重构前

```
block.rs ←→ heading.rs    （heading 需要 run_in_transaction, derive_doc_id_from_parent）
block.rs ←→ document.rs   （document 需要同样的工具函数）
block.rs ←→ paragraph.rs  （同上）
```

heading.rs 中 `use super::block::{self, BlockTypeOps, MoveContext}` — 既用 block 的 trait，
又用 block 的工具函数，形成紧耦合。

### 重构后

```rust
// heading.rs — 只依赖 L0 + L1
use super::traits::{BlockTypeOps, MoveContext};
use super::helpers;

// document.rs — 只依赖 L0 + L1
use super::traits::{BlockTypeOps, TreeMoveOps, ExportDepth, MoveContext};
use super::helpers;

// paragraph.rs — 只依赖 L0 + L1
use super::traits::BlockTypeOps;
use super::helpers;

// batch.rs — 只依赖 L0 + L1
use super::helpers;
use super::traits::ExportDepth;

// block.rs — 唯一整合者，依赖 L0 + L1 + L2
use super::traits::{BlockTypeOps, TreeMoveOps, ExportDepth, MoveContext};
use super::helpers;
use super::heading::HeadingOps;      // 分派用
use super::document::DocumentOps;    // 分派用
use super::paragraph::ParagraphOps;  // 分派用
```

依赖方向：单向，无循环。

---

## CLI 修复

### 重构前

```rust
// wem_cli.rs — 直接调 repo（违反原则 1）
use wem_kernel::repo::block_repo::find_doc_by_parent_and_title;
let conn = repo::lock_db(db);
let doc = find_doc_by_parent_and_title(&conn, parent, name)?;
```

### 重构后

```rust
// document.rs — 新增 service 函数
pub fn find_doc_by_name(db: &Db, parent_id: &str, name: &str)
    -> Result<Option<Block>, AppError>

// wem_cli.rs — 走 service
let doc = document::find_doc_by_name(db, parent, name)?;
```

---

## 执行顺序

每步独立可验证，做完 `cargo test` 确认无回归。

| 步骤 | 变更                                  | 风险 |
|------|---------------------------------------|------|
| 1    | repo/mod.rs → repo/init.rs            | 低   |
| 2    | block_repo.rs dedup → document.rs     | 低   |
| 3    | CLI resolve_doc_arg 走 service        | 低   |
| 4    | block.rs → helpers.rs + traits.rs     | 中   |
| 5    | block.rs → batch.rs                   | 低   |
| 6    | 更新 heading/document/paragraph 的 import | 中 |
| 7    | 最终验证 cargo test                   | —    |

步骤 1+2+3 互相独立，可并行。
步骤 4 是核心改动（消除循环依赖）。
步骤 5 纯机械操作。

---

## 重构前后文件分布

### 重构前

| 文件     | 行数   | 问题                       |
|----------|--------|----------------------------|
| block.rs | ~2581  | 巨型文件，混合了所有职责   |
| repo/mod.rs | ~190 | 初始化与运行时混杂        |
| block_repo.rs | ~1470 | 含业务策略（dedup）    |

### 重构后

| 文件              | 行数(估) | 职责                         |
|-------------------|---------|------------------------------|
| repo/mod.rs       | ~20     | Db 类型 + lock_db            |
| repo/init.rs      | ~100    | 启动初始化                   |
| repo/block_repo.rs| ~900    | blocks 表纯持久化            |
| repo/oplog_repo.rs| 不变    | operations/changes 持久化    |
| traits.rs         | ~120    | trait 定义 + 类型            |
| helpers.rs        | ~200    | 事务 + 工具函数              |
| block.rs          | ~700    | CRUD + Move + 分派 + re-export + 测试 |
| heading.rs        | ~350    | heading 特化                 |
| document.rs       | ~450    | document 特化 + dedup        |
| paragraph.rs      | ~240    | paragraph 特化               |
| batch.rs          | ~350    | 批量操作                     |
| position.rs       | 不变    | 分数索引                     |
| oplog.rs          | 不变    | 操作日志                     |
| event.rs          | 不变    | 事件通知                     |
