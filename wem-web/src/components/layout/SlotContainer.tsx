/**
 * SlotContainer — 面板槽位容器
 *
 * 代表布局中的一个区域（left / right / top），可以包含多个 PanelContainer。
 * 支持可拖拽调整槽位宽度（left / right）。
 *
 * 面板的拖拽移动由 ActivityBar 负责（drag source + drop target）。
 */

import { useCallback, useRef, type ReactNode } from 'react'
import { useLayoutStore, type SlotPosition } from '@/stores/layoutStore'

// ─── Types ───

export interface SlotContainerProps {
  slot: SlotPosition
  children: ReactNode
}

// ─── Component ───

export function SlotContainer({ slot, children }: SlotContainerProps) {
  const { slots, setSlotSize } = useLayoutStore()
  const slotState = slots[slot]
  const slotRef = useRef<HTMLDivElement>(null)

  // ─── 尺寸调整（resize） ───

  const handleResizeStart = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault()
      const startX = e.clientX
      const startSize = slotState.size
      const el = slotRef.current

      // 拖拽期间禁用文本选择并设置全局光标
      document.body.style.cursor = 'col-resize'
      document.body.style.userSelect = 'none'

      const handleMove = (ev: MouseEvent) => {
        const dx = ev.clientX - startX
        const newSize = slot === 'left'
          ? Math.max(0, startSize + dx)
          : Math.max(0, startSize - dx)
        if (el) el.style.width = `${newSize}px`
      }

      const handleUp = (ev: MouseEvent) => {
        document.removeEventListener('mousemove', handleMove)
        document.removeEventListener('mouseup', handleUp)
        document.body.style.cursor = ''
        document.body.style.userSelect = ''
        const dx = ev.clientX - startX
        const newSize = slot === 'left'
          ? Math.max(0, startSize + dx)
          : Math.max(0, startSize - dx)
        setSlotSize(slot, newSize)
      }

      document.addEventListener('mousemove', handleMove)
      document.addEventListener('mouseup', handleUp)
    },
    [slot, slotState.size, setSlotSize],
  )

  // ─── 渲染 ───

  const isVertical = slot === 'left' || slot === 'right'
  return (
    <div
      ref={slotRef}
      className="relative flex flex-col shrink-0 bg-sidebar"
      style={{
        width: isVertical ? `${slotState.size}px` : '100%',
        height: slot === 'top' ? `${slotState.size}px` : '100%',
      }}
    >
      {/* 面板内容 */}
      <div className="flex-1 flex flex-col overflow-hidden">
        {children}
      </div>

      {/* 宽度调整手柄 */}
      {isVertical && (
        <div
          className={`absolute top-0 bottom-0 w-1 cursor-col-resize hover:bg-primary/30 transition-colors z-10 ${slot === 'left' ? 'right-0' : 'left-0'}`}
          onMouseDown={handleResizeStart}
        />
      )}
    </div>
  )
}
