import type { BlockNode } from '@/types/api'
import type { BlockRendererProps } from '../core/types'
import { ParagraphBlock } from '../blocks/ParagraphBlock'
import { HeadingBlock } from '../blocks/HeadingBlock'
import { ThematicBreakBlock } from '../blocks/ThematicBreakBlock'
import { CodeBlock } from '../blocks/CodeBlock'
import { ListItemBlock } from '../blocks/ListItemBlock'
import { BlockquoteBlock } from '../blocks/BlockquoteBlock'
import { getHeadingLevel } from '../core/BlockOperations'
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
    case 'codeBlock':
      return <CodeBlock block={block} {...props} />
    case 'list':
      // List 是纯结构容器，不需要渲染内容（子项通过 .wem-block-children 渲染）
      return null
    case 'listItem':
      return <ListItemBlock block={block} {...props} />
    case 'blockquote':
      return <BlockquoteBlock block={block} {...props} />
    // Phase 2 块类型占位
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

// ─── Drop Indicator ───

/** 拖拽放置指示器 — 在块之间显示蓝色线条 */
function DropIndicator({ position }: { position: 'before' | 'after' }) {
  return <div className={`wem-drop-indicator wem-drop-indicator-${position}`} />
}

// ─── BlockContainer ───

interface BlockContainerProps extends BlockRendererProps {
  block: BlockNode
}

/** 判断一个块是否可折叠（heading 且有子块） */
function isCollapsible(block: BlockNode): boolean {
  return (block.block_type.type === 'heading' || block.block_type.type === 'list') && block.children.length > 0
}

/** 获取 List 块的 ordered 属性（用于 CSS data 属性） */
function getListOrdered(block: BlockNode): string | undefined {
  if (block.block_type.type === 'list') {
    return String(block.block_type.ordered)
  }
  return undefined
}

export function BlockContainer({
  block,
  collapsedIds,
  dragState,
  dragHandlers,
  onToggleCollapse,
  onBlockContextMenu,
  ...props
}: BlockContainerProps) {
  const { selectedBlockIds } = props
  const collapsible = isCollapsible(block)
  const collapsed = collapsedIds.has(block.id)
  const selected = selectedBlockIds.has(block.id)
  const isDragging = dragState.draggingBlockId === block.id
  const isDropTarget = dragState.dropTarget?.blockId === block.id
  const className = [
    'wem-block-container',
    `wem-block-type-${block.block_type.type}`,
    collapsible && 'wem-block-collapsible',
    collapsed && 'wem-block-collapsed',
    selected && 'wem-block-selected',
    isDragging && 'wem-block-dragging',
    isDropTarget && 'wem-block-drop-target',
  ]
    .filter(Boolean)
    .join(' ')

  return (
    <div
      className={className}
      data-block-id={block.id}
      data-block-type={block.block_type.type}
      data-heading-level={getHeadingLevel(block.block_type) ?? undefined}
      data-list-ordered={getListOrdered(block)}
      onDragOver={(e) => dragHandlers.onDragOver(e, block.id)}
      onDragLeave={(e) => dragHandlers.onDragLeave(e, block.id)}
      onDrop={(e) => dragHandlers.onDrop(e, block.id)}
      onContextMenu={(e) => onBlockContextMenu?.(e, block)}
    >
      {/* 放置指示器 — before 位置 */}
      {isDropTarget && dragState.dropTarget?.position === 'before' && (
        <DropIndicator position="before" />
      )}

      <div className="wem-block-content">
        <div className="wem-block-editable">
          {/* 块左侧操作区（折叠、拖拽等） — 绝对定位于 editable 左侧，紧贴文字 */}
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
            {/* List 类型切换按钮 */}
            {block.block_type.type === 'list' && !props.readonly && (
              <button
                className="wem-gutter-btn wem-list-toggle-btn"
                onClick={() => props.onAction({ type: 'toggle-list-type', blockId: block.id })}
                title={block.block_type.ordered ? '切换为无序列表' : '切换为有序列表'}
                tabIndex={-1}
              >
                {block.block_type.ordered ? '≡' : '☰'}
              </button>
            )}
            {/* 拖拽手柄 */}
            {!props.readonly && (
              <button
                className="wem-gutter-btn wem-drag-handle"
                draggable
                onDragStart={(e) => dragHandlers.onDragStart(e, block.id)}
                onDragEnd={dragHandlers.onDragEnd}
                title="拖拽移动块"
                tabIndex={-1}
              >
                ⋮⋮
              </button>
            )}
          </div>
          <BlockContentRouter
            block={block}
            collapsedIds={collapsedIds}
            dragState={dragState}
            dragHandlers={dragHandlers}
            onToggleCollapse={onToggleCollapse}
            {...props}
          />
        </div>
      </div>

      {/* 放置指示器 — after 位置 */}
      {isDropTarget && dragState.dropTarget?.position === 'after' && (
        <DropIndicator position="after" />
      )}

      {/* 子块（折叠时隐藏） */}
      {block.children.length > 0 && !collapsed && (
        <div className="wem-block-children">
          {block.children.map((child) => (
            <BlockContainer
              key={child.id}
              block={child}
              collapsedIds={collapsedIds}
              dragState={dragState}
              dragHandlers={dragHandlers}
              onToggleCollapse={onToggleCollapse}
              {...props}
            />
          ))}
        </div>
      )}
    </div>
  )
}
