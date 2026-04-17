import type { BlockNode } from '@/types/api'
import type { BlockRendererProps } from '../core/types'
import { ParagraphBlock } from '../blocks/ParagraphBlock'
import { HeadingBlock } from '../blocks/HeadingBlock'
import { ThematicBreakBlock } from '../blocks/ThematicBreakBlock'
import { UnknownBlock } from '../blocks/UnknownBlock'

// ─── Block Type → Component 路由 ───

interface ContentRouterProps extends BlockRendererProps {
  block: BlockNode
}

function BlockContentRouter({ block, ...props }: ContentRouterProps) {
  switch (block.block_type.type) {
    case 'paragraph':
      return <ParagraphBlock block={block} {...props} />
    case 'heading':
      return <HeadingBlock block={block} {...props} />
    case 'thematicBreak':
      return <ThematicBreakBlock />
    // Phase 2 块类型占位
    case 'blockquote':
    case 'list':
    case 'listItem':
    case 'codeBlock':
    case 'mathBlock':
    case 'callout':
    case 'image':
    case 'audio':
    case 'video':
    case 'iframe':
    case 'embed':
    case 'attributeView':
    case 'widget':
    case 'document':
    default:
      return <UnknownBlock block={block} />
  }
}

// ─── BlockContainer ───

interface BlockContainerProps extends BlockRendererProps {
  block: BlockNode
}

/** 判断一个块是否可折叠（heading 且有子块） */
function isCollapsible(block: BlockNode): boolean {
  return block.block_type.type === 'heading' && block.children.length > 0
}

export function BlockContainer({ block, collapsedIds, onToggleCollapse, ...props }: BlockContainerProps) {
  const collapsible = isCollapsible(block)
  const collapsed = collapsedIds.has(block.id)

  return (
    <div
      className={`wem-block-container ${collapsible ? 'wem-block-collapsible' : ''} ${collapsed ? 'wem-block-collapsed' : ''}`}
      data-block-id={block.id}
      data-block-type={block.block_type.type}
    >
      <div className="wem-block-content">
        {/* 块左侧操作区（折叠、拖拽等） — 在 content 内部以正确对齐 */}
        <div className="wem-block-gutter" contentEditable={false}>
          {collapsible && (
            <button
              className="wem-gutter-btn"
              onClick={() => onToggleCollapse(block.id)}
              title={collapsed ? '展开子块' : '折叠子块'}
            >
              <span className={`wem-collapse-arrow ${collapsed ? 'collapsed' : ''}`}>▶</span>
            </button>
          )}
        </div>
        <div className="wem-block-editable">
          <BlockContentRouter block={block} {...props} />
        </div>
      </div>

      {/* 子块（折叠时隐藏） */}
      {block.children.length > 0 && !collapsed && (
        <div className="wem-block-children">
          {block.children.map((child) => (
            <BlockContainer
              key={child.id}
              block={child}
              collapsedIds={collapsedIds}
              onToggleCollapse={onToggleCollapse}
              {...props}
            />
          ))}
        </div>
      )}
    </div>
  )
}
