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
import { deleteBlock, splitBlock, mergeBlock } from '@/api/client'
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
   * 获取最新创建的块 ID（消费式：读取后应立即 setLatestBlockId(null) 清除）
   *
   * 快速连续按 Enter 时，keydown 捕获的 blockId 是过期的（UI 还没更新），
   * 用此方法获取上一个 split 创建的块 ID。
   * 读取后必须清除，防止残留到后续无关操作导致 offset 被错误置零。
   */
  getLatestBlockId: () => string | null
  /**
   * 设置最新创建的块 ID
   *
   * split 完成后调用。被下一次 executeSplit 消费后自动清除。
   */
  setLatestBlockId: (id: string | null) => void
  /**
   * 取消指定块的待处理内容保存（debounce 定时器）
   *
   * 结构变更（split/merge/delete）前调用，防止旧的 debounce 定时器
   * 在操作完成后触发，覆盖服务端已更新的内容。
   */
  cancelPendingSave: (blockId: string) => void
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
 * 原子意图 API：前端做文本切割，后端一次性完成 update+create。
 * 悲观更新：等 API 返回真实数据后再刷新 UI。
 *
 * 连续 split 处理：
 *   当队列中有多个 split 排队时，通过 latestBlockId 跟踪上一个 split 创建的块，
 *   后续 split 自动指向新块（而不是使用 keydown 时捕获的过期 blockId）。
 */
export async function executeSplit(ctx: CommandContext, params: SplitParams): Promise<void> {
  // ── 解析目标块 ──
  const latestId = ctx.getLatestBlockId()
  ctx.setLatestBlockId(null)

  const tree = ctx.getTree()
  const flat = flattenTree(tree)

  // latestBlockId 可能已被 delete/merge 消灭，验证是否仍在树中
  const targetId = (latestId && flat.some((b) => b.id === latestId))
    ? latestId
    : params.blockId
  const block = flat.find((b) => b.id === targetId)
  if (!block) return

  const effectiveOffset = targetId !== params.blockId ? 0 : params.offset

  const text = block.content ?? ''
  const contentBefore = text.slice(0, effectiveOffset)
  const contentAfter = text.slice(effectiveOffset)

  ctx.cancelPendingSave(targetId)

  const newBlockType: BlockType =
    block.block_type.type === 'heading' ? { type: 'paragraph' } : { ...block.block_type }

  // ── 原子 API：一次调用完成 split ──
  try {
    const { updated_block, new_block } = await splitBlock(targetId, {
      content_before: contentBefore,
      content_after: contentAfter,
      new_block_type: newBlockType,
    })

    // 用后端返回的真实数据更新 UI
    const newBlock: BlockNode = { ...new_block, children: [] }
    ctx.setTreeSync((prev) => {
      const updated = updateBlockInTree(prev, targetId, { content: updated_block.content })
      return insertAfter(updated, targetId, newBlock)
    })

    syncBlockContent(targetId, updated_block.content ?? contentBefore)
    ctx.setLatestBlockId(new_block.id)
    focusBlock(new_block.id)
  } catch (err) {
    console.error('[split] 拆分块失败:', err)
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

  // 取消目标块的 debounce 定时器
  ctx.cancelPendingSave(params.blockId)

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
 * 原子意图 API：后端一次性完成 content 合并 + 源块删除。
 * 悲观更新：等 API 返回真实数据后再刷新 UI。
 */
export async function executeMerge(ctx: CommandContext, params: MergeParams): Promise<void> {
  const tree = ctx.getTree()
  const block = flattenTree(tree).find((b) => b.id === params.blockId)
  if (!block) return

  const prev = findPrevBlock(tree, params.blockId)
  if (!prev) return

  const mergePoint = (prev.content ?? '').length

  // 取消两个块的 debounce 定时器，防止旧内容覆盖合并结果
  ctx.cancelPendingSave(prev.id)
  ctx.cancelPendingSave(params.blockId)

  // ── 原子 API：一次调用完成 merge ──
  try {
    const { merged_block } = await mergeBlock(params.blockId, {
      direction: 'previous',
    })

    ctx.setLatestBlockId(null)

    // 用后端返回的真实数据更新 UI
    ctx.setTreeSync((prevTree) => {
      const updated = updateBlockInTree(prevTree, merged_block.id, {
        content: merged_block.content,
      })
      return removeBlock(updated, params.blockId)
    })

    syncBlockContent(merged_block.id, merged_block.content ?? '')
    focusBlock(merged_block.id, mergePoint)
  } catch (err) {
    console.error('[merge] 合并块失败:', err)
  }
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
