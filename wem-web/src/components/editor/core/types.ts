import type { BlockNode, BlockType } from '@/types/api'

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

// ─── Component Props ───

/** 文本块组件共享 Props */
export interface TextBlockProps {
  block: BlockNode
  readonly: boolean
  placeholder?: string
  onContentChange: (blockId: string, content: string) => void
  onAction: (action: BlockAction) => void
}

/** BlockContainer / BlockTreeRenderer 共享 Props */
export interface BlockRendererProps {
  readonly: boolean
  placeholder?: string
  collapsedIds: Set<string>
  onContentChange: (blockId: string, content: string) => void
  onAction: (action: BlockAction) => void
  onToggleCollapse: (blockId: string) => void
}
