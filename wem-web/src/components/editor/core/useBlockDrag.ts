/**
 * useBlockDrag — 块拖拽 Hook
 *
 * 基于 HTML5 Drag and Drop API 实现块拖拽移动。
 * 支持嵌套容器（heading 子块）：可拖入/拖出 heading 容器。
 *
 * 职责：
 * - 管理拖拽状态（draggingBlockId、dropTarget）
 * - 计算 drop target 位置（before/after/child）
 * - 校验合法性（不可拖入自身后代）
 * - 统一走 move-block，heading 单独移动，子块由吸收逻辑决定
 */

import { useCallback, useRef, useState } from 'react'
import type { BlockNode } from '@/types/api'
import type { BlockAction, DragHandlers, DragState, DropPosition, DropTarget } from './types'
import { findBlockById, flattenTree } from './BlockOperations'

// ─── 配置常量 ───

/** 鼠标距离块底部多少像素内判定为"放在块之后" */
const AFTER_THRESHOLD = 8
/** 鼠标距离左侧缩进多少像素内判定为"拖入容器作为子块" */
const CHILD_INDENT_THRESHOLD = 40

// ─── Hook 参数 ───

export interface UseBlockDragOptions {
  /** 获取当前块树快照 */
  getTree: () => BlockNode[]
  /** 拖拽完成时发出 action */
  onAction: (action: BlockAction) => void
  /** 判断块是否处于折叠状态（折叠 heading 拖拽走 heading-tree） */
  isCollapsed: (blockId: string) => boolean
}

// ─── 辅助函数 ───

/** 判断块是否为容器（可接收子块） */
function isContainerBlock(block: BlockNode): boolean {
  return block.block_type.type === 'heading'
}

/** 获取块的所有后代 ID（用于防止拖入自身后代） */
function getDescendantIds(block: BlockNode): Set<string> {
  const ids = new Set<string>()
  function walk(children: BlockNode[]) {
    for (const child of children) {
      ids.add(child.id)
      walk(child.children)
    }
  }
  walk(block.children)
  return ids
}

/** 判断 blockId 是否为 targetBlockId 的后代 */
function isDescendantOf(tree: BlockNode[], blockId: string, ancestorId: string): boolean {
  const ancestor = findBlockById(tree, ancestorId)
  if (!ancestor) return false
  const descendants = getDescendantIds(ancestor)
  return descendants.has(blockId)
}

// ─── Hook ───

