/**
 * UndoManager — 编辑器撤销/重做管理器
 *
 * 基于快照的历史栈。每次结构变更（split/delete/merge/convert/move）
 * 前捕获 { tree, cursor } 快照压入 undo 栈。
 *
 * 内容变更（打字）使用防抖合并：同一块内 500ms 内的连续打字
 * 合并为一个 undo 条目。
 *
 * 设计：
 * - 快照包含 BlockNode[] 深拷贝 + 光标位置
 * - undo/redo 仅恢复客户端 UI 状态，后端状态已变更
 * - 对于结构操作的 undo，依赖后端 API 执行逆操作：
 *   undo-delete → restoreBlock, undo-split → merge 等
 * - 最大历史 50 条（环形缓冲）
 */

import type { BlockNode } from '@/types/api'
import { getCursorPosition } from './SelectionManager'

// ─── 类型 ───

export interface CursorSnapshot {
  blockId: string
  offset: number
}

export interface HistoryEntry {
  /** 快照时刻的块树（深拷贝） */
  tree: BlockNode[]
  /** 快照时刻的光标位置 */
  cursor: CursorSnapshot | null
}

export type OperationType =
  | 'split'
  | 'delete'
  | 'merge'
  | 'convert'
  | 'move'
  | 'delete-range'
  | 'content-change'

/** 操作记录 — 逆操作信息从快照树推导，无需额外存储 */
export interface OperationRecord {
  type: OperationType
  blockIds: string[]
}

export interface HistoryEntryWithMeta extends HistoryEntry {
  /** 记录是什么操作导致了这个快照（用于确定逆操作） */
  operation: OperationRecord
}

// ─── 配置 ───

const MAX_HISTORY = 50
const CONTENT_DEBOUNCE_MS = 500

// ─── UndoManager ───

export class UndoManager {
  private undoStack: HistoryEntryWithMeta[] = []
  private redoStack: HistoryEntryWithMeta[] = []

  /** 内容变更防抖 */
  private contentDebounceBlockId: string | null = null
  private contentDebounceTimer: ReturnType<typeof setTimeout> | null = null

  // ─── 查询 ───

  canUndo(): boolean {
    return this.undoStack.length > 0
  }

  canRedo(): boolean {
    return this.redoStack.length > 0
  }

  // ─── 压栈 ───

  /** 结构操作前调用：捕获当前状态压入 undo 栈，清空 redo 栈 */
  pushBeforeStructuralOp(tree: BlockNode[], operation: OperationRecord): void {
    this.undoStack.push({
      tree: deepCloneTree(tree),
      cursor: captureCursor(),
      operation,
    })
    this.redoStack = []
    this.trimStack()
  }

  /** 内容变更前调用。同一块 500ms 内连续打字合并为一个条目 */
  pushContentChange(tree: BlockNode[], blockId: string): void {
    // 同一块且在防抖窗口内 → 合并（不压新快照）
    if (
      this.contentDebounceBlockId === blockId &&
      this.contentDebounceTimer !== null
    ) {
      clearTimeout(this.contentDebounceTimer)
      this.contentDebounceTimer = setTimeout(() => {
        this.contentDebounceBlockId = null
        this.contentDebounceTimer = null
      }, CONTENT_DEBOUNCE_MS)
      return
    }

    this.pushBeforeStructuralOp(tree, {
      type: 'content-change',
      blockIds: [blockId],
    })

    this.contentDebounceBlockId = blockId
    if (this.contentDebounceTimer) clearTimeout(this.contentDebounceTimer)
    this.contentDebounceTimer = setTimeout(() => {
      this.contentDebounceBlockId = null
      this.contentDebounceTimer = null
    }, CONTENT_DEBOUNCE_MS)
  }

  // ─── Undo / Redo ───

  /** 撤销：弹出 undo 栈顶。调用方负责 pushRedo */
  undo(): HistoryEntryWithMeta | null {
    return this.undoStack.length > 0 ? this.undoStack.pop()! : null
  }

  /** 重做：弹出 redo 栈顶。调用方负责 pushUndo */
  redo(): HistoryEntryWithMeta | null {
    return this.redoStack.length > 0 ? this.redoStack.pop()! : null
  }

  /**
   * 将当前状态压入 redo 栈（undo 时调用）。
   */
  pushRedo(tree: BlockNode[], operation: OperationRecord): void {
    this.redoStack.push({
      tree: deepCloneTree(tree),
      cursor: captureCursor(),
      operation,
    })
    this.trimStack()
  }

  /**
   * 将当前状态压入 undo 栈（redo 时调用）。
   */
  pushUndo(tree: BlockNode[], operation: OperationRecord): void {
    this.undoStack.push({
      tree: deepCloneTree(tree),
      cursor: captureCursor(),
      operation,
    })
    this.trimStack()
  }

  // ─── 清理 ───

  clear(): void {
    this.undoStack = []
    this.redoStack = []
    this.contentDebounceBlockId = null
    if (this.contentDebounceTimer) {
      clearTimeout(this.contentDebounceTimer)
      this.contentDebounceTimer = null
    }
  }

  destroy(): void {
    this.clear()
  }

  // ─── 内部 ───

  private trimStack(): void {
    if (this.undoStack.length > MAX_HISTORY) {
      this.undoStack.splice(0, this.undoStack.length - MAX_HISTORY)
    }
    if (this.redoStack.length > MAX_HISTORY) {
      this.redoStack.splice(0, this.redoStack.length - MAX_HISTORY)
    }
  }
}

// ─── 工具函数 ───

function deepCloneTree(tree: BlockNode[]): BlockNode[] {
  return tree.map((block) => ({
    ...block,
    children: deepCloneTree(block.children),
  }))
}

function captureCursor(): CursorSnapshot | null {
  const pos = getCursorPosition()
  if (!pos) return null
  return { blockId: pos.blockId, offset: pos.offset }
}
