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
}

export interface UpdateBlockReq {
  content?: string
  block_type?: BlockType
  properties?: Record<string, string>
  properties_mode?: 'merge' | 'replace'
  version: number // 乐观锁必填
}

export interface MoveBlockReq {
  target_parent_id?: string
  before_id?: string
  after_id?: string
  version: number // 乐观锁必填
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
  version: number
  content?: string
  block_type?: BlockType
  properties?: Record<string, string>
  properties_mode?: 'merge' | 'replace'
}

export interface BatchDeleteOp {
  action: 'delete'
  block_id: string
  version: number
}

export interface BatchMoveOp {
  action: 'move'
  block_id: string
  version: number
  target_parent_id?: string
  before_id?: string
  after_id?: string
}

export type BatchOp = BatchCreateOp | BatchUpdateOp | BatchDeleteOp | BatchMoveOp

export interface BatchReq {
  operations: BatchOp[] // 上限 50
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
  version: number
  cascade_count: number
}

export interface RestoreResult {
  id: string
  version: number
  cascade_count: number
}

export interface HistoryEntry {
  version: number
  timestamp: string
  operation: string
  summary?: string
}

export interface RollbackReq {
  target_version: number
  current_version: number
}
