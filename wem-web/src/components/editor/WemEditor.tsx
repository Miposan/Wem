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
import { getDocument, updateBlock, createBlock } from '@/api/client'

/** 自身操作完成后抑制 SSE 回声的时间窗口（ms） */
const SSE_ECHO_SUPPRESS_MS = 1_000
import { BlockTreeRenderer } from './components/BlockTreeRenderer'
import { updateBlockInTree } from './core/BlockOperations'
import { OperationQueue } from './core/OperationQueue'
import {
  executeSplit,
  executeDelete,
  executeMerge,
  executeFocusPrevious,
  executeFocusNext,
  executeConvertBlock,
} from './core/Commands'
import type { CommandContext } from './core/Commands'
import type { BlockAction } from './core/types'
import { useDocumentSSE } from '@/hooks/useDocumentSSE'
import { syncBlockContent, focusBlock, findEditable } from './core/SelectionManager'

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
   * SSE 回声抑制：记录自身操作完成后的时间戳，在此窗口内忽略结构性 SSE 事件。
   * 防止自身 split/delete/merge 触发的 SSE echo 导致 refetch 与 OperationQueue 竞态。
   */
  const suppressRefetchUntil = useRef(0)

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

  useEffect(() => {
    blocksRef.current = blocks
  }, [blocks])

  // 仅在 documentId 变化时从 props 同步数据
  useEffect(() => {
    setTreeState(blocksRef.current)
    treeRef.current = blocksRef.current
    setCollapsedIds(new Set())
    opQueue.current.clear()
  }, [documentId])

  // ─── SSE 实时事件订阅 ───
  //
  // 后端广播所有 mutation。自身操作已通过 OperationQueue 悲观更新 UI，
  // 同源事件通过 suppressRefetchUntil 抑制，仅外部事件触发 refetch。

  // 结构性变更（创建/删除/移动/恢复）统一 refetch 整个文档
  const refetchDocument = useCallback(() => {
    if (!documentId) return
    getDocument(documentId)
      .then((res) => setTreeSync(() => res.blocks))
      .catch((err) => console.error('[SSE] refetch 失败:', err))
  }, [documentId, setTreeSync])

  // SSE 结构性回调包装器：抑制自身操作的回声
  // 自身 split/delete/merge 完成后 suppressRefetchUntil 设为当前时间 + 1s
  // 在此窗口内收到 block_created/block_deleted 等事件时跳过 refetch，
  // 避免 OperationQueue 中的后续操作被全量 refetch 覆盖。
  const dedupedRefetch = useCallback(() => {
    if (Date.now() < suppressRefetchUntil.current) {
      console.log('[SSE] 抑制自身操作回声，跳过 refetch')
      return
    }
    refetchDocument()
  }, [refetchDocument])

  useDocumentSSE(documentId, {
    // 块内容更新：增量合并到 tree + 同步 contentEditable DOM
    // 注意：后端 serde(flatten) 将 block 字段展平到事件顶层，直接用 event.id 访问
    onBlockUpdated: useCallback(
      (event) => {
        // 外部内容更新无需 flushSync（DOM 已通过 syncBlockContent 即时同步）
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
        // 树状态仍会更新（版本号等），但 contentEditable 内容由用户控制
        const el = findEditable(event.id)
        if (!el || document.activeElement !== el) {
          syncBlockContent(event.id, event.content)
        }
      },
      [setTreeAsync],
    ),
    // 结构性变更：带去重的 refetch
    onBlockCreated: dedupedRefetch,
    onBlockDeleted: dedupedRefetch,
    onBlockMoved: dedupedRefetch,
    onBlockRestored: dedupedRefetch,
  })

  // ─── Command Context ───

  const makeContext = useCallback((): CommandContext => ({
    documentId,
    getTree: () => treeRef.current,
    setTreeSync,
    cancelPendingSave,
    refetchDocument: async () => {
      if (!documentId) return
      const res = await getDocument(documentId)
      setTreeSync(() => res.blocks)
    },
  }), [documentId, setTreeSync, cancelPendingSave])

  /** 结构操作入队辅助：统一 suppressRefetchUntil + 消除重复的 try/finally 模式 */
  const enqueueStructuralOp = useCallback(
    (label: string, execute: (ctx: CommandContext) => Promise<void>) => {
      opQueue.current.enqueue({
        label,
        execute: async () => {
          const ctx = makeContext()
          try {
            await execute(ctx)
          } finally {
            suppressRefetchUntil.current = Date.now() + SSE_ECHO_SUPPRESS_MS
          }
        },
      })
    },
    [makeContext],
  )

  // ─── 内容变更（打字）→ debounce 保存 ───

  const handleContentChange = useCallback((blockId: string, content: string) => {
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
        // 导航操作 → 无 API 调用，无需队列
        case 'focus-previous':
          executeFocusPrevious(makeContext(), action.blockId)
          break
        case 'focus-next':
          executeFocusNext(makeContext(), action.blockId)
          break
      }
    },
    [makeContext, enqueueStructuralOp],
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

      createBlock({
        parent_id: documentId,
        block_type: { type: 'paragraph' },
        content: '',
        content_type: 'markdown',
      })
        .then((created) => {
          const newBlock: BlockNode = { ...created, children: [] }
          setTreeSync(() => [newBlock])
          // 下一帧聚焦，确保 DOM 已渲染
          requestAnimationFrame(() => focusBlock(created.id))
        })
        .catch((err) => console.error('[editor] 创建初始段落失败:', err))
    },
    [readonly, documentId, setTreeSync],
  )

  return (
    <div className="wem-editor-root" onClick={handleEditorClick}>
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
