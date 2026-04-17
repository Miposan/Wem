import type { BlockNode } from '@/types/api'

// ─── Block Actions ───

/** 文本块键盘/交互操作 */
export type BlockAction =
  | { type: 'split'; blockId: string; offset: number }
  | { type: 'delete'; blockId: string }
  | { type: 'merge-with-previous'; blockId: string }
  | { type: 'focus-previous'; blockId: string }
  | { type: 'focus-next'; blockId: string }

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
