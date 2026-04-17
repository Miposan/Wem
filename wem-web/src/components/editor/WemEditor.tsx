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
import { getDocument, updateBlock, createBlock, deleteBlock, restoreBlock, mergeBlock, moveBlock } from '@/api/client'
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
} from './core/Commands'
import type { CommandContext } from './core/Commands'
import type { BlockAction, EditorSelection } from './core/types'
import { useDocumentSSE } from '@/hooks/useDocumentSSE'
import { syncBlockContent, focusBlock, findEditable } from './core/SelectionManager'
import { useSelectionManager } from './core/useSelectionManager'
import { useBlockDrag } from './core/useBlockDrag'
import { getSelectedBlockIdsSet } from './core/EditorSelection'
import { UndoManager } from './core/UndoManager'
import type { OperationRecord, HistoryEntryWithMeta } from './core/UndoManager'

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
  const undoManager = useRef(new UndoManager())

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
    (action: { type: 'move-block'; blockId: string; target: { blockId: string; position: 'before' | 'after' | 'child' } }) => {
      handleActionRef.current(action)
    },
    [],
  )

  const { dragState, dragHandlers } = useBlockDrag({
    getTree: () => treeRef.current,
    onAction: handleDragAction,
  })

  /**
   * SSE 回声去重：追踪自身发起的操作 ID。
   * 前端为每个结构操作生成唯一 operation_id，后端在 SSE 事件中原样回传。
   * 收到 SSE 事件时，若 operation_id 在此集合中，说明是自身操作的回声，跳过 refetch。
   */
  const pendingOperationIds = useRef<Set<string>>(new Set())

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
    undoManager.current.clear()
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [documentId])

  // ─── SSE 实时事件订阅 ───
  //
  // 后端广播所有 mutation。自身操作已通过 OperationQueue 悲观更新 UI，
  // 同源事件通过 operation_id 匹配抑制，仅外部事件触发 refetch。

  // 结构性变更（创建/删除/移动/恢复）统一 refetch 整个文档
  const refetchDocument = useCallback(() => {
    if (!documentId) return
    getDocument(documentId)
      .then((res) => setTreeSync(() => res.blocks))
      .catch((err) => console.error('[SSE] refetch 失败:', err))
  }, [documentId, setTreeSync])

  /**
   * SSE 结构性事件处理：基于 operation_id 去重。
   *
   * 前端发起结构操作时生成唯一 operation_id 并记录到 pendingOperationIds。
   * 后端在 SSE 事件中原样回传该 operation_id。
   * 收到事件时，若 operation_id 在 pending 集合中 → 自身操作回声，跳过 refetch。
   * 否则 → 外部操作，触发 refetch。
   */
  const handleStructuralEvent = useCallback(
    (event: { operation_id?: string }) => {
      if (event.operation_id && pendingOperationIds.current.has(event.operation_id)) {
        console.log(`[SSE] 自身操作回声 (${event.operation_id})，跳过 refetch`)
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
    // 结构性变更：基于 operation_id 去重
    onBlockCreated: handleStructuralEvent,
    onBlockDeleted: handleStructuralEvent,
    onBlockMoved: handleStructuralEvent,
    onBlockRestored: handleStructuralEvent,
  })

  // ─── Command Context ───

  const makeContext = useCallback((operationId?: string): CommandContext => ({
    documentId,
    operationId,
    getTree: () => treeRef.current,
    setTreeSync,
    cancelPendingSave,
    refetchDocument: async () => {
      if (!documentId) return
      const res = await getDocument(documentId)
      setTreeSync(() => res.blocks)
    },
  }), [documentId, setTreeSync, cancelPendingSave])

  /** 结构操作入队辅助：捕获 undo 快照 + 生成 operation_id + 追踪 pending 集合 */
  const enqueueStructuralOp = useCallback(
    (label: string, operation: OperationRecord, execute: (ctx: CommandContext) => Promise<void>) => {
      opQueue.current.enqueue({
        label,
        execute: async () => {
          // 在队列执行时捕获快照（保证 treeRef 是最新状态）
          undoManager.current.pushBeforeStructuralOp(treeRef.current, operation)

          const operationId = crypto.randomUUID()
          pendingOperationIds.current.add(operationId)
          try {
            await execute(makeContext(operationId))
          } finally {
            pendingOperationIds.current.delete(operationId)
          }
        },
      })
    },
    [makeContext],
  )

  // ─── 内容变更（打字）→ debounce 保存 ───

  const handleContentChange = useCallback((blockId: string, content: string) => {
    // 捕获 undo 快照（防抖合并：同一块 500ms 内合并为一个条目）
    undoManager.current.pushContentChange(treeRef.current, blockId)

    // 乐观更新（不经过队列，打字是高频操作）
    setTreeAsync((prev) => updateBlockInTree(prev, blockId, { content }))

    // Debounce 保存到后端
    const timer = setTimeout(async () => {
      pendingTimers.current.delete(blockId)
      try {
        await updateBlock(blockId, { content })
      } catch (err) {
        console.error('自动保存失败:', err)
      }
    }, 300)

    const saves = pendingTimers.current
    if (saves.has(blockId)) clearTimeout(saves.get(blockId)!)
    saves.set(blockId, timer)
  }, [setTreeAsync])

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
          enqueueStructuralOp('split', { type: 'split', blockIds: [] }, (ctx) =>
            executeSplit(ctx),
          )
          break
        case 'delete': {
          const { blockId } = action
          enqueueStructuralOp(`delete:${blockId}`, { type: 'delete', blockIds: [blockId] }, (ctx) =>
            executeDelete(ctx, { blockId }),
          )
          break
        }
        case 'merge-with-previous': {
          const { blockId } = action
          enqueueStructuralOp(`merge:${blockId}`, { type: 'merge', blockIds: [blockId] }, (ctx) =>
            executeMerge(ctx, { blockId }),
          )
          break
        }
        case 'convert-block': {
          const { blockId, content, blockType } = action
          enqueueStructuralOp(`convert:${blockId}`, { type: 'convert', blockIds: [blockId] }, (ctx) =>
            executeConvertBlock(ctx, { blockId, content, blockType }),
          )
          break
        }
        case 'delete-range': {
          const { blockIds } = action
          if (blockIds.length === 0) break
          enqueueStructuralOp('delete-range', { type: 'delete-range', blockIds }, async (ctx) => {
            for (const id of blockIds) {
              ctx.cancelPendingSave(id)
            }
            for (const id of blockIds) {
              await deleteBlock(id, ctx.operationId)
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
          enqueueStructuralOp(`move:${blockId}`, { type: 'move', blockIds: [blockId] }, (ctx) =>
            executeMove(ctx, { blockId, target }),
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
  // 快照包含完整的 pre-operation 树，所有逆操作信息从快照推导。

  /** 对后端执行逆操作（基于快照树与当前树的差异） */
  const executeInverseOp = useCallback(
    async (entry: HistoryEntryWithMeta, currentTree: BlockNode[]) => {
      const { operation, tree: snapshotTree } = entry
      const snapshotFlat = flattenTree(snapshotTree)

      switch (operation.type) {
        case 'content-change': {
          // 快照有原始内容 → updateBlock 恢复
          for (const blockId of operation.blockIds) {
            const block = findBlockById(snapshotTree, blockId)
            if (block) {
              try { await updateBlock(blockId, { content: block.content ?? '' }) }
              catch (err) { console.error('[undo] content-change 失败:', err) }
            }
          }
          break
        }
        case 'delete':
        case 'delete-range': {
          // 快照有被删块 → restoreBlock
          for (const blockId of operation.blockIds) {
            try { await restoreBlock(blockId) }
            catch (err) { console.error('[undo] restore 失败:', err) }
          }
          refetchDocument()
          break
        }
        case 'split': {
          // 快照有拆分前的单块，当前树有拆分后的两块
          // 找出拆分产生的新块（在 currentTree 中但不在 snapshotTree 中）
          const snapshotIds = new Set(snapshotFlat.map((b) => b.id))
          const currentFlat = flattenTree(currentTree)
          const newBlockId = currentFlat.find((b) => !snapshotIds.has(b.id))?.id
          if (newBlockId) {
            try { await mergeBlock(newBlockId, { direction: 'previous' }) }
            catch (err) { console.error('[undo] split→merge 失败:', err) }
          }
          refetchDocument()
          break
        }
        case 'merge': {
          // 快照有两个独立块（source + target），当前只有合并后的 target
          // 恢复 source + 还原 target 内容
          const sourceId = operation.blockIds[0]
          const sourceBlock = findBlockById(snapshotTree, sourceId)
          if (!sourceBlock) break

          // target 是 source 在快照中的前一个块
          const sourceIdx = snapshotFlat.findIndex((b) => b.id === sourceId)
          if (sourceIdx <= 0) break
          const targetBlock = snapshotFlat[sourceIdx - 1]

          try {
            await restoreBlock(sourceId)
            await updateBlock(targetBlock.id, { content: targetBlock.content ?? '' })
          } catch (err) {
            console.error('[undo] merge→restore 失败:', err)
          }
          refetchDocument()
          break
        }
        case 'convert': {
          // 快照有原始 block_type → updateBlock 恢复
          const blockId = operation.blockIds[0]
          const block = findBlockById(snapshotTree, blockId)
          if (block) {
            try {
              await updateBlock(blockId, {
                block_type: block.block_type,
                content: block.content ?? '',
              })
            } catch (err) {
              console.error('[undo] convert 失败:', err)
            }
            // heading ↔ paragraph 转换可能触发 reparent
            const wasHeading = block.block_type.type === 'heading'
            if (wasHeading) refetchDocument()
          }
          break
        }
        case 'move': {
          // 快照有原始位置 → moveBlock 回去
          const blockId = operation.blockIds[0]
          const idx = snapshotFlat.findIndex((b) => b.id === blockId)
          if (idx < 0) break

          const moveReq: Record<string, string> = {}
          if (idx > 0) {
            moveReq.after_id = snapshotFlat[idx - 1].id
          } else if (snapshotFlat.length > 1) {
            moveReq.before_id = snapshotFlat[1].id
          }
          try { await moveBlock(blockId, moveReq) }
          catch (err) { console.error('[undo] move 失败:', err) }
          refetchDocument()
          break
        }
      }
    },
    [refetchDocument],
  )

  /** 对后端重新执行正向操作（redo 时） */
  const executeForwardOp = useCallback(
    async (entry: HistoryEntryWithMeta, preRedoTree: BlockNode[]) => {
      const { operation, tree: postOpTree } = entry
      const preRedoFlat = flattenTree(preRedoTree)
      const postOpFlat = flattenTree(postOpTree)

      switch (operation.type) {
        case 'content-change': {
          // redo 条目的快照树就是操作后的状态 → 从中取新内容
          for (const blockId of operation.blockIds) {
            const block = findBlockById(postOpTree, blockId)
            if (block) {
              try { await updateBlock(blockId, { content: block.content ?? '' }) }
              catch (err) { console.error('[redo] content-change 失败:', err) }
            }
          }
          break
        }
        case 'delete':
        case 'delete-range': {
          for (const blockId of operation.blockIds) {
            try { await deleteBlock(blockId) }
            catch (err) { console.error('[redo] delete 失败:', err) }
          }
          refetchDocument()
          break
        }
        case 'split': {
          // redo 条目的快照树是拆分后的状态
          // 找出拆分产生的新块
          const preRedoIds = new Set(preRedoFlat.map((b) => b.id))
          const newBlock = postOpFlat.find((b) => !preRedoIds.has(b.id))
          if (newBlock) {
            // 找到新块前面的块（被拆分的原始块）
            const newIdx = postOpFlat.findIndex((b) => b.id === newBlock.id)
            if (newIdx > 0) {
              const origBlock = postOpFlat[newIdx - 1]
              try {
                await updateBlock(origBlock.id, { content: origBlock.content ?? '' })
                // 新块需要 createBlock
                const { createBlock: createBlk } = await import('@/api/client')
                await createBlk({
                  parent_id: documentId,
                  block_type: newBlock.block_type,
                  content: newBlock.content ?? '',
                  content_type: 'markdown',
                  after_id: origBlock.id,
                })
              } catch (err) {
                console.error('[redo] split 失败:', err)
              }
            }
          }
          refetchDocument()
          break
        }
        case 'merge': {
          // redo 条目的快照树是合并后的状态
          const sourceId = operation.blockIds[0]
          // 在 preRedo 树中找到 source 块（redo 前它还在）
          const sourceBlock = findBlockById(preRedoTree, sourceId)
          if (sourceBlock) {
            // target 是 source 前面的块
            const sourceIdx = preRedoFlat.findIndex((b) => b.id === sourceId)
            if (sourceIdx > 0) {
              try { await mergeBlock(sourceId, { direction: 'previous' }) }
              catch (err) { console.error('[redo] merge 失败:', err) }
            }
          }
          refetchDocument()
          break
        }
        case 'convert': {
          const blockId = operation.blockIds[0]
          const block = findBlockById(postOpTree, blockId)
          if (block) {
            try {
              await updateBlock(blockId, {
                block_type: block.block_type,
                content: block.content ?? '',
              })
            } catch (err) {
              console.error('[redo] convert 失败:', err)
            }
            if (block.block_type.type === 'heading') refetchDocument()
          }
          break
        }
        case 'move': {
          // redo 条目的快照树是移动后的状态
          const blockId = operation.blockIds[0]
          const idx = postOpFlat.findIndex((b) => b.id === blockId)
          if (idx < 0) break

          const moveReq: Record<string, string> = {}
          if (idx > 0) {
            moveReq.after_id = postOpFlat[idx - 1].id
          } else if (postOpFlat.length > 1) {
            moveReq.before_id = postOpFlat[1].id
          }
          try { await moveBlock(blockId, moveReq) }
          catch (err) { console.error('[redo] move 失败:', err) }
          refetchDocument()
          break
        }
      }
    },
    [refetchDocument, documentId],
  )

  /** 执行撤销 */
  const handleUndo = useCallback(() => {
    if (!undoManager.current.canUndo()) return
    if (opQueue.current.isRunning()) return // 操作进行中不中断

    const entry = undoManager.current.undo()
    if (!entry) return

    // 保存当前状态到 redo 栈（在恢复快照之前！）
    const currentTree = [...treeRef.current]
    undoManager.current.pushRedo(currentTree, entry.operation)

    // 恢复快照（客户端立即生效）
    setTreeSync(() => entry.tree)

    // 恢复光标
    if (entry.cursor) {
      focusBlock(entry.cursor.blockId, entry.cursor.offset)
    }

    // 后端执行逆操作（异步，不阻塞 UI）
    executeInverseOp(entry, currentTree).catch((err) =>
      console.error('[undo] 后端逆操作失败:', err),
    )
  }, [setTreeSync, executeInverseOp])

  /** 执行重做 */
  const handleRedo = useCallback(() => {
    if (!undoManager.current.canRedo()) return
    if (opQueue.current.isRunning()) return

    const entry = undoManager.current.redo()
    if (!entry) return

    // 保存当前状态到 undo 栈
    const currentTree = [...treeRef.current]
    undoManager.current.pushUndo(currentTree, entry.operation)

    // 恢复快照（客户端立即生效）
    setTreeSync(() => entry.tree)

    // 恢复光标
    if (entry.cursor) {
      focusBlock(entry.cursor.blockId, entry.cursor.offset)
    }

    // 后端重新执行正向操作
    executeForwardOp(entry, currentTree).catch((err) =>
      console.error('[redo] 后端正向操作失败:', err),
    )
  }, [setTreeSync, executeForwardOp])

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

      const operationId = crypto.randomUUID()
      pendingOperationIds.current.add(operationId)

      createBlock({
        parent_id: documentId,
        block_type: { type: 'paragraph' },
        content: '',
        content_type: 'markdown',
        operation_id: operationId,
      })
        .then((created) => {
          const newBlock: BlockNode = { ...created, children: [] }
          setTreeSync(() => [newBlock])
          // 下一帧聚焦，确保 DOM 已渲染
          requestAnimationFrame(() => focusBlock(created.id))
        })
        .catch((err) => console.error('[editor] 创建初始段落失败:', err))
        .finally(() => {
          pendingOperationIds.current.delete(operationId)
        })
    },
    [readonly, documentId, setTreeSync],
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
