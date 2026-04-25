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
export type DropPosition = 'before' | 'after'

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
  | { type: 'merge-with-next'; blockId: string }
  | { type: 'focus-previous'; blockId: string }
  | { type: 'focus-next'; blockId: string }
  /** Markdown 快捷键转换块类型（如 `## ` → heading），同时更新内容 */
  | { type: 'convert-block'; blockId: string; content: string; blockType: BlockType }
  /** 跨块选区删除 */
  | { type: 'delete-range'; blockIds: string[] }
  /** 块拖拽移动 */
  | { type: 'move-block'; blockId: string; target: DropTarget }
  /** 折叠的 heading / list 整棵子树拖拽移动（后端 moveTree API） */
  | { type: 'move-heading-tree'; blockId: string; target: DropTarget }
  /** 切换 List 的有序/无序类型 */
  | { type: 'toggle-list-type'; blockId: string }
  /** 列表项缩进（Tab）：在当前 ListItem 下创建子 List + ListItem */
  | { type: 'indent-list-item'; blockId: string }
  /** 列表项反缩进（Shift+Tab）：提升到上级 List 或退出列表 */
  | { type: 'outdent-list-item'; blockId: string }
  /** 退出列表（空 ListItem 按 Enter）：删除 ListItem，在 List 后创建 Paragraph */
  | { type: 'exit-list'; blockId: string }
  /** 退出代码块（Ctrl/Cmd+Enter）：在代码块后创建 Paragraph */
  | { type: 'exit-code-block'; blockId: string; content: string }

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
  /** 右键菜单回调 */
  onBlockContextMenu?: BlockContextMenuHandler
}

/** 拖拽事件处理函数（由 useBlockDrag 返回，传入 BlockContainer） */
export interface DragHandlers {
  onDragStart: (e: React.DragEvent, blockId: string) => void
  onDragOver: (e: React.DragEvent, blockId: string) => void
  onDragLeave: (e: React.DragEvent, blockId: string) => void
  onDrop: (e: React.DragEvent, blockId: string) => void
  onDragEnd: (e: React.DragEvent) => void
}

/** 右键菜单处理函数 */
export type BlockContextMenuHandler = (
  e: React.MouseEvent,
  block: BlockNode,
) => void
