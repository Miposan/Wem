import type { BlockNode } from '@/types/api'

/**
 * 未知块类型降级渲染
 */
export function UnknownBlock({ block }: { block: BlockNode }) {
  return (
    <div className="wem-unknown-block">
      <p className="text-muted-foreground text-sm">
        不支持的块类型: {block.block_type.type}
      </p>
      {block.content && (
        <pre className="text-xs text-muted-foreground mt-1 overflow-hidden">
          {block.content.slice(0, 200)}
        </pre>
      )}
    </div>
  )
}
