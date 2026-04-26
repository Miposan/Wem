import { memo } from 'react'
import type { BlockNode } from '@/types/api'
import type { BlockRendererProps } from '../core/types'
import { BlockContainer } from './BlockContainer'

/**
 * 递归渲染 BlockNode[] 树
 */
const BlockTreeRenderer = memo(function BlockTreeRenderer({
  blocks,
  ...props
}: { blocks: BlockNode[] } & BlockRendererProps) {
  return (
    <div className="wem-block-tree">
      {blocks.map((block) => (
        <BlockContainer key={block.id} block={block} {...props} />
      ))}
    </div>
  )
}, (prev, next) => {
  if (prev.blocks.length !== next.blocks.length) return false
  for (let i = 0; i < prev.blocks.length; i++) {
    if (prev.blocks[i] !== next.blocks[i]) return false
  }
  if (prev.readonly !== next.readonly) return false
  if (prev.placeholder !== next.placeholder) return false
  if (prev.collapsedIds !== next.collapsedIds) return false
  if (prev.selectedBlockIds !== next.selectedBlockIds) return false
  if (prev.dragState !== next.dragState) return false
  if (prev.dragHandlers !== next.dragHandlers) return false
  if (prev.onToggleCollapse !== next.onToggleCollapse) return false
  if (prev.onContentChange !== next.onContentChange) return false
  if (prev.onAction !== next.onAction) return false
  if (prev.onBlockContextMenu !== next.onBlockContextMenu) return false
  return true
})

export { BlockTreeRenderer }
