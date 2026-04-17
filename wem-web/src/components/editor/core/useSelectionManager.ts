/**
 * 跨块选区管理 hook
 *
 * 在编辑器根节点监听 mousedown → mousemove → mouseup，
 * 当鼠标跨越块边界时进入自定义选区模式。
 *
 * 设计原则：
 * - 块内选区由浏览器原生 Selection 处理，hook 不干预
 * - 跨块时接管选区：隐藏原生光标，显示自定义高亮
 * - mouseup 后选区定稿，直到用户点击/按键清除
 */

import { useCallback, useRef } from 'react'
import type { BlockNode } from '@/types/api'
import type { EditorSelection } from './types'
import {
  getBlockIdFromPoint,
  getOffsetFromMouseEvent,
} from './EditorSelection'

interface UseSelectionManagerOptions {
  /** 获取当前块树快照 */
  getTree: () => BlockNode[]
  /** 选区变更回调 */
  onSelectionChange: (selection: EditorSelection | null) => void
}

/**
 * 跨块选区 hook
 *
 * 返回需要绑定到编辑器根容器的事件处理器。
 * 挂载方式：`<div {...selectionHandlers}>`
 */
export function useSelectionManager({
  getTree,
  onSelectionChange,
}: UseSelectionManagerOptions) {
  // ── 拖拽选区状态 ──
  const isDragging = useRef(false)
  const anchorRef = useRef<{ blockId: string; offset: number } | null>(null)

  /**
   * mousedown：记录锚点。
   * 如果后续 mousemove 跨越了块边界，isDragging 变为 true，
   * 进入自定义选区模式。
   */
  const handleMouseDown = useCallback(
    (e: React.MouseEvent) => {
      // 只响应主键（左键）
      if (e.button !== 0) return

      const blockId = getBlockIdFromPoint(e.target as Node)
      if (!blockId) {
        // 点击在空白区域 → 清除选区
        anchorRef.current = null
        isDragging.current = false
        onSelectionChange(null)
        return
      }

      const offset = getOffsetFromMouseEvent(e.nativeEvent, blockId)
      anchorRef.current = { blockId, offset }
      isDragging.current = false // 尚未跨越块边界

      // 清除之前的跨块选区
      onSelectionChange(null)
    },
    [onSelectionChange],
  )

  /**
   * mousemove：如果已按下鼠标且跨越了块边界，更新选区焦点。
   */
  const handleMouseMove = useCallback(
    (e: React.MouseEvent) => {
      if (!anchorRef.current) return
      // 仅在鼠标按下状态（buttons & 1）
      if (!(e.buttons & 1)) return

      const blockId = getBlockIdFromPoint(e.target as Node)
      if (!blockId) return

      const offset = getOffsetFromMouseEvent(e.nativeEvent, blockId)

      // 检测是否跨越块边界
      if (blockId !== anchorRef.current.blockId) {
        isDragging.current = true
      }

      if (isDragging.current) {
        // 跨块选区模式：隐藏浏览器原生选区
        const sel = window.getSelection()
        if (sel) sel.removeAllRanges()

        onSelectionChange({
          anchorBlockId: anchorRef.current.blockId,
          anchorOffset: anchorRef.current.offset,
          focusBlockId: blockId,
          focusOffset: offset,
        })
      }
    },
    [onSelectionChange],
  )

  /**
   * mouseup：结束选区拖拽。
   * 选区状态保持不变（直到下次 mousedown 或键盘操作清除）。
   */
  const handleMouseUp = useCallback(() => {
    if (isDragging.current) {
      isDragging.current = false
      // 选区保持，直到下一次点击清除
    }
    anchorRef.current = null
  }, [])

  /**
   * 清除选区（供外部调用，如键盘导航时）
   */
  const clearSelection = useCallback(() => {
    isDragging.current = false
    anchorRef.current = null
    onSelectionChange(null)
  }, [onSelectionChange])

  return {
    /** 绑定到编辑器根容器的事件处理器 */
    selectionHandlers: {
      onMouseDown: handleMouseDown,
      onMouseMove: handleMouseMove,
      onMouseUp: handleMouseUp,
    },
    /** 程序化清除选区 */
    clearSelection,
    /** 当前是否处于跨块拖拽中（供拖拽 hook 判断冲突） */
    isSelecting: isDragging,
  }
}
