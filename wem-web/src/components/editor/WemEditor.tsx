/**
 * WemEditor — 块编辑器主组件
 *
 * 架构：
 *   用户操作 → useTextBlock → handleAction → OperationQueue → Command
 *                                                        ↓
 *                                               API 同步 (await splitBlock/...)
 *                                               更新 UI (flushSync + 真实数据)
 *                                               光标从 DOM 读取 (getCursorPosition)
 *                                               DOM 光标 (SelectionManager)
 *
 * 核心设计：
 * - 悲观更新：结构操作先等 API 返回，再用真实数据更新 UI
 * - Command 从 DOM 读取光标：OperationQueue 串行保证 DOM 始终最新
 * - OperationQueue 串行化所有结构变更操作，保证有序
 */

import { useCallback, useEffect, useRef, useState } from 'react'
import { flushSync } from 'react-dom'
import type { BlockNode } from '@/types/api'
import { getDocument, updateBlock, createBlock, deleteBlock, restoreBlock, mergeBlock, moveBlock, undoDocument, redoDocument } from '@/api/client'
import { BlockTreeRenderer } from './components/BlockTreeRenderer'
import { updateBlockInTree, removeBlock, flattenTree, findBlockById } from './core/BlockOperations'
import { OperationQueue } from './core/OperationQueue'
import {
  executeSplit,
  executeDelete,
  executeMerge,
  executeFocusPrevious,
  executeFocusNext,
  executeConvertBlock,
  executeMove,
  executeMoveHeadingTree,
} from './core/Commands'
import type { CommandContext } from './core/Commands'
import type { BlockAction, EditorSelection } from './core/types'
import { useDocumentSSE } from '@/hooks/useDocumentSSE'
import { syncBlockContent, focusBlock, findEditable } from './core/SelectionManager'
import { useSelectionManager } from './core/useSelectionManager'
import { useBlockDrag } from './core/useBlockDrag'
import { getSelectedBlockIdsSet } from './core/EditorSelection'

// ─── Props ───

export interface WemEditorProps {
  blocks: BlockNode[]
  documentId: string
  readonly?: boolean
  placeholder?: string
}

// ─── Main Component ───

