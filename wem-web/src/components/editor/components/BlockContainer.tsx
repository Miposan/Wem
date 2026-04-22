import type { BlockNode } from '@/types/api'
import type { BlockRendererProps, DropPosition } from '../core/types'
import { ParagraphBlock } from '../blocks/ParagraphBlock'
import { HeadingBlock } from '../blocks/HeadingBlock'
import { ThematicBreakBlock } from '../blocks/ThematicBreakBlock'
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

// ─── Drop Indicator ───

/** 拖拽放置指示器 — 在块之间显示蓝色线条 */
function DropIndicator({ position }: { position: DropPosition }) {
  if (position === 'child') {
    // child 模式：在块下方显示缩进的蓝色指示区
    return <div className="wem-drop-indicator-child" />
  }
  // before/after：在块的顶/底边缘显示蓝色线
  return <div className={`wem-drop-indicator wem-drop-indicator-${position}`} />
}

// ─── BlockContainer ───

interface BlockContainerProps extends BlockRendererProps {
  block: BlockNode
}

/** 判断一个块是否可折叠（heading 且有子块） */
function isCollapsible(block: BlockNode): boolean {
  return block.block_type.type === 'heading' && block.children.length > 0
}

/** 判断一个块是否为容器（可接收子块） */
function isContainerBlock(block: BlockNode): boolean {
  return block.block_type.type === 'heading'
}

export function BlockContainer({
  block,
  collapsedIds,
  dragState,
  dragHandlers,
  onToggleCollapse,
  ...props
}: BlockContainerProps) {
  const { selectedBlockIds } = props
  const collapsible = isCollapsible(block)
  const collapsed = collapsedIds.has(block.id)
  const selected = selectedBlockIds.has(block.id)
  const isDragging = dragState.draggingBlockId === block.id
  const isDropTarget = dragState.dropTarget?.blockId === block.id

  return (
    <div
      className={`wem-block-container ${collapsible ? 'wem-block-collapsible' : ''} ${collapsed ? 'wem-block-collapsed' : ''} ${selected ? 'wem-block-selected' : ''} ${isDragging ? 'wem-block-dragging' : ''} ${isDropTarget ? 'wem-block-drop-target' : ''}`}
      data-block-id={block.id}
      data-block-type={block.block_type.type}
      data-heading-level={getHeadingLevel(block.block_type) ?? undefined}
      onDragOver={(e) => dragHandlers.onDragOver(e, block.id)}
      onDragLeave={(e) => dragHandlers.onDragLeave(e, block.id)}
      onDrop={(e) => dragHandlers.onDrop(e, block.id)}
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
          <BlockContentRouter block={block} {...props} />
        </div>
      </div>

      {/* 放置指示器 — after 位置 */}
      {isDropTarget && dragState.dropTarget?.position === 'after' && (
        <DropIndicator position="after" />
      )}

      {/* 放置指示器 — child 位置（在 children 区域显示） */}
      {isDropTarget && dragState.dropTarget?.position === 'child' && (
        <DropIndicator position="child" />
      )}

      {/* 子块（折叠时隐藏） */}
      {block.children.length > 0 && !collapsed && (
        <div className="wem-block-children">
          {block.children.map((child) => (
            <BlockContainer
              key={child.id}
              block={child}
              collapsedIds={collapsedIds}
              selectedBlockIds={selectedBlockIds}
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
