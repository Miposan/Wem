import type { BlockNode, BlockType } from '@/types/api'

// ─── Editor Selection ───

/** 跨块选区：锚点 + 焦点，均以 { blockId, offset } 定位 */
export interface EditorSelection {
  anchorBlockId: string
  anchorOffset: number
  focusBlockId: string
  focusOffset: number
}

// ─── Drag-Drop ───

/** 拖拽放置目标位置 */
export type DropPosition = 'before' | 'after' | 'child'

/** 当前拖拽放置目标 */
export interface DropTarget {
  /** 放置目标块 ID */
  blockId: string
  /** 放置位置：目标块之前 / 之后 / 作为子块 */
  position: DropPosition
}

/** 拖拽状态（WemEditor 维护，下传给 BlockContainer） */
export interface DragState {
  /** 正在拖拽的块 ID */
  draggingBlockId: string | null
  /** 当前放置目标 */
  dropTarget: DropTarget | null
}

// ─── Block Actions ───

/** 文本块键盘/交互操作 */
export type BlockAction =
  | { type: 'split' }
  | { type: 'delete'; blockId: string }
  | { type: 'merge-with-previous'; blockId: string }
  | { type: 'focus-previous'; blockId: string }
  | { type: 'focus-next'; blockId: string }
  /** Markdown 快捷键转换块类型（如 `## ` → heading），同时更新内容 */
  | { type: 'convert-block'; blockId: string; content: string; blockType: BlockType }
  /** 跨块选区删除 */
  | { type: 'delete-range'; blockIds: string[] }
  /** 块拖拽移动（普通块 / 无子节点的 heading） */
  | { type: 'move-block'; blockId: string; target: DropTarget }
  /** heading 子树整体拖拽移动（折叠 heading + 其下属内容） */
  | { type: 'move-heading-tree'; blockId: string; target: DropTarget }

// ─── Component Props ───

/** 文本块组件共享 Props */
export interface TextBlockProps {
  block: BlockNode
  readonly: boolean
  placeholder?: string
  /** 被选中的块 ID 集合（跨块选中时使用） */
  selectedBlockIds: ReadonlySet<string>
  onContentChange: (blockId: string, content: string) => void
  onAction: (action: BlockAction) => void
}

/** BlockContainer / BlockTreeRenderer 共享 Props */
export interface BlockRendererProps {
  readonly: boolean
  placeholder?: string
  collapsedIds: Set<string>
  /** 当前跨块选区（null = 无选区） */
  selection: EditorSelection | null
  /** 被选中的块 ID 集合（从 selection 派生，便于 BlockContainer 快速判断） */
  selectedBlockIds: ReadonlySet<string>
  /** 拖拽状态 */
  dragState: DragState
  /** 拖拽事件处理函数 */
  dragHandlers: DragHandlers
  onContentChange: (blockId: string, content: string) => void
  onAction: (action: BlockAction) => void
  onToggleCollapse: (blockId: string) => void
  onSelectionChange: (selection: EditorSelection | null) => void
}

/** 拖拽事件处理函数（由 useBlockDrag 返回，传入 BlockContainer） */
export interface DragHandlers {
  onDragStart: (e: React.DragEvent, blockId: string) => void
  onDragOver: (e: React.DragEvent, blockId: string) => void
  onDragLeave: (e: React.DragEvent, blockId: string) => void
  onDrop: (e: React.DragEvent, blockId: string) => void
  onDragEnd: (e: React.DragEvent) => void
}
