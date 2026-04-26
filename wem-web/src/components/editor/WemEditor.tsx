/**
 * WemEditor — 块编辑器主组件
 *
 * 架构：
 *   用户操作 → useTextBlock → handleAction → OperationQueue → Command
 *                                                        ↓
 *                                               API 同步 (await createBlock/...)
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
import { getDocument, updateBlock, createBlock, deleteBlock, undoDocument, redoDocument } from '@/api/client'
import { BlockTreeRenderer } from './components/BlockTreeRenderer'
import { updateBlockInTree, removeBlock, flattenTree, findBlockById, insertAfter } from './core/BlockOperations'
import { OperationQueue } from './core/OperationQueue'
import {
  executeSplit,
  executeDelete,
  executeMerge,
  executeMergeNext,
  executeFocusPrevious,
  executeFocusNext,
  executeConvertBlock,
  executeMove,
  executeMoveHeadingTree,
  executeToggleListType,
  executeIndentListItem,
  executeOutdentListItem,
  executeExitList,
  executeExitCodeBlock,
} from './core/Commands'
import type { CommandContext } from './core/Commands'
import type { BlockAction, EditorSelection } from './core/types'
import { useDocumentSSE } from '@/hooks/useDocumentSSE'
import type { BlockUpdatedEvent } from '@/hooks/useDocumentSSE'
import { syncBlockContent, focusBlock, findEditable } from './core/SelectionManager'
import { useSelectionManager } from './core/useSelectionManager'
import { useBlockDrag } from './core/useBlockDrag'
import { getSelectedBlockIdsSet } from './core/EditorSelection'
import { BlockContextMenu } from './components/BlockContextMenu'
import type { BlockContextMenuState, BlockContextAction } from './components/BlockContextMenu'

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
  const [contextMenuState, setContextMenuState] = useState<BlockContextMenuState>({
    visible: false,
    x: 0,
    y: 0,
    block: null,
  })
  const treeRef = useRef<BlockNode[]>([])
  const pendingTimers = useRef<Map<string, ReturnType<typeof setTimeout>>>(new Map())

  // ─── 跨块选区刚结束标记 ───
  //
  // 跨块拖选结束后 click 事件会在公共祖先触发，handleEditorClick
  // 需要跳过「聚焦最后块 / 创建空白段落」逻辑，否则焦点会跳到文档末尾。
  // 用 ref 而非 state 是因为 click 与 mouseup 在同一微任务中，
  // React state 更新可能尚未提交。
  const crossBlockSelectionRef = useRef(false)
  const creatingBlankBlockRef = useRef(false)

  // ─── 跨块选区 ───

  const handleSelectionChange = useCallback(
    (newSelection: EditorSelection | null) => {
      // 同步标记：有跨块选区 → true，清除 → false
      crossBlockSelectionRef.current = newSelection !== null
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
      (event: BlockUpdatedEvent) => {
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
        case 'merge-with-next': {
          const { blockId } = action
          enqueueStructuralOp(`merge-next:${blockId}`, (ctx) =>
            executeMergeNext(ctx, { blockId }),
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
        case 'toggle-list-type': {
          const { blockId } = action
          enqueueStructuralOp(`toggle-list-type:${blockId}`, (ctx) =>
            executeToggleListType(ctx, blockId),
          )
          break
        }
        case 'indent-list-item': {
          const { blockId } = action
          enqueueStructuralOp(`indent-list-item:${blockId}`, (ctx) =>
            executeIndentListItem(ctx, blockId),
          )
          break
        }
        case 'outdent-list-item': {
          const { blockId } = action
          enqueueStructuralOp(`outdent-list-item:${blockId}`, (ctx) =>
            executeOutdentListItem(ctx, blockId),
          )
          break
        }
        case 'exit-list': {
          const { blockId } = action
          enqueueStructuralOp(`exit-list:${blockId}`, (ctx) =>
            executeExitList(ctx, blockId),
          )
          break
        }
        case 'exit-code-block': {
          const { blockId, content } = action
          enqueueStructuralOp(`exit-code-block:${blockId}`, (ctx) =>
            executeExitCodeBlock(ctx, blockId, content),
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
      // Ctrl+A → 全选所有块
      if ((e.ctrlKey || e.metaKey) && e.key === 'a') {
        const flat = flattenTree(treeRef.current)
        if (flat.length < 2) return // 单块或空文档交给浏览器原生全选
        const first = flat[0]
        const last = flat[flat.length - 1]
        e.preventDefault()
        handleSelectionChange({
          anchorBlockId: first.id,
          anchorOffset: 0,
          focusBlockId: last.id,
          focusOffset: (last.content ?? '').length,
        })
        return
      }
      // Escape → 清除跨块选区
      if (e.key === 'Escape') {
        if (selection) {
          e.preventDefault()
          clearSelection()
        }
        return
      }
    },
    [handleUndo, handleRedo, handleSelectionChange, selection, clearSelection],
  )

  // ─── 空白区域点击 → 聚焦编辑入口 ───
  //
  // 点击编辑器空白区域时，优先聚焦已有的最后一个 Paragraph。
  // 如果文档里没有 Paragraph，则在文档末尾创建一个新的 Paragraph。
  // 通过 closest('[data-block-id]') 判断点击是否落在已有块上，避免误触发。

  const focusLastBlockIfParagraph = useCallback((): boolean => {
    const lastBlock = treeRef.current.at(-1)
    if (!lastBlock || lastBlock.block_type.type !== 'paragraph') return false

    focusBlock(lastBlock.id, (lastBlock.content ?? '').length)
    return true
  }, [])

  const createParagraphFromBlank = useCallback(() => {
    if (readonly || creatingBlankBlockRef.current) return

    creatingBlankBlockRef.current = true

    const editorId = crypto.randomUUID()
    addPendingOperationId(editorId)

    const currentTree = treeRef.current
    const lastTopLevelBlock = currentTree.at(-1)

    createBlock({
      parent_id: documentId,
      block_type: { type: 'paragraph' },
      content: '',
      after_id: lastTopLevelBlock?.id,
      editor_id: editorId,
    })
      .then((created) => {
        const newBlock: BlockNode = { ...created, children: [] }
        setTreeSync((prev) => [...prev, newBlock])
        requestAnimationFrame(() => focusBlock(created.id))
      })
      .catch((err) => console.error('[editor] 创建空白段落失败:', err))
      .finally(() => {
        creatingBlankBlockRef.current = false
      })
  }, [readonly, documentId, setTreeSync, addPendingOperationId])

  const handleEditorClick = useCallback(
    (e: React.MouseEvent) => {
      if (readonly) return
      // 点击在已有块上，忽略
      if ((e.target as HTMLElement).closest('[data-block-id]')) return

      // 跨块选区刚结束时，click 在公共祖先触发，跳过聚焦逻辑
      if (crossBlockSelectionRef.current) {
        crossBlockSelectionRef.current = false
        return
      }

      // 只在点击到编辑器底部空白区域（所有块之下）时才聚焦/创建段落
      // 点击在块之间的空白区域（margin）不触发
      const editorEl = (e.currentTarget as HTMLElement)
      const lastChild = editorEl.querySelector(':scope > .wem-block-container:last-of-type')
      if (lastChild) {
        const lastRect = lastChild.getBoundingClientRect()
        if (e.clientY < lastRect.bottom) return
      }

      if (focusLastBlockIfParagraph()) return
      createParagraphFromBlank()
    },
    [readonly, focusLastBlockIfParagraph, createParagraphFromBlank],
  )

  // ─── 右键菜单 ───

  const handleBlockContextMenu = useCallback(
    (e: React.MouseEvent, block: BlockNode) => {
      e.preventDefault()
      setContextMenuState({
        visible: true,
        x: e.clientX,
        y: e.clientY,
        block,
      })
    },
    [],
  )

  const handleContextMenuClose = useCallback(() => {
    setContextMenuState((prev) => ({ ...prev, visible: false }))
  }, [])

  const handleContextAction = useCallback(
    (action: BlockContextAction) => {
      switch (action.type) {
        case 'delete':
          handleAction({ type: 'delete', blockId: action.blockId })
          break
        case 'copy': {
          const block = findBlockById(treeRef.current, action.blockId)
          if (block) navigator.clipboard.writeText(block.content ?? '').catch(() => {})
          break
        }
        case 'cut': {
          const block = findBlockById(treeRef.current, action.blockId)
          if (block) {
            navigator.clipboard.writeText(block.content ?? '').catch(() => {})
            handleAction({ type: 'delete', blockId: action.blockId })
          }
          break
        }
        case 'duplicate': {
          const block = findBlockById(treeRef.current, action.blockId)
          if (block) {
            const editorId = crypto.randomUUID()
            addPendingOperationId(editorId)
            createBlock({
              parent_id: documentId,
              block_type: block.block_type,
              content: block.content ?? '',
              after_id: block.id,
              editor_id: editorId,
            })
              .then((created) => {
        const newBlock: BlockNode = { ...created, children: [] }
        setTreeSync((prev) => insertAfter(prev, block.id, newBlock))
      })
              .catch((err) => console.error('[context-menu] 复制块失败:', err))
          }
          break
        }
        case 'convert': {
          const block = findBlockById(treeRef.current, action.blockId)
          if (block) {
            handleAction({
              type: 'convert-block',
              blockId: action.blockId,
              content: block.content ?? '',
              blockType: action.blockType,
            })
          }
          break
        }
        case 'copy-id':
          navigator.clipboard.writeText(action.blockId).catch(() => {})
          break
      }
    },
    [handleAction, documentId, setTreeSync, addPendingOperationId],
  )

  /** 在指定块后面插入新块（顶层） */
  function insertBlockAfter(tree: BlockNode[], afterId: string, newBlock: BlockNode): BlockNode[] {
    return tree.flatMap((node) => {
      if (node.id === afterId) return [node, { ...newBlock, children: [] }]
      if (node.children.length > 0) {
        return [{ ...node, children: insertBlockAfter(node.children, afterId, newBlock) }]
      }
      return [node]
    })
  }

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
        onBlockContextMenu={handleBlockContextMenu}
      />
      <BlockContextMenu
        state={contextMenuState}
        onClose={handleContextMenuClose}
        onAction={handleContextAction}
      />
    </div>
  )
}
