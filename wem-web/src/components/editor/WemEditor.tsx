/**
 * WemEditor — 自研块编辑器主组件
 *
 * 架构概览（v3 悲观同步 + latestBlockId 跟踪）：
 *
 *   用户操作 → useTextBlock → handleAction → OperationQueue → Command
 *                                                        ↓
 *                                               API 同步 (await createBlock/...)
 *                                               更新 UI (flushSync + 真实数据)
 *                                               latestBlockId 跟踪（解决连续 Enter）
 *                                               DOM 光标 (SelectionManager)
 *
 * 核心设计：
 * - 悲观更新：结构操作先等 API 返回，再用真实数据更新 UI
 * - latestBlockId：快速连续 Enter 时，后续 split 自动指向前一个 split 创建的新块
 * - OperationQueue 串行化所有结构变更操作，保证操作有序
 */

import { useCallback, useEffect, useRef, useState } from 'react'
import { flushSync } from 'react-dom'
import type { BlockNode } from '@/types/api'
import { getDocument, updateBlock } from '@/api/client'
import { BlockTreeRenderer } from './components/BlockTreeRenderer'
import { updateBlockInTree } from './core/BlockOperations'
import { OperationQueue } from './core/OperationQueue'
import {
  executeSplit,
  executeDelete,
  executeMerge,
  executeFocusPrevious,
  executeFocusNext,
} from './core/Commands'
import type { CommandContext } from './core/Commands'
import type { BlockAction } from './core/types'
import { useDocumentSSE } from '@/hooks/useDocumentSSE'

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
  const treeRef = useRef<BlockNode[]>([])
  const blocksRef = useRef(blocks)
  const pendingTimers = useRef<Map<string, ReturnType<typeof setTimeout>>>(new Map())

  /**
   * 最新创建的块 ID — 解决快速连续 Enter 的过期 blockId 问题
   *
   * 快速连按时，keydown 捕获的 blockId 是 UI 更新前的旧值。
   * 每个 split 完成后把新块 ID 写入此 ref，后续 split 优先使用它。
   */
  const latestBlockIdRef = useRef<string | null>(null)

  /**
   * 操作队列 — 序列化所有结构变更操作
   *
   * 保证快速连续操作（如快速 Enter）时，每个操作等上一个完成后再执行，
   * 避免并发修改导致数据不一致。
   */
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

  useEffect(() => {
    blocksRef.current = blocks
  }, [blocks])

  // treeRef.current 由 setTreeSync/setTreeAsync 直接管理，不需要从 React state 反向同步
  // （移除了 useEffect(() => { treeRef.current = tree }, [tree])，
  //   否则它会把 treeRef 拉回到旧的 React state，覆盖 setTreeAsync 的即时更新）

  // 仅在 documentId 变化时从 props 同步数据
  useEffect(() => {
    setTreeState(blocksRef.current)
    treeRef.current = blocksRef.current
    setCollapsedIds(new Set())
    opQueue.current.clear()
    latestBlockIdRef.current = null
  }, [documentId])

  // ─── SSE 实时事件订阅 ───
  //
  // 后端是唯一数据真相源。所有 mutation（REST + Agent）都通过 EventBus 广播。
  // 前端自己的操作（split/delete/merge）已通过 OperationQueue 悲观更新了 UI，
  // 所以 SSE 回调中的同源事件是"回声"——乐观数据已匹配，无需操作。
  // 真正的外部事件（如 Agent 操作）会触发 UI 更新。

  // ─── SSE 回调 ───

  // 结构性变更（创建/删除/移动/恢复）统一 refetch 整个文档
  const refetchDocument = useCallback(() => {
    if (!documentId) return
    getDocument(documentId)
      .then((res) => setTreeSync(() => res.blocks))
      .catch((err) => console.error('[SSE] refetch 失败:', err))
  }, [documentId, setTreeSync])

  useDocumentSSE(documentId, {
    // 块内容更新：增量合并到 tree
    onBlockUpdated: useCallback(
      (event) => {
        setTreeSync((prev) =>
          updateBlockInTree(prev, event.block.id, {
            content: event.block.content,
            block_type: event.block.block_type,
            properties: event.block.properties,
            version: event.block.version,
            modified: event.block.modified,
          }),
        )
      },
      [setTreeSync],
    ),
    // 结构性变更：统一 refetch
    onBlockCreated: refetchDocument,
    onBlockDeleted: refetchDocument,
    onBlockMoved: refetchDocument,
    onBlockRestored: refetchDocument,
  })

  // ─── Command Context ───

  const makeContext = useCallback((): CommandContext => ({
    documentId,
    getTree: () => treeRef.current,
    setTreeSync,
    getLatestBlockId: () => latestBlockIdRef.current,
    setLatestBlockId: (id: string | null) => { latestBlockIdRef.current = id },
  }), [documentId, setTreeSync])

  // ─── 内容变更（打字）→ debounce 保存 ───

  const handleContentChange = useCallback((blockId: string, content: string) => {
    // 乐观更新（不经过队列，打字是高频操作）
    setTreeAsync((prev) => updateBlockInTree(prev, blockId, { content }))

    // Debounce 保存到后端
    const timer = setTimeout(async () => {
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
      switch (action.type) {
        // 结构操作 → 入队串行执行（队列积压过多时静默丢弃）
        case 'split': {
          const { blockId, offset } = action
          opQueue.current.enqueue({
            label: `split:${blockId}@${offset}`,
            execute: () => {
              const ctx = makeContext()
              return executeSplit(ctx, { blockId, offset })
            },
          })
          break
        }

        case 'delete': {
          const { blockId } = action
          opQueue.current.enqueue({
            label: `delete:${blockId}`,
            execute: () => {
              const ctx = makeContext()
              return executeDelete(ctx, { blockId })
            },
          })
          break
        }

        case 'merge-with-previous': {
          const { blockId } = action
          opQueue.current.enqueue({
            label: `merge:${blockId}`,
            execute: () => {
              const ctx = makeContext()
              return executeMerge(ctx, { blockId })
            },
          })
          break
        }

        // 导航操作 → 无需队列（无 API 调用，不会产生竞态）
        case 'focus-previous': {
          executeFocusPrevious(makeContext(), action.blockId)
          break
        }

        case 'focus-next': {
          executeFocusNext(makeContext(), action.blockId)
          break
        }
      }
    },
    [makeContext],
  )

  return (
    <div className="wem-editor-root">
      <BlockTreeRenderer
        blocks={tree}
        readonly={readonly}
        placeholder={placeholder}
        collapsedIds={collapsedIds}
        onToggleCollapse={handleToggleCollapse}
        onContentChange={handleContentChange}
        onAction={handleAction}
      />
    </div>
  )
}
