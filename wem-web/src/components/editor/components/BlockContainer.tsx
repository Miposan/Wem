import { memo } from 'react'
import type { BlockNode } from '@/types/api'
import type { BlockRendererProps } from '../core/types'
import { ParagraphBlock } from '../blocks/ParagraphBlock'
import { HeadingBlock } from '../blocks/HeadingBlock'
import { ThematicBreakBlock } from '../blocks/ThematicBreakBlock'
import { CodeBlock } from '../blocks/CodeBlock'
import { ListItemBlock } from '../blocks/ListItemBlock'
import { BlockquoteBlock } from '../blocks/BlockquoteBlock'
import { MathBlock } from '../blocks/MathBlock'
import { TableBlock } from '../blocks/TableBlock'
import { ImageBlock } from '../blocks/ImageBlock'
import { VideoBlock } from '../blocks/VideoBlock'
import { EmbedBlock } from '../blocks/EmbedBlock'
import { AudioBlock } from '../blocks/AudioBlock'
import { getHeadingLevel } from '../core/BlockOperations'
import { UnknownBlock } from '../blocks/UnknownBlock'
import { ChevronRight, GripVertical, List, ListOrdered, Plus } from 'lucide-react'
import { focusBlock } from '../core/SelectionManager'
import { EditorErrorBoundary } from '../core/EditorErrorBoundary'

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
    case 'mathBlock':
      return <MathBlock block={block} readonly={props.readonly} onContentChange={props.onContentChange} />
    case 'table':
      return <TableBlock block={block} readonly={props.readonly} onContentChange={props.onContentChange} onAction={props.onAction} />
    case 'image':
      return <ImageBlock block={block} readonly={props.readonly} onContentChange={props.onContentChange} onAction={props.onAction} />
    case 'video':
      return <VideoBlock block={block} readonly={props.readonly} />
    case 'iframe':
    case 'embed':
      return <EmbedBlock block={block} readonly={props.readonly} />
    case 'audio':
      return <AudioBlock block={block} readonly={props.readonly} />
    case 'callout':
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

/** 非文本块类型：没有 contentEditable，需要 BlockContainer 层面的焦点管理 */
const NON_TEXT_BLOCK_TYPES = new Set([
  'thematicBreak', 'mathBlock', 'image', 'video', 'audio', 'iframe', 'embed',
])

function isNonTextBlock(block: BlockNode): boolean {
  return NON_TEXT_BLOCK_TYPES.has(block.block_type.type)
}

/** 获取 List 块的 ordered 属性（用于 CSS data 属性） */
function getListOrdered(block: BlockNode): string | undefined {
  if (block.block_type.type === 'list') {
    return String(block.block_type.ordered)
  }
  return undefined
}

function setsEqual<T>(a: Set<T>, b: Set<T>): boolean {
  if (a.size !== b.size) return false
  for (const v of a) if (!b.has(v)) return false
  return true
}

const BlockContainer = memo(function BlockContainer({
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
  const nonText = isNonTextBlock(block)
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

  // 非文本块：为 .wem-block-content 添加 tabIndex 以支持键盘导航
  const contentTabProps = nonText ? {
    tabIndex: -1 as const,
    onKeyDown: (e: React.KeyboardEvent) => {
      if (e.key === 'Backspace' || e.key === 'Delete') {
        e.preventDefault()
        props.onAction({ type: 'delete', blockId: block.id })
      }
      if (e.key === 'ArrowUp') {
        e.preventDefault()
        props.onAction({ type: 'focus-previous', blockId: block.id })
      }
      if (e.key === 'ArrowDown') {
        e.preventDefault()
        props.onAction({ type: 'focus-next', blockId: block.id })
      }
    },
  } : {}

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
      onClick={(e) => {
        const target = e.target as HTMLElement
        // 只响应块自身内容区域的点击，忽略从子块冒泡上来的事件
        // 避免跨块选区结束后 click 落在公共祖先上导致光标跳转
        const ownContent = (e.currentTarget as HTMLElement).querySelector(':scope > .wem-block-content')
        if (ownContent?.contains(target) && !target.closest('[contenteditable], textarea, input, select')) {
          focusBlock(block.id)
        }
      }}
    >
      {/* 放置指示器 — before 位置 */}
      {isDropTarget && dragState.dropTarget?.position === 'before' && (
        <DropIndicator position="before" />
      )}

      <div className="wem-block-content" {...contentTabProps}>
        <div className="wem-block-editable">
          {/* 块左侧操作区（折叠、拖拽等） — 绝对定位于 editable 左侧，紧贴文字 */}
          <div className="wem-block-gutter" contentEditable={false}>
            {/* + 按钮：在当前块后添加新段落 */}
            {!props.readonly && (
              <button
                className="wem-gutter-btn wem-add-btn"
                onClick={() => props.onAction({ type: 'add-block-after', blockId: block.id })}
                title="添加块"
                tabIndex={-1}
              >
                <Plus className="h-3.5 w-3.5" />
              </button>
            )}
            {collapsible && (
              <button
                className="wem-gutter-btn"
                onClick={() => onToggleCollapse(block.id)}
                title={collapsed ? '展开子块' : '折叠子块'}
              >
                <ChevronRight className={`wem-collapse-arrow ${collapsed ? 'collapsed' : ''} h-3.5 w-3.5`} />
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
                {block.block_type.ordered ? <ListOrdered className="h-3.5 w-3.5" /> : <List className="h-3.5 w-3.5" />}
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
                <GripVertical className="h-3.5 w-3.5" />
              </button>
            )}
          </div>
          <EditorErrorBoundary blockId={block.id}>
            <BlockContentRouter
              block={block}
              collapsedIds={collapsedIds}
              dragState={dragState}
              dragHandlers={dragHandlers}
              onToggleCollapse={onToggleCollapse}
              {...props}
            />
          </EditorErrorBoundary>
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
}, (prev, next) => {
  if (prev.block !== next.block) return false
  if (prev.readonly !== next.readonly) return false
  if (prev.placeholder !== next.placeholder) return false
  if (!setsEqual(prev.collapsedIds, next.collapsedIds)) return false
  if (!setsEqual(prev.selectedBlockIds, next.selectedBlockIds)) return false
  if (prev.dragState !== next.dragState) return false
  if (prev.dragHandlers !== next.dragHandlers) return false
  if (prev.onToggleCollapse !== next.onToggleCollapse) return false
  if (prev.onContentChange !== next.onContentChange) return false
  if (prev.onAction !== next.onAction) return false
  if (prev.onBlockContextMenu !== next.onBlockContextMenu) return false
  return true
})

export { BlockContainer }
