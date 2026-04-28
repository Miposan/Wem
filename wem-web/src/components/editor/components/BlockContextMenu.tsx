/**
 * BlockContextMenu — 块级右键菜单
 *
 * 右键点击块时弹出，提供：
 * - 块信息（ID、类型、创建/修改时间）
 * - 复制/剪切/粘贴
 * - 删除
 * - 块类型转换
 */

import { useEffect, useRef, useState } from 'react'
import { ChevronRight } from 'lucide-react'
import type { BlockNode, BlockType } from '@/types/api'
import { makeParagraphType, makeHeadingType, makeListType, makeCodeBlockType, makeMathBlockType, makeTableType } from '@/types/api'

// ─── Types ───

export interface BlockContextMenuState {
  visible: boolean
  x: number
  y: number
  block: BlockNode | null
}

interface BlockContextMenuProps {
  state: BlockContextMenuState
  onClose: () => void
  onAction: (action: BlockContextAction) => void
}

export type BlockContextAction =
  | { type: 'delete'; blockId: string }
  | { type: 'copy'; blockId: string }
  | { type: 'cut'; blockId: string }
  | { type: 'duplicate'; blockId: string }
  | { type: 'convert'; blockId: string; blockType: BlockType }
  | { type: 'copy-id'; blockId: string }

// ─── Block type labels ───

const BLOCK_TYPE_LABELS: Record<string, string> = {
  paragraph: '段落',
  heading: '标题',
  blockquote: '引用',
  list: '列表',
  listItem: '列表项',
  codeBlock: '代码块',
  thematicBreak: '分隔线',
  mathBlock: '数学公式',
  table: '表格',
  image: '图片',
  audio: '音频',
  video: '视频',
  callout: '提示框',
  document: '文档',
}

function getBlockTypeLabel(blockType: BlockType): string {
  if (blockType.type === 'heading') return `H${(blockType as { level: number }).level}`
  return BLOCK_TYPE_LABELS[blockType.type] ?? blockType.type
}

/** 获取可转换的目标类型列表 */
function getConvertibleTypes(currentType: BlockType): { label: string; type: BlockType }[] {
  const types: { label: string; type: BlockType }[] = []
  const isCode = currentType.type === 'codeBlock'

  if (!isCode) {
    types.push({ label: '段落', type: makeParagraphType() })
    for (let i = 1; i <= 6; i++) {
      types.push({ label: `标题 ${i}`, type: makeHeadingType(i) })
    }
    types.push({ label: '无序列表', type: makeListType(false) })
    types.push({ label: '有序列表', type: makeListType(true) })
    types.push({ label: '引用块', type: { type: 'blockquote' } })
    types.push({ label: '代码块', type: makeCodeBlockType('') })
    types.push({ label: '公式块', type: makeMathBlockType() })
    types.push({ label: '表格', type: makeTableType() })
  } else {
    types.push({ label: '段落', type: makeParagraphType() })
  }

  // 过滤掉当前类型
  return types.filter((t) => {
    if (t.type.type !== currentType.type) return true
    if (t.type.type === 'heading' && currentType.type === 'heading') {
      return (t.type as { level: number }).level !== (currentType as { level: number }).level
    }
    return false
  })
}

// ─── Component ───

export function BlockContextMenu({ state, onClose, onAction }: BlockContextMenuProps) {
  const menuRef = useRef<HTMLDivElement>(null)
  // visible 变化时自动重置子菜单状态：用 key 强制重建组件
  const visibleKey = state.visible ? 'open' : 'closed'

  // 点击外部关闭
  useEffect(() => {
    if (!state.visible) return
    const handleClick = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        onClose()
      }
    }
    // 延迟绑定，避免当前右键事件立即触发关闭
    setTimeout(() => document.addEventListener('mousedown', handleClick), 0)
    return () => document.removeEventListener('mousedown', handleClick)
  }, [state.visible, onClose])

  // ESC 关闭
  useEffect(() => {
    if (!state.visible) return
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    document.addEventListener('keydown', handleKeyDown)
    return () => document.removeEventListener('keydown', handleKeyDown)
  }, [state.visible, onClose])

  if (!state.visible || !state.block) return null

  return (
    <BlockContextMenuInner
      key={visibleKey}
      state={state}
      menuRef={menuRef}
      onClose={onClose}
      onAction={onAction}
    />
  )
}

