/** wem-kernel 统一响应格式 */
export interface ApiResponse<T = unknown> {
  code: number
  msg: string
  data: T
}

// 分页相关类型已删除：MVP 阶段不需要分页

// ---- Block ----

/**
 * 后端 BlockType 使用 serde(tag = "type", rename_all = "camelCase")
 * 序列化为 { type: "document" } 或 { type: "heading", level: 1 } 等对象
 */
export interface DocumentBlockType { type: 'document' }
export interface HeadingBlockType { type: 'heading'; level: number }
export interface BlockquoteBlockType { type: 'blockquote' }
export interface ListBlockType { type: 'list'; ordered: boolean }
export interface ListItemBlockType { type: 'listItem' }
export interface CalloutBlockType { type: 'callout' }
export interface ParagraphBlockType { type: 'paragraph' }
export interface CodeBlockBlockType { type: 'codeBlock'; language: string }
export interface MathBlockBlockType { type: 'mathBlock' }
export interface ThematicBreakBlockType { type: 'thematicBreak' }
export interface ImageBlockType { type: 'image'; url: string }
export interface AudioBlockType { type: 'audio'; url: string }
export interface VideoBlockType { type: 'video'; url: string }
export interface IframeBlockType { type: 'iframe'; url: string }
export interface EmbedBlockType { type: 'embed' }
export interface AttributeViewBlockType { type: 'attributeView'; av_id: string }
export interface WidgetBlockType { type: 'widget' }

export type BlockType =
  | DocumentBlockType
  | HeadingBlockType
  | BlockquoteBlockType
  | ListBlockType
  | ListItemBlockType
  | CalloutBlockType
  | ParagraphBlockType
  | CodeBlockBlockType
  | MathBlockBlockType
  | ThematicBreakBlockType
  | ImageBlockType
  | AudioBlockType
  | VideoBlockType
  | IframeBlockType
  | EmbedBlockType
  | AttributeViewBlockType
  | WidgetBlockType

export interface Block {
  id: string
  parent_id: string
  position: string
  block_type: BlockType
  content_type: string
  content: string
  properties: Record<string, string>
  version: number
  status: 'normal' | 'draft' | 'deleted'
  schema_version: number
  encrypted: boolean
  created: string
  modified: string
  author: string
  owner_id?: string
}

/** 带子节点的树形 Block */
export interface BlockNode extends Block {
  children: BlockNode[]
}

// ---- Document ----

export interface CreateDocumentReq {
  title: string
  parent_id?: string
  after_id?: string
}

export interface DocumentContentResult {
  document: Block
  blocks: BlockNode[]
}

export interface DocumentChildrenResult {
  children: Block[]
}

// ---- Block CRUD ----

export interface CreateBlockReq {
  parent_id: string
  block_type: BlockType
  content_type?: string
  content?: string
  properties?: Record<string, string>
  after_id?: string
  editor_id?: string
}

export interface UpdateBlockReq {
  id: string
  content?: string
  block_type?: BlockType
  properties?: Record<string, string>
  properties_mode?: 'merge' | 'replace'
  editor_id?: string
}

export interface MoveBlockReq {
  id: string
  target_parent_id?: string
  before_id?: string
  after_id?: string
  editor_id?: string
}

export interface MoveTreeReq {
  id: string
  before_id?: string
  after_id?: string
  editor_id?: string
}

// ---- Batch ----

export interface BatchCreateOp {
  action: 'create'
  temp_id: string
  parent_id: string
  block_type: BlockType
  content_type?: string
  content?: string
  properties?: Record<string, string>
  after_id?: string
}

export interface BatchUpdateOp {
  action: 'update'
  block_id: string
  content?: string
  block_type?: BlockType
  properties?: Record<string, string>
  properties_mode?: 'merge' | 'replace'
}

export interface BatchDeleteOp {
  action: 'delete'
  block_id: string
}

export interface BatchMoveOp {
  action: 'move'
  block_id: string
  target_parent_id?: string
  before_id?: string
  after_id?: string
}

export type BatchOp = BatchCreateOp | BatchUpdateOp | BatchDeleteOp | BatchMoveOp

export interface BatchReq {
  operations: BatchOp[] // 上限 50
  editor_id?: string
}

export interface BatchOpResult {
  action: string
  block_id: string
  version?: number
  error?: string
}

export interface BatchResult {
  id_map: Record<string, string> // temp_id → real_id
  results: BatchOpResult[]
}

// ---- Import / Export ----

export interface ImportTextReq {
  format: string
  content: string
  parent_id?: string
  after_id?: string
  title?: string
}

export interface ParseWarning {
  line?: number
  message: string
}

export interface ImportResult {
  root: Block
  blocks_imported: number
  warnings: ParseWarning[]
}

export interface ExportResult {
  content: string
  filename?: string
  blocks_exported: number
  lossy_types: string[]
}

// ---- Version / History ----

export interface DeleteResult {
  id: string
  document_id: string
  version: number
  cascade_count: number
}

export interface RestoreResult {
  id: string
  document_id: string
  version: number
  cascade_count: number
}

export interface HistoryEntry {
  version: number
  timestamp: string
  operation: string
  summary?: string
}

// ---- Undo / Redo ----

export interface UndoRedoResult {
  operation_id: string
  affected_block_ids: string[]
  affected_document_ids: string[]
  action: string
}

// ---- Split / Merge 意图 API ----

export interface SplitReq {
  /** Block ID */
  id: string
  /** 光标前的内容（更新当前块） */
  content_before: string
  /** 光标后的内容（创建新块） */
  content_after: string
  /** 新块的类型（不传则默认 paragraph） */
  new_block_type?: BlockType
  /** 是否将新块嵌套为当前块的子块（heading Enter 时为 true） */
  nest_under_parent?: boolean
  editor_id?: string
}

export interface SplitResult {
  /** 更新后的原块 */
  updated_block: Block
  /** 新创建的块 */
  new_block: Block
}

export interface MergeReq {
  /** Block ID */
  id: string
  /** 合并方向（默认 "previous"） */
  direction?: string
  /** 前一个兄弟块的当前内容（校验用，可选） */
  prev_content?: string
  editor_id?: string
}

export interface MergeResult {
  /** 合并后的前驱兄弟块 */
  merged_block: Block
  /** 被删除的块 ID */
  deleted_block_id: string
}

// ---- RPC 请求类型（全 POST） ----

export interface GetDocumentReq {
  id: string
}

export interface GetChildrenReq {
  id: string
}

export interface DeleteDocumentReq {
  id: string
  editor_id?: string
}

export interface ExportReq {
  id: string
  format?: string
}

export interface GetBlockReq {
  id: string
  include_deleted?: boolean
}

export interface DeleteBlockReq {
  id: string
  editor_id?: string
}

export interface RestoreReq {
  id: string
  editor_id?: string
}

export interface GetHistoryReq {
  id: string
  limit?: number
}


