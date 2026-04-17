import type { BlockNode } from '@/types/api'
import type { BlockRendererProps } from '../core/types'
import { BlockContainer } from './BlockContainer'

/**
 * 递归渲染 BlockNode[] 树
 */
export function BlockTreeRenderer({
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
}
