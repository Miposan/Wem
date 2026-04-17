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
import { deleteBlock, splitBlock, mergeBlock, updateBlock } from '@/api/client'
import {
  flattenTree,
  findBlockById,
  insertAfter,
  insertAsFirstChild,
  removeBlock,
  updateBlockInTree,
  findPrevBlock,
  findNextBlock,
} from './BlockOperations'
import { focusBlock, focusBlockEnd, getCursorPosition, syncBlockContent } from './SelectionManager'

// ─── Types ───

/** Command 执行上下文 */
export interface CommandContext {
  documentId: string
  /** 操作 ID（前端生成，用于 SSE 回声去重），仅结构操作需要 */
  operationId?: string
  /** 获取当前块树快照 */
  getTree: () => BlockNode[]
  /** 同步更新块树（内部用 flushSync，确保 DOM 立即更新） */
  setTreeSync: (updater: (prev: BlockNode[]) => BlockNode[]) => void
  /**
   * 取消指定块的待处理内容保存（debounce 定时器）
   *
   * 结构变更（split/merge/delete）前调用，防止旧的 debounce 定时器
   * 在操作完成后触发，覆盖服务端已更新的内容。
   */
  cancelPendingSave: (blockId: string) => void
  /**
   * 从服务端重新拉取整棵块树并同步到 UI
   *
   * 用于结构变更（如 heading 自动嵌套导致多块 reparent），
   * 局部 updateBlockInTree 无法反映父级变化的情况。
   */
  refetchDocument: () => Promise<void>
}

/** Delete 参数 */
export interface DeleteParams {
  blockId: string
}

/** Merge 参数 */
export interface MergeParams {
  blockId: string
}

/** Convert 参数（Markdown 快捷键转换块类型） */
export interface ConvertParams {
  blockId: string
  content: string
  blockType: BlockType
}

// ─── Commands ───

/**
 * Split — 在光标处拆分段落
 *
 * 原子意图 API：前端做文本切割，后端一次性完成 update+create。
 * 悲观更新：等 API 返回真实数据后再刷新 UI。
 *
 * 直接从 DOM 读取当前光标位置，不依赖 keydown 时捕获的参数。
 * OperationQueue 保证串行执行：上一个 split 完成后 flushSync + focusBlock
 * 已将 DOM 更新到最新状态，此时读到的光标位置就是准确的。
 */
export async function executeSplit(ctx: CommandContext): Promise<void> {
  // ── 从 DOM 读取真实光标位置 ──
  const cursor = getCursorPosition()
  if (!cursor) return

  const { blockId: targetId, offset } = cursor

  const tree = ctx.getTree()
  const block = findBlockById(tree, targetId)
  if (!block) return

  const text = block.content ?? ''
  const contentBefore = text.slice(0, offset)
  const contentAfter = text.slice(offset)

  ctx.cancelPendingSave(targetId)

  const isHeading = block.block_type.type === 'heading'
  const newBlockType: BlockType =
    isHeading ? { type: 'paragraph' } : { ...block.block_type }

  // ── 原子 API：一次调用完成 split（含 heading 嵌套） ──
  try {
    const { updated_block, new_block } = await splitBlock(targetId, {
      content_before: contentBefore,
      content_after: contentAfter,
      new_block_type: newBlockType,
      nest_under_parent: isHeading || undefined,
      operation_id: ctx.operationId,
    })

    // 用后端返回的真实数据更新 UI
    const newBlock: BlockNode = { ...new_block, children: [] }
    ctx.setTreeSync((prev) => {
      const updated = updateBlockInTree(prev, targetId, { content: updated_block.content })
      if (isHeading) {
        return insertAsFirstChild(updated, targetId, newBlock)
      }
      return insertAfter(updated, targetId, newBlock)
    })

    syncBlockContent(targetId, updated_block.content ?? contentBefore)
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
    await deleteBlock(params.blockId, ctx.operationId)
  } catch (err) {
    console.error('[delete] 删除块失败:', err)
    return
  }

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
  const block = findBlockById(tree, params.blockId)
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
      operation_id: ctx.operationId,
    })

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

/**
 * Convert — Markdown 快捷键转换块类型
 *
 * 悲观更新：调 updateBlock API 同时更新 block_type + content，
 * 成功后用真实数据刷新 UI（触发组件重路由，如 paragraph → heading）。
 */
export async function executeConvertBlock(
  ctx: CommandContext,
  params: ConvertParams,
): Promise<void> {
  ctx.cancelPendingSave(params.blockId)

  try {
    // 检查旧类型是否为 heading（用于判断是否需要 refetch 整棵树）
    const oldBlock = findBlockById(ctx.getTree(), params.blockId)
    const wasHeading = oldBlock?.block_type.type === 'heading'
    const becomesHeading = params.blockType.type === 'heading'
    // heading 相关的类型变化会触发后端自动嵌套（reparent），需要 refetch 整棵树
    const needsRefetch = wasHeading || becomesHeading

    const updated = await updateBlock(params.blockId, {
      block_type: params.blockType,
      content: params.content,
      operation_id: ctx.operationId,
    })

    if (needsRefetch) {
      await ctx.refetchDocument()
    } else {
      ctx.setTreeSync((prev) =>
        updateBlockInTree(prev, params.blockId, {
          content: updated.content,
          block_type: updated.block_type,
          version: updated.version,
          modified: updated.modified,
        }),
      )
    }

    // 块类型变更后 React 会卸载旧组件、挂载新组件，
    // 新组件的 mount effect 会同步 contentEditable 内容，无需手动 syncBlockContent。
    // 但需要重新聚焦以确保光标不丢失。
    requestAnimationFrame(() => focusBlock(params.blockId, params.content.length))
  } catch (err) {
    console.error('[convert] 转换块类型失败:', err)
  }
}