/** 内部菜单（每次 visible 变化时通过 key 重建，自然重置所有 useState） */
function BlockContextMenuInner({
  state,
  menuRef,
  onClose,
  onAction,
}: {
  state: BlockContextMenuState
  menuRef: React.RefObject<HTMLDivElement | null>
  onClose: () => void
  onAction: (action: BlockContextAction) => void
}) {
  const [showConvert, setShowConvert] = useState(false)
  const block = state.block!
  const blockId = block.id
  const convertibleTypes = getConvertibleTypes(block.block_type)

  // 计算菜单位置（防止溢出屏幕）
  const menuStyle = {
    left: Math.min(state.x, window.innerWidth - 200),
    top: Math.min(state.y, window.innerHeight - 400),
  }

  return (
    <div
      ref={menuRef}
      className="fixed z-[9999] bg-popover border border-border rounded-lg shadow-xl py-1 min-w-[180px]"
      style={menuStyle}
    >
      {/* 块信息头 */}
      <div className="px-3 py-2 border-b border-border">
        <div className="text-xs text-muted-foreground">
          <span className="font-medium text-foreground">{getBlockTypeLabel(block.block_type)}</span>
          <span className="ml-2 text-muted-foreground/60">{block.id.slice(0, 8)}…</span>
        </div>
        <div className="text-[10px] text-muted-foreground/50 mt-0.5">
          {new Date(block.created).toLocaleString()} · v{block.version}
        </div>
      </div>

      {/* 操作项 */}
      <MenuItem label="复制" shortcut="Ctrl+C" onClick={() => { onAction({ type: 'copy', blockId }); onClose() }} />
      <MenuItem label="剪切" shortcut="Ctrl+X" onClick={() => { onAction({ type: 'cut', blockId }); onClose() }} />
      <MenuItem label="复制块 ID" onClick={() => { onAction({ type: 'copy-id', blockId }); onClose() }} />
      <MenuItem label="复制为副本" onClick={() => { onAction({ type: 'duplicate', blockId }); onClose() }} />

      <div className="my-1 border-t border-border" />

      {/* 转换为子菜单 */}
      <div
        className="relative"
        onMouseEnter={() => setShowConvert(true)}
        onMouseLeave={() => setShowConvert(false)}
      >
        <MenuItem label="转换为…" hasSubmenu />
        {showConvert && (
          <div className="absolute left-full top-0 bg-popover border border-border rounded-lg shadow-xl py-1 min-w-[140px]">
            {convertibleTypes.map((ct) => (
              <MenuItem
                key={ct.label}
                label={ct.label}
                onClick={() => { onAction({ type: 'convert', blockId, blockType: ct.type }); onClose() }}
              />
            ))}
          </div>
        )}
      </div>

      <div className="my-1 border-t border-border" />

      <MenuItem
        label="删除"
        shortcut="Del"
        destructive
        onClick={() => { onAction({ type: 'delete', blockId }); onClose() }}
      />
    </div>
  )
}

// ─── MenuItem ───

function MenuItem({
  label,
  shortcut,
  hasSubmenu,
  destructive,
  onClick,
}: {
  label: string
  shortcut?: string
  hasSubmenu?: boolean
  destructive?: boolean
  onClick?: () => void
}) {
  return (
    <button
      className={`
        w-full flex items-center px-3 py-1.5 text-sm transition-colors cursor-pointer
        ${destructive
          ? 'text-destructive hover:bg-destructive/10'
          : 'text-popover-foreground hover:bg-accent'
        }
      `}
      onClick={onClick}
    >
      <span className="flex-1 text-left">{label}</span>
      {shortcut && (
        <span className="text-xs text-muted-foreground ml-4">{shortcut}</span>
      )}
      {hasSubmenu && (
        <ChevronRight className="h-3 w-3 text-muted-foreground ml-2" />
      )}
    </button>
  )
}


