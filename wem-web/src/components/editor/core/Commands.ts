/**
 * 编辑器 Command 模块
 *
 * 每个 Command 封装一个"用户意图"（split/merge/delete），返回 Promise。
 * 所有 Command 通过 OperationQueue 串行执行，保证操作有序。
 *
 * 乐观更新策略：先更新 UI（零延迟），后台发 API，用后端返回的真实数据做最终同步。
 * - 不在前端生成 ID：split 用临时占位块，API 返回后替换为真实数据
 * - 失败时通过 refetchDocument 回滚到后端真实状态
 * - 后端是唯一真相源，前端 UI 仅用于即时反馈
 */

import type { BlockNode, BlockType } from '@/types/api'
import { deleteBlock, splitBlock, mergeBlock, updateBlock, moveBlock, moveHeadingTree } from '@/api/client'
import {
  flattenTree,
  findBlockById,
  insertAfter,
  insertAsFirstChild,
  removeBlock,
  updateBlockInTree,
  replaceBlockInTree,
  findPrevBlock,
  findNextBlock,
  moveBlockInTree,
  moveSubtreeInTree,
} from './BlockOperations'
import { focusBlock, focusBlockEnd, getCursorPosition, syncBlockContent } from './SelectionManager'

// ─── Types ───

/** Command 执行上下文 */
export interface CommandContext {
  documentId: string
  /** 操作 ID（前端生成，用于 SSE 回声去重），仅结构操作需要 */
  editorId?: string
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

/** Move 参数（块拖拽移动） */
export interface MoveParams {
  /** 被移动的块 ID */
  blockId: string
  /** 放置目标 */
  target: {
    blockId: string
    position: 'before' | 'after' | 'child'
  }
}

// ─── Commands ───

/**
 * Split — 在光标处拆分段落
 *
 * 乐观更新：先更新 UI（当前块截断 + 占位块），后台调 API，用真实数据替换。
 * 占位块使用临时 ID（`_pending:xxx`），不发送给后端。
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

  // ── 乐观更新：立即更新 UI ──
  const placeholderId = `_pending:${Date.now()}`
  const placeholder: BlockNode = {
    id: placeholderId,
    block_type: newBlockType,
    content: contentAfter,
    children: [],
  }

  ctx.setTreeSync((prev) => {
    const updated = updateBlockInTree(prev, targetId, { content: contentBefore })
    if (isHeading) {
      return insertAsFirstChild(updated, targetId, placeholder)
    }
    return insertAfter(updated, targetId, placeholder)
  })

  syncBlockContent(targetId, contentBefore)
  // 光标移到占位块开头
  requestAnimationFrame(() => focusBlock(placeholderId))

  // ── 后台 API：后端生成真实 ID ──
  try {
    const { updated_block, new_block } = await splitBlock(targetId, {
      content_before: contentBefore,
      content_after: contentAfter,
      new_block_type: newBlockType,
      nest_under_parent: isHeading || undefined,
      editor_id: ctx.editorId,
    })

    // 用后端真实数据替换占位块 + 同步当前块内容
    const realBlock: BlockNode = { ...new_block, children: [] }
    ctx.setTreeSync((prev) => {
      // 先替换占位块为真实块
      const withReal = replaceBlockInTree(prev, placeholderId, realBlock)
      // 再同步当前块内容（后端可能有调整）
      return updateBlockInTree(withReal, targetId, { content: updated_block.content })
    })

    syncBlockContent(targetId, updated_block.content)
    focusBlock(new_block.id)
  } catch (err) {
    console.error('[split] 拆分块失败，回滚:', err)
    ctx.refetchDocument()
  }
}

/**
 * Delete — 删除空块
 *
 * 乐观更新：立即从 UI 树中移除块，后台调 API，失败则 refetch 回滚。
 */
export async function executeDelete(ctx: CommandContext, params: DeleteParams): Promise<void> {
  const tree = ctx.getTree()
  const flat = flattenTree(tree)
  if (flat.length <= 1) return

  const block = flat.find((b) => b.id === params.blockId)
  if (!block) return

  const prev = findPrevBlock(tree, params.blockId)

  ctx.cancelPendingSave(params.blockId)

  // ── 乐观更新：立即移除块 ──
  ctx.setTreeSync((prevTree) => removeBlock(prevTree, params.blockId))
  if (prev) focusBlockEnd(prev.id)

  // ── 后台 API ──
  try {
    await deleteBlock(params.blockId, ctx.editorId)
  } catch (err) {
    console.error('[delete] 删除块失败，回滚:', err)
    ctx.refetchDocument()
  }
}

/**
 * Merge — 将当前块内容合并到前一个块
 *
 * 乐观更新：先在 UI 中合并内容并移除当前块，后台调 API，用真实数据同步，失败则 refetch 回滚。
 */
export async function executeMerge(ctx: CommandContext, params: MergeParams): Promise<void> {
  const tree = ctx.getTree()
  const block = findBlockById(tree, params.blockId)
  if (!block) return

  const prev = findPrevBlock(tree, params.blockId)
  if (!prev) return

  const mergePoint = (prev.content ?? '').length
  const mergedContent = (prev.content ?? '') + (block.content ?? '')

  // 取消两个块的 debounce 定时器，防止旧内容覆盖合并结果
  ctx.cancelPendingSave(prev.id)
  ctx.cancelPendingSave(params.blockId)

  // ── 乐观更新：立即合并 UI ──
  ctx.setTreeSync((prevTree) => {
    const updated = updateBlockInTree(prevTree, prev.id, { content: mergedContent })
    return removeBlock(updated, params.blockId)
  })
  syncBlockContent(prev.id, mergedContent)
  focusBlock(prev.id, mergePoint)

  // ── 后台 API ──
  try {
    const { merged_block } = await mergeBlock(params.blockId, {
      direction: 'previous',
      editor_id: ctx.editorId,
    })

    // 用后端真实数据同步（后端可能有调整）
    ctx.setTreeSync((prevTree) =>
      updateBlockInTree(prevTree, merged_block.id, { content: merged_block.content }),
    )
    syncBlockContent(merged_block.id, merged_block.content ?? '')
  } catch (err) {
    console.error('[merge] 合并块失败，回滚:', err)
    ctx.refetchDocument()
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
 * 乐观更新：先在 UI 中更新 block_type + content，后台调 API，用真实数据同步。
 * heading 相关的类型变化会触发后端自动嵌套（reparent），此时 refetch 整棵树。
 */
export async function executeConvertBlock(
  ctx: CommandContext,
  params: ConvertParams,
): Promise<void> {
  ctx.cancelPendingSave(params.blockId)

  // 检查旧类型是否为 heading（用于判断是否需要 refetch 整棵树）
  const oldBlock = findBlockById(ctx.getTree(), params.blockId)
  const wasHeading = oldBlock?.block_type.type === 'heading'
  const becomesHeading = params.blockType.type === 'heading'
  // heading 相关的类型变化会触发后端自动嵌套（reparent），需要 refetch 整棵树
  const needsRefetch = wasHeading || becomesHeading

  // ── 乐观更新：立即更新 UI ──
  ctx.setTreeSync((prev) =>
    updateBlockInTree(prev, params.blockId, {
      content: params.content,
      block_type: params.blockType,
    }),
  )
  // 块类型变更后 React 会卸载旧组件、挂载新组件
  requestAnimationFrame(() => focusBlock(params.blockId, params.content.length))

  // ── 后台 API ──
  try {
    const updated = await updateBlock(params.blockId, {
      block_type: params.blockType,
      content: params.content,
      editor_id: ctx.editorId,
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
  } catch (err) {
    console.error('[convert] 转换块类型失败，回滚:', err)
    ctx.refetchDocument()
  }
}

/**
 * Move — 块拖拽移动
 *
 * 乐观更新：先在 UI 中移动块（纯前端树操作），后台调 API，失败则 refetch 回滚。
 * heading 有子块时前端先 detach 子块再移动 heading 本体，与后端语义一致。
 */
export async function executeMove(ctx: CommandContext, params: MoveParams): Promise<void> {
  const { blockId, target } = params
  const tree = ctx.getTree()

  const movingBlock = findBlockById(tree, blockId)
  if (!movingBlock) return

  const targetBlock = findBlockById(tree, target.blockId)
  if (!targetBlock) return

  if (blockId === target.blockId) return

  ctx.cancelPendingSave(blockId)

  // ── 乐观更新：立即在 UI 中移动块 ──
  ctx.setTreeSync((prev) => moveBlockInTree(prev, blockId, target))
  focusBlock(blockId)

  // 构建 moveBlock API 请求参数
  const moveReq: Record<string, string> = { editor_id: ctx.editorId ?? '' }

  switch (target.position) {
    case 'before':
    case 'after': {
      if (target.position === 'before') {
        moveReq.before_id = target.blockId
      } else {
        moveReq.after_id = target.blockId
      }
      break
    }
    case 'child':
      moveReq.target_parent_id = target.blockId
      break
  }

  // ── 后台 API + refetch 修正 ──
  try {
    await moveBlock(blockId, moveReq)
    await ctx.refetchDocument()
    focusBlock(blockId)
  } catch (err) {
    console.error('[move] 移动块失败，回滚:', err)
    ctx.refetchDocument()
  }
}

/**
 * MoveHeadingTree — 折叠 heading 子树整体拖拽移动
 *
 * 乐观更新：先在 UI 中移动整棵子树（纯前端树操作），后台调 API，失败则 refetch 回滚。
 * 后端的吸收逻辑（heading 移动后自动吸收后续同级节点）由 refetch 修正。
 */
export async function executeMoveHeadingTree(
  ctx: CommandContext,
  params: MoveParams,
): Promise<void> {
  const { blockId, target } = params
  const tree = ctx.getTree()

  const movingBlock = findBlockById(tree, blockId)
  if (!movingBlock) return

  const targetBlock = findBlockById(tree, target.blockId)
  if (!targetBlock) return

  if (blockId === target.blockId) return

  ctx.cancelPendingSave(blockId)

  // ── 乐观更新：立即在 UI 中移动子树 ──
  ctx.setTreeSync((prev) => moveSubtreeInTree(prev, blockId, target))
  focusBlock(blockId)

  // 构建 moveHeadingTree API 请求参数（只有 before_id / after_id，无 target_parent_id）
  const moveReq: Record<string, string> = { editor_id: ctx.editorId ?? '' }

  switch (target.position) {
    case 'before':
      moveReq.before_id = target.blockId
      break
    case 'after':
      moveReq.after_id = target.blockId
      break
    case 'child':
      // 作为目标块的第一个子块 → 用 after_id 指向目标块
      // 后端会把 heading 子树放到目标块之后（作为目标块的子节点）
      moveReq.after_id = target.blockId
      break
  }

  // ── 后台 API + refetch 修正 ──
  try {
    await moveHeadingTree(blockId, moveReq)
    // heading-tree 涉及吸收逻辑，refetch 确保与后端一致
    await ctx.refetchDocument()
    focusBlock(blockId)
  } catch (err) {
    console.error('[move-heading-tree] 移动 heading 子树失败，回滚:', err)
    ctx.refetchDocument()
  }
}