export function WemEditor({
  blocks,
  documentId,
  readonly = false,
  placeholder = '输入 / 插入块…',
}: WemEditorProps) {
  const [tree, setTreeState] = useState<BlockNode[]>([])
  const [collapsedIds, setCollapsedIds] = useState<Set<string>>(new Set())
  const [selection, setSelection] = useState<EditorSelection | null>(null)
  const [selectedBlockIds, setSelectedBlockIds] = useState<Set<string>>(new Set())
  const treeRef = useRef<BlockNode[]>([])
  const pendingTimers = useRef<Map<string, ReturnType<typeof setTimeout>>>(new Map())

  // ─── 跨块选区 ───

  const handleSelectionChange = useCallback(
    (newSelection: EditorSelection | null) => {
      setSelection(newSelection)
      setSelectedBlockIds(getSelectedBlockIdsSet(treeRef.current, newSelection))
    },
    [],
  )

  const { selectionHandlers, clearSelection } = useSelectionManager({
    getTree: () => treeRef.current,
    onSelectionChange: handleSelectionChange,
  })

  // ─── 拖拽 action 转发 ref（解决 handleAction 循环依赖） ──
  //
  // useBlockDrag 需要在 handleAction 之前初始化（hooks 顺序固定），
  // 但 useBlockDrag 的 onAction 回调要调用 handleAction。
  // 用 ref 打破循环：ref 在 handleAction 定义后立即更新。

  const handleActionRef = useRef<(action: BlockAction) => void>(() => {})

  const handleDragAction = useCallback(
    (action: BlockAction) => {
      handleActionRef.current(action)
    },
    [],
  )

  const { dragState, dragHandlers } = useBlockDrag({
    getTree: () => treeRef.current,
    onAction: handleDragAction,
    isCollapsed: (id) => collapsedIds.has(id),
  })

  /**
   * SSE 回声去重：追踪自身发起的 editor_id。
   * 前端为每个结构操作生成唯一 editor_id，后端在 SSE 事件中原样回传。
   * 收到 SSE 事件时，若 editor_id 在此集合中，说明是自身操作的回声，跳过 refetch。
   */
  const pendingEditorIds = useRef<Set<string>>(new Set())

  /** 将 editor_id 延迟清理，避免 SSE 事件到达时已被删除导致误判为外部事件 */
  const addPendingOperationId = useCallback((id: string) => {
    pendingEditorIds.current.add(id)
    setTimeout(() => {
      pendingEditorIds.current.delete(id)
    }, 5000)
  }, [])

  /** 取消指定块的 debounce 保存定时器（split/merge/delete 前调用，防止覆盖新数据） */
  const cancelPendingSave = useCallback((blockId: string) => {
    const saves = pendingTimers.current
    if (saves.has(blockId)) {
      clearTimeout(saves.get(blockId)!)
      saves.delete(blockId)
    }
  }, [])

  const opQueue = useRef(
    new OperationQueue((err, label) => {
      console.error(`[opQueue] ${label} 失败:`, err)
    }),
  )

  // ─── Tree 更新封装 ───
  //
  // 关键设计：treeRef.current 是"单一真相源"。
  // 所有更新先作用于 treeRef.current，再同步给 React 触发 re-render。
  // 这样 OperationQueue 中连续的 Command 调用 getTree() 时，
  // 总能拿到最新状态（即使 React 还没处理上一次 setState）。

  /** 同步更新（flushSync），用于需要立即 DOM 的操作（split/delete/merge 的乐观更新） */
  const setTreeSync = useCallback((updater: (prev: BlockNode[]) => BlockNode[]) => {
    const next = updater(treeRef.current)
    treeRef.current = next
    flushSync(() => {
      setTreeState(next)
    })
  }, [])

  /** 异步更新（不用 flushSync），用于内容变更的乐观更新 */
  const setTreeAsync = useCallback((updater: (prev: BlockNode[]) => BlockNode[]) => {
    const next = updater(treeRef.current)
    treeRef.current = next
    setTreeState(next)
  }, [])

  // ─── 折叠控制 ───

  const handleToggleCollapse = useCallback((blockId: string) => {
    setCollapsedIds((prev) => {
      const next = new Set(prev)
      if (next.has(blockId)) {
        next.delete(blockId)
      } else {
        next.add(blockId)
      }
      return next
    })
  }, [])

  // ─── refs 同步 ───

  // 仅在 documentId 变化时从 props 同步数据
  useEffect(() => {
    treeRef.current = blocks
    setTreeState(blocks)
    setCollapsedIds(new Set())
    opQueue.current.clear()
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [documentId])

  // ─── SSE 实时事件订阅 ───
  //
  // 后端广播所有 mutation。自身操作已通过 OperationQueue 悲观更新 UI，
  // 同源事件通过 editor_id 匹配抑制，仅外部事件触发 refetch。

  // 结构性变更（创建/删除/移动/恢复）统一 refetch 整个文档
  const refetchDocument = useCallback(() => {
    if (!documentId) return
    getDocument(documentId)
      .then((res) => setTreeSync(() => res.blocks))
      .catch((err) => console.error('[SSE] refetch 失败:', err))
  }, [documentId, setTreeSync])

  /**
   * SSE 结构性事件处理：基于 editor_id 去重。
   *
   * 前端发起结构操作时生成唯一 editor_id 并记录到 pendingEditorIds。
   * 后端在 SSE 事件中原样回传该 editor_id。
   * 收到事件时，若 editor_id 在 pending 集合中 → 自身操作回声，跳过 refetch。
   * 否则 → 外部操作，触发 refetch。
   */
  const handleStructuralEvent = useCallback(
    (event: { editor_id?: string }) => {
      if (event.editor_id && pendingEditorIds.current.has(event.editor_id)) {
        console.log(`[SSE] 自身操作回声 (${event.editor_id})，跳过 refetch`)
        return
      }
      refetchDocument()
    },
    [refetchDocument],
  )

  useDocumentSSE(documentId, {
    // 块内容更新：增量合并到 tree + 同步 contentEditable DOM
    // 注意：后端 serde(flatten) 将 block 字段展平到事件顶层，直接用 event.id 访问
    onBlockUpdated: useCallback(
      (event) => {
        setTreeAsync((prev) =>
          updateBlockInTree(prev, event.id, {
            content: event.content,
            block_type: event.block_type,
            properties: event.properties,
            version: event.version,
            modified: event.modified,
          }),
        )
        // 正在编辑的块不覆盖 DOM，避免 SSE 回声重置光标
        const el = findEditable(event.id)
        if (!el || document.activeElement !== el) {
          syncBlockContent(event.id, event.content)
        }
      },
      [setTreeAsync],
    ),
    // 结构性变更：基于 editor_id 去重
    onBlockCreated: handleStructuralEvent,
    onBlockDeleted: handleStructuralEvent,
    onBlockMoved: handleStructuralEvent,
    onBlockRestored: handleStructuralEvent,
    onBlocksBatchChanged: handleStructuralEvent,
  })

  // ─── Command Context ───

  const makeContext = useCallback((editorId?: string): CommandContext => ({
    documentId,
    editorId,
    getTree: () => treeRef.current,
    setTreeSync,
    cancelPendingSave,
    refetchDocument: async () => {
      if (!documentId) return
      const res = await getDocument(documentId)
      setTreeSync(() => res.blocks)
    },
  }), [documentId, setTreeSync, cancelPendingSave])

  /** 结构操作入队辅助：生成 editor_id + 追踪 pending 集合 */
  const enqueueStructuralOp = useCallback(
    (label: string, execute: (ctx: CommandContext) => Promise<void>) => {
      opQueue.current.enqueue({
        label,
        execute: async () => {
          const editorId = crypto.randomUUID()
          addPendingOperationId(editorId)
          await execute(makeContext(editorId))
        },
      })
    },
    [makeContext, addPendingOperationId],
  )

  // ─── 内容变更（打字）→ debounce 保存 ───

  const handleContentChange = useCallback((blockId: string, content: string) => {
    // 乐观更新（不经过队列，打字是高频操作）
    setTreeAsync((prev) => updateBlockInTree(prev, blockId, { content }))

    // Debounce 保存到后端
    const timer = setTimeout(async () => {
      pendingTimers.current.delete(blockId)
      const editorId = crypto.randomUUID()
      addPendingOperationId(editorId)
      try {
        await updateBlock(blockId, { content, editor_id: editorId })
      } catch (err) {
        console.error('自动保存失败:', err)
      }
    }, 300)

    const saves = pendingTimers.current
    if (saves.has(blockId)) clearTimeout(saves.get(blockId)!)
    saves.set(blockId, timer)
  }, [setTreeAsync, addPendingOperationId])

  // 清理定时器
  useEffect(() => {
    const timers = pendingTimers.current
    return () => {
      timers.forEach((t) => clearTimeout(t))
      timers.clear()
    }
  }, [])

  // ─── 结构变更（Enter/Backspace 等）→ 通过队列序列化 ───

  const handleAction = useCallback(
    (action: BlockAction) => {
      // 任何操作都清除跨块选区
      if (selection) clearSelection()

      switch (action.type) {
        // 结构操作 → 入队串行执行 + 抑制 SSE 回声
        case 'split':
          enqueueStructuralOp('split', (ctx) =>
            executeSplit(ctx),
          )
          break
        case 'delete': {
          const { blockId } = action
          enqueueStructuralOp(`delete:${blockId}`, (ctx) =>
            executeDelete(ctx, { blockId }),
          )
          break
        }
        case 'merge-with-previous': {
          const { blockId } = action
          enqueueStructuralOp(`merge:${blockId}`, (ctx) =>
            executeMerge(ctx, { blockId }),
          )
          break
        }
        case 'convert-block': {
          const { blockId, content, blockType } = action
          enqueueStructuralOp(`convert:${blockId}`, (ctx) =>
            executeConvertBlock(ctx, { blockId, content, blockType }),
          )
          break
        }
        case 'delete-range': {
          const { blockIds } = action
          if (blockIds.length === 0) break
          enqueueStructuralOp('delete-range', async (ctx) => {
            for (const id of blockIds) {
              ctx.cancelPendingSave(id)
            }
            for (const id of blockIds) {
              await deleteBlock(id, ctx.editorId)
            }
            ctx.setTreeSync((prev) => {
              let result = prev
              for (const id of blockIds) {
                result = removeBlock(result, id)
              }
              return result
            })
            // 聚焦到被删除范围之前的块（如果存在）
            const flat = flattenTree(treeRef.current)
            if (flat.length > 0) {
              focusBlock(flat[0].id, 0)
            }
          })
          break
        }
        case 'move-block': {
          const { blockId, target } = action
          enqueueStructuralOp(`move:${blockId}`, (ctx) =>
            executeMove(ctx, { blockId, target }),
          )
          break
        }
        case 'move-heading-tree': {
          const { blockId, target } = action
          enqueueStructuralOp(`move-heading-tree:${blockId}`, (ctx) =>
            executeMoveHeadingTree(ctx, { blockId, target }),
          )
          break
        }
        // 导航操作 → 无 API 调用，无需队列
        case 'focus-previous':
          executeFocusPrevious(makeContext(), action.blockId)
          break
        case 'focus-next':
          executeFocusNext(makeContext(), action.blockId)
          break
      }
    },
    [makeContext, enqueueStructuralOp, selection, clearSelection],
  )

  // 同步 ref → useBlockDrag 的回调现在能调用最新的 handleAction
  handleActionRef.current = handleAction

  // ─── Undo / Redo ───
  //
  // 直接调用后端 oplog undo/redo，refetch 拿真实数据。

  /** 执行撤销 */
  const handleUndo = useCallback(async () => {
    if (opQueue.current.isRunning()) return
    try {
      await undoDocument(documentId)
      refetchDocument()
    } catch (err) {
      console.error('[undo] 失败:', err)
    }
  }, [documentId, refetchDocument])

  /** 执行重做 */
  const handleRedo = useCallback(async () => {
    if (opQueue.current.isRunning()) return
    try {
      await redoDocument(documentId)
      refetchDocument()
    } catch (err) {
      console.error('[redo] 失败:', err)
    }
  }, [documentId, refetchDocument])

  /** 键盘快捷键：Ctrl+Z / Ctrl+Shift+Z / Ctrl+Y */
  const handleEditorKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      // Ctrl+Z (undo)
      if ((e.ctrlKey || e.metaKey) && e.key === 'z' && !e.shiftKey) {
        e.preventDefault()
        handleUndo()
        return
      }
      // Ctrl+Shift+Z or Ctrl+Y (redo)
      if (
        ((e.ctrlKey || e.metaKey) && e.key === 'z' && e.shiftKey) ||
        ((e.ctrlKey || e.metaKey) && e.key === 'y')
      ) {
        e.preventDefault()
        handleRedo()
        return
      }
    },
    [handleUndo, handleRedo],
  )

  // ─── 空编辑器点击 → 创建初始段落 ───
  //
  // 当编辑器没有块时，点击空白区域自动创建一个 paragraph block 并聚焦。
  // 通过 closest('[data-block-id]') 判断点击是否落在已有块上，避免误触发。

  const handleEditorClick = useCallback(
    (e: React.MouseEvent) => {
      if (readonly) return
      // 点击在已有块上，忽略
      if ((e.target as HTMLElement).closest('[data-block-id]')) return
      // 树不为空，忽略
      if (treeRef.current.length > 0) return

      const editorId = crypto.randomUUID()
      addPendingOperationId(editorId)

      createBlock({
        parent_id: documentId,
        block_type: { type: 'paragraph' },
        content: '',
        content_type: 'markdown',
        editor_id: editorId,
      })
        .then((created) => {
          const newBlock: BlockNode = { ...created, children: [] }
          setTreeSync(() => [newBlock])
          requestAnimationFrame(() => focusBlock(created.id))
        })
        .catch((err) => console.error('[editor] 创建初始段落失败:', err))
    },
    [readonly, documentId, setTreeSync, addPendingOperationId],
  )

  return (
    <div
      className="wem-editor-root"
      onClick={handleEditorClick}
      onKeyDown={handleEditorKeyDown}
      {...selectionHandlers}
    >
      <BlockTreeRenderer
        blocks={tree}
        readonly={readonly}
        placeholder={placeholder}
        collapsedIds={collapsedIds}
        selection={selection}
        selectedBlockIds={selectedBlockIds}
        dragState={dragState}
        dragHandlers={dragHandlers}
        onToggleCollapse={handleToggleCollapse}
        onContentChange={handleContentChange}
        onAction={handleAction}
        onSelectionChange={handleSelectionChange}
      />
    </div>
  )
}