export function useBlockDrag(options: UseBlockDragOptions) {
  const { getTree, onAction, isCollapsed } = options

  const [dragState, setDragState] = useState<DragState>({
    draggingBlockId: null,
    dropTarget: null,
  })

  /** 拖拽开始时的块 ID（通过 dataTransfer 持久化） */
  const dragBlockIdRef = useRef<string | null>(null)

  /**
   * 根据鼠标位置和目标块计算放置位置。
   *
   * 逻辑：
   * - 鼠标在块上半部分 → before
   * - 鼠标在块下半部分（普通块或非容器） → after
   * - 鼠标在块下半部分 + 目标是容器块 + 鼠标有缩进 → child（作为容器第一个子块）
   *
   * @param e 拖拽事件
   * @param targetBlockId 目标块 ID
   */
  const computeDropPosition = useCallback(
    (e: React.DragEvent, targetBlockId: string): DropPosition | null => {
      const tree = getTree()
      const targetBlock = findBlockById(tree, targetBlockId)
      if (!targetBlock) return null

      const rect = (e.currentTarget as HTMLElement).getBoundingClientRect()
      const relativeY = e.clientY - rect.top
      const blockHeight = rect.height
      const midPoint = blockHeight / 2

      const relativeX = e.clientX - rect.left
      const distToBottom = blockHeight - relativeY

      // 鼠标在块最底部 AFTER_THRESHOLD 内 → after（作为兄弟放在块之后）
      // 这优先级最高，确保即使在容器块底部也能放置到容器后面
      if (distToBottom <= AFTER_THRESHOLD) {
        return 'after'
      }

      // 上半部分 → before
      if (relativeY < midPoint - AFTER_THRESHOLD) {
        return 'before'
      }

      // 下半部分 + 容器块 → child（拖入容器作为子块）
      if (isContainerBlock(targetBlock)) {
        if (relativeX > CHILD_INDENT_THRESHOLD || targetBlock.children.length > 0) {
          return 'child'
        }
      }

      // 其余 → after
      return 'after'
    },
    [getTree],
  )

  /**
   * 校验放置是否合法。
   * 规则：
   * - 不能放到自己上面（无意义）
   * - 不能放到自己的后代上面（会破坏树结构）
   * - 拖拽块与目标块的父级关系要合理
   */
  const isValidDrop = useCallback(
    (dragBlockId: string, targetBlockId: string, position: DropPosition): boolean => {
      // 不能放到自己上
      if (dragBlockId === targetBlockId) return false

      const tree = getTree()

      // 只有 child 位置才会产生循环（拖入自己的后代作为子块），需要阻止
      // before/after 位置只是放在后代旁边，后端会自己处理父子关系
      if (position === 'child' && isDescendantOf(tree, targetBlockId, dragBlockId)) {
        return false
      }

      return true
    },
    [getTree],
  )

  // ─── 拖拽事件处理 ───

  const onDragStart = useCallback(
    (e: React.DragEvent, blockId: string) => {
      const tree = getTree()
      const block = findBlockById(tree, blockId)
      if (!block) return

      // 扁平列表只有一个块 → 不允许拖拽
      const flat = flattenTree(tree)
      if (flat.length <= 1) {
        e.preventDefault()
        return
      }

      dragBlockIdRef.current = blockId

      // 设置拖拽数据（必须有才能触发后续事件）
      e.dataTransfer.effectAllowed = 'move'
      e.dataTransfer.setData('text/plain', blockId)

      // 设置拖拽预览（使用块本身，稍微偏移）
      // 浏览器默认使用拖拽元素作为预览

      setDragState({
        draggingBlockId: blockId,
        dropTarget: null,
      })
    },
    [getTree],
  )

  const onDragOver = useCallback(
    (e: React.DragEvent, targetBlockId: string) => {
      e.preventDefault()
      e.stopPropagation()

      const dragBlockId = dragBlockIdRef.current
      if (!dragBlockId) return

      const position = computeDropPosition(e, targetBlockId)
      if (!position) return

      if (!isValidDrop(dragBlockId, targetBlockId, position)) {
        e.dataTransfer.dropEffect = 'none'
        setDragState((prev) =>
          prev.draggingBlockId === dragBlockId
            ? { ...prev, dropTarget: null }
            : prev,
        )
        return
      }

      e.dataTransfer.dropEffect = 'move'

      setDragState((prev) => {
        // 性能优化：相同目标不更新
        if (
          prev.dropTarget?.blockId === targetBlockId &&
          prev.dropTarget?.position === position
        ) {
          return prev
        }
        return {
          draggingBlockId: dragBlockId,
          dropTarget: { blockId: targetBlockId, position },
        }
      })
    },
    [computeDropPosition, isValidDrop],
  )

  const onDragLeave = useCallback(
    (e: React.DragEvent, _blockId: string) => {
      // 只有离开当前块时才清除（不是进入子元素）
      const relatedTarget = e.relatedTarget as HTMLElement | null
      const currentTarget = e.currentTarget as HTMLElement
      if (relatedTarget && currentTarget.contains(relatedTarget)) {
        return
      }

      setDragState((prev) => {
        if (prev.dropTarget?.blockId === _blockId) {
          return { ...prev, dropTarget: null }
        }
        return prev
      })
    },
    [],
  )

  const onDrop = useCallback(
    (e: React.DragEvent, targetBlockId: string) => {
      e.preventDefault()
      e.stopPropagation()

      const dragBlockId = dragBlockIdRef.current
      if (!dragBlockId || dragBlockId === targetBlockId) return

      const position = computeDropPosition(e, targetBlockId)
      if (!position) return

      if (!isValidDrop(dragBlockId, targetBlockId, position)) return

      const target: DropTarget = { blockId: targetBlockId, position }

      // 清除拖拽状态
      dragBlockIdRef.current = null
      setDragState({ draggingBlockId: null, dropTarget: null })

      // 折叠 heading（有子块）拖到 before/after 位置 → move-heading-tree（整体移动子树）
      // 其余情况（child 位置、非折叠、非 heading）→ move-block
      // move_heading_tree 不支持 target_parent_id，无法处理 child 语义
      const tree = getTree()
      const draggedBlock = findBlockById(tree, dragBlockId)
      if (
        draggedBlock &&
        draggedBlock.block_type.type === 'heading' &&
        draggedBlock.children.length > 0 &&
        isCollapsed(dragBlockId) &&
        target.position !== 'child'
      ) {
        onAction({ type: 'move-heading-tree', blockId: dragBlockId, target })
      } else {
        onAction({ type: 'move-block', blockId: dragBlockId, target })
      }
    },
    [onAction],
  )

  const onDragEnd = useCallback((_e: React.DragEvent) => {
    // 无论是否成功 drop，都清理状态
    dragBlockIdRef.current = null
    setDragState({ draggingBlockId: null, dropTarget: null })
  }, [])

  const dragHandlers: DragHandlers = {
    onDragStart,
    onDragOver,
    onDragLeave,
    onDrop,
    onDragEnd,
  }

  return { dragState, dragHandlers }
}
