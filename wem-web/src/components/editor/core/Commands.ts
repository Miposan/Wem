/**
 * 编辑器 Command 模块
 *
 * 每个 Command 封装一个"用户意图"（split/merge/delete），返回 Promise。
 * 所有 Command 通过 OperationQueue 串行执行，保证操作有序。
 *
 * 悲观更新策略：先等 API 返回，再用真实数据更新 UI。
 * 无 tempId、无客户端缓存，状态以服务端为准。
 */

import type { BlockNode, BlockType } from '@/types/api'
import { createBlock, updateBlock, deleteBlock } from '@/api/client'
import {
  flattenTree,
  insertAfter,
  removeBlock,
  updateBlockInTree,
  findPrevBlock,
  findNextBlock,
} from './BlockOperations'
import { focusBlock, focusBlockEnd, syncBlockContent } from './SelectionManager'

// ─── Types ───

/** Command 执行上下文 */
export interface CommandContext {
  documentId: string
  /** 获取当前块树快照 */
  getTree: () => BlockNode[]
  /** 同步更新块树（内部用 flushSync，确保 DOM 立即更新） */
  setTreeSync: (updater: (prev: BlockNode[]) => BlockNode[]) => void
  /**
   * 获取最新创建的块 ID（用于连续 split 时自动指向新块）
   *
   * 快速连续按 Enter 时，keydown 捕获的 blockId 是过期的（UI 还没更新），
   * 用此方法获取上一个 split 创建的块 ID。
   */
  getLatestBlockId: () => string | null
  /**
   * 设置最新创建的块 ID
   *
   * split 完成后调用，后续排队的 split 会通过 getLatestBlockId() 获取。
   */
  setLatestBlockId: (id: string | null) => void
}

/** Split 参数 */
export interface SplitParams {
  blockId: string
  offset: number
}

/** Delete 参数 */
export interface DeleteParams {
  blockId: string
}

/** Merge 参数 */
export interface MergeParams {
  blockId: string
}

// ─── Commands ───

/**
 * Split — 在光标处拆分段落
 *
 * 悲观更新：并行调 API（updateBlock + createBlock），成功后用真实数据更新 UI。
 *
 * 连续 split 处理：
 *   当队列中有多个 split 排队时，通过 latestBlockId 跟踪上一个 split 创建的块，
 *   后续 split 自动指向新块（而不是使用 keydown 时捕获的过期 blockId）。
 */
export async function executeSplit(ctx: CommandContext, params: SplitParams): Promise<void> {
  // ── 解析目标块：优先使用最新创建的块 ──
  const latestId = ctx.getLatestBlockId()
  const tree = ctx.getTree()

  // latestBlockId 可能已被 delete/merge 消灭，验证是否仍在树中
  const targetId = (latestId && flattenTree(tree).some((b) => b.id === latestId))
    ? latestId
    : params.blockId
  const block = flattenTree(tree).find((b) => b.id === targetId)
  if (!block) return

  // 如果目标是最新块（刚被 split 创建），offset 应为 0
  const effectiveOffset = latestId ? 0 : params.offset

  const text = block.content ?? ''
  const firstHalf = text.slice(0, effectiveOffset)
  const secondHalf = text.slice(effectiveOffset)

  const newBlockType: BlockType =
    block.block_type.type === 'heading' ? { type: 'paragraph' } : { ...block.block_type }

  // ── 悲观：并行等 API ──
  try {
    const [, created] = await Promise.all([
      updateBlock(targetId, { content: firstHalf }),
      createBlock({
        parent_id: block.parent_id,
        after_id: targetId,
        block_type: newBlockType,
        content: secondHalf,
        content_type: 'markdown',
      }),
    ])

    // API 成功 → 用真实数据更新 UI
    const newBlock: BlockNode = { ...created, children: [] }
    ctx.setTreeSync((prev) => {
      const updated = updateBlockInTree(prev, targetId, { content: firstHalf })
      return insertAfter(updated, targetId, newBlock)
    })

    // 直接同步原块 DOM（contentEditable 不受 React 控制）
    syncBlockContent(targetId, firstHalf)

    // 记录最新创建的块，供后续排队的 split 使用
    ctx.setLatestBlockId(created.id)

    // 聚焦新块
    focusBlock(created.id)
  } catch (err) {
    console.error('[split] 创建块失败:', err)
  }
}

/**
 * Delete — 删除空块
 *
 * 悲观更新：先调 API，成功后更新 UI。
 */
export async function executeDelete(ctx: CommandContext, params: DeleteParams): Promise<void> {
  const tree = ctx.getTree()
  const flat = flattenTree(tree)
  if (flat.length <= 1) return

  const block = flat.find((b) => b.id === params.blockId)
  if (!block) return

  const prev = findPrevBlock(tree, params.blockId)

  // 悲观：先等 API
  try {
    await deleteBlock(params.blockId)
  } catch (err) {
    console.error('[delete] 删除块失败:', err)
    return
  }

  // 清除 latestBlockId（目标块可能已被删除）
  ctx.setLatestBlockId(null)

  // API 成功 → 更新 UI
  ctx.setTreeSync((prevTree) => removeBlock(prevTree, params.blockId))
  if (prev) focusBlockEnd(prev.id)
}

/**
 * Merge — 将当前块内容合并到前一个块
 *
 * 悲观更新：先调 API（updateBlock + deleteBlock），成功后更新 UI。
 */
export async function executeMerge(ctx: CommandContext, params: MergeParams): Promise<void> {
  const tree = ctx.getTree()
  const block = flattenTree(tree).find((b) => b.id === params.blockId)
  if (!block) return

  const prev = findPrevBlock(tree, params.blockId)
  if (!prev) return

  const prevText = prev.content ?? ''
  const currentText = block.content ?? ''
  const merged = prevText + currentText
  const mergePoint = prevText.length

  // 悲观：先等 API
  try {
    await updateBlock(prev.id, { content: merged })
    try {
      await deleteBlock(params.blockId)
    } catch {
      // 非致命：内容已合并，源块未删除
      console.warn('[merge] 删除源块失败（非致命）')
    }
  } catch (err) {
    console.error('[merge] 合并失败:', err)
    return
  }

  // 清除 latestBlockId（当前块已被合并掉）
  ctx.setLatestBlockId(null)

  // API 成功 → 更新 UI
  ctx.setTreeSync((prevTree) => {
    const updated = updateBlockInTree(prevTree, prev.id, { content: merged })
    return removeBlock(updated, params.blockId)
  })

  // 直接同步 prev 块 DOM（contentEditable 不受 React 控制）
  syncBlockContent(prev.id, merged)

  focusBlock(prev.id, mergePoint)
}

/**
 * Focus Previous — 聚焦前一个块末尾
 */
export function executeFocusPrevious(ctx: CommandContext, blockId: string): void {
  const prev = findPrevBlock(ctx.getTree(), blockId)
  if (prev) focusBlockEnd(prev.id)
}

/**
 * Focus Next — 聚焦下一个块开头
 */
export function executeFocusNext(ctx: CommandContext, blockId: string): void {
  const next = findNextBlock(ctx.getTree(), blockId)
  if (next) focusBlock(next.id, 0)
}
