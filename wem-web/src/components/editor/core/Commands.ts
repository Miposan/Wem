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

import type { BlockNode, BlockType, ListBlockType } from '@/types/api'
import { makeListType, makeListItemType } from '@/types/api'
import { deleteBlock, createBlock, mergeBlock, updateBlock, moveBlock, moveTree } from '@/api/client'
import {
  flattenTree,
  findBlockById,
  insertAfter,
  insertAsFirstChild,
  insertBefore,
  removeBlock,
  removeBlocks,
  updateBlockInTree,
  replaceBlockInTree,
  findPrevBlock,
  findNextBlock,
  moveBlockInTree,
  moveHeadingTreeInTree,
} from './BlockOperations'
import { focusBlock, focusBlockEnd, getCursorPosition, syncBlockContent, splitContentAtCursor, findEditable } from './SelectionManager'

function pendingBlockId(): string {
  return `_pending:${crypto.randomUUID()}`
}

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
  blockId: string
  target: { blockId: string; position: 'before' | 'after' }
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

  // 使用 DOM 直接拆分行内格式内容（避免 markdown 偏移错位）
  const editEl = findEditable(targetId)
  const { before: contentBefore, after: contentAfter } = editEl
    ? splitContentAtCursor(editEl)
    : { before: block.content ?? '', after: '' }

  const text = block.content ?? ''

  ctx.cancelPendingSave(targetId)

  const isHeading = block.block_type.type === 'heading'
  const isListItem = block.block_type.type === 'listItem'

  // ── 确定新块的类型和插入策略 ──
  // ListItem split → 新块也是 ListItem（同列表内的兄弟）
  // Heading split → 新块是 Paragraph，作为 heading 的子块
  // 其他 → 同类型兄弟块
  const newBlockType: BlockType =
    isHeading ? { type: 'paragraph' } : { ...block.block_type }

  // 空的 ListItem split → 退出列表（不创建新 ListItem）
  // 在空 ListItem 里按 Enter 的语义是"结束列表"
  if (isListItem && text.length === 0) {
    // 删除当前空 ListItem，在 List 后面创建一个 Paragraph
    const prev = findPrevBlock(tree, targetId)
    ctx.setTreeSync((prevTree) => removeBlock(prevTree, targetId))
    if (prev) focusBlockEnd(prev.id)

    try {
      await deleteBlock(targetId, ctx.editorId)
      // refetch 以处理"List 变空后应自动删除"等后端逻辑
      await ctx.refetchDocument()
    } catch (err) {
      console.error('[split] 退出列表失败，回滚:', err)
      ctx.refetchDocument()
    }
    return
  }

  // ── Heading at offset 0: 在标题前插入空段落（Notion 行为）──
  // 光标在标题开头按 Enter → 在标题上方创建空段落，标题内容不变
  if (isHeading && offset === 0 && text.length > 0) {
    const placeholderId = pendingBlockId()
    const placeholder: BlockNode = {
      id: placeholderId,
      parent_id: block.parent_id,
      block_type: { type: 'paragraph' },
      content: '',
      children: [],
    }

    ctx.setTreeSync((prev) => insertBefore(prev, targetId, placeholder))
    requestAnimationFrame(() => focusBlock(placeholderId))

    try {
      const prevSib = findPrevSibling(tree, targetId)
      const paragraph = prevSib
        ? await createBlock({
            parent_id: block.parent_id,
            after_id: prevSib.id,
            block_type: { type: 'paragraph' },
            content: '',
            editor_id: ctx.editorId,
          })
        : await createBlock({
            parent_id: block.parent_id,
            block_type: { type: 'paragraph' },
            content: '',
            editor_id: ctx.editorId,
          }).then(async (p) => {
            await moveBlock(p.id, { before_id: targetId, editor_id: ctx.editorId ?? '' })
            return p
          })

      const realBlock: BlockNode = { ...paragraph, children: [] }
      ctx.setTreeSync((prev) => replaceBlockInTree(prev, placeholderId, realBlock))
      focusBlock(paragraph.id)
    } catch (err) {
      console.error('[split] 在标题前插入段落失败，回滚:', err)
      ctx.refetchDocument()
    }
    return
  }

  // ── 乐观更新：立即更新 UI ──
  const placeholderId = pendingBlockId()
  const placeholder: BlockNode = {
    ...block,
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

  // ── 后台 API：update + insert 组合（替代显式 split）──
  try {
    // 1. 更新当前块内容（截断为前半段）
    const updated = await updateBlock(targetId, {
      content: contentBefore,
      editor_id: ctx.editorId,
    })

    // 2. 创建新块
    //    Heading → parent_id=targetId（子块）
    //    ListItem → parent_id=同 List（兄弟）
    //    其他 → parent_id=同父（兄弟）
    const newBlock = await createBlock({
      parent_id: isHeading ? targetId : block.parent_id,
      block_type: newBlockType,
      content: contentAfter,
      ...(isHeading ? {} : { after_id: targetId }),
      editor_id: ctx.editorId,
    })

    // 用后端真实数据替换占位块 + 同步当前块内容
    const realBlock: BlockNode = { ...newBlock, children: [] }
    ctx.setTreeSync((prev) => {
      const withReal = replaceBlockInTree(prev, placeholderId, realBlock)
      return updateBlockInTree(withReal, targetId, { content: updated.content })
    })

    syncBlockContent(targetId, updated.content)
    focusBlock(newBlock.id)
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
 * MergeNext — 将下一个块的内容合并到当前块（Delete 键在末尾触发）
 *
 * 与 executeMerge 方向相反：当前块保留，下一个块的内容追加到末尾。
 * 后端 merge API 仅支持 previous 方向，所以这里用 updateBlock + deleteBlock 组合实现。
 */
export async function executeMergeNext(ctx: CommandContext, params: MergeParams): Promise<void> {
  const tree = ctx.getTree()
  const block = findBlockById(tree, params.blockId)
  if (!block) return

  const next = findNextBlock(tree, params.blockId)
  if (!next) return

  const mergedContent = (block.content ?? '') + (next.content ?? '')
  const mergePoint = (block.content ?? '').length

  ctx.cancelPendingSave(params.blockId)
  ctx.cancelPendingSave(next.id)

  // ── 乐观更新：立即合并 UI ──
  ctx.setTreeSync((prevTree) => {
    const updated = updateBlockInTree(prevTree, params.blockId, { content: mergedContent })
    return removeBlock(updated, next.id)
  })
  syncBlockContent(params.blockId, mergedContent)
  focusBlock(params.blockId, mergePoint)

  // ── 后台 API ──
  try {
    const updated = await updateBlock(params.blockId, {
      content: mergedContent,
      editor_id: ctx.editorId,
    })
    await deleteBlock(next.id, ctx.editorId)

    ctx.setTreeSync((prevTree) =>
      updateBlockInTree(prevTree, params.blockId, { content: updated.content }),
    )
    syncBlockContent(params.blockId, updated.content ?? '')
  } catch (err) {
    console.error('[merge-next] 合并下一块失败，回滚:', err)
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
 * list 相关的类型变化需要创建 ListItem 子块，也需 refetch。
 */
export async function executeConvertBlock(
  ctx: CommandContext,
  params: ConvertParams,
): Promise<void> {
  ctx.cancelPendingSave(params.blockId)

  // 检查旧类型（用于判断是否需要 refetch 整棵树）
  const oldBlock = findBlockById(ctx.getTree(), params.blockId)
  if (!oldBlock) return

  const wasHeading = oldBlock.block_type.type === 'heading'
  const becomesHeading = params.blockType.type === 'heading'
  const becomesList = params.blockType.type === 'list'
  const becomesThematicBreak = params.blockType.type === 'thematicBreak'
  // heading / list 变化涉及结构性改动，需要 refetch 整棵树
  const needsRefetch = wasHeading || becomesHeading || becomesList

  // ── List 转换特殊处理 ──
  // Paragraph → List：前端仅做乐观更新 + 调用 updateBlock。
  // 后端 on_type_changed 会自动创建 ListItem 子块（迁移 content），无需前端 createBlock。
  if (becomesList) {
    const listContent = params.content

    // 1. 乐观更新：将当前块变为空 List 容器，插入一个 placeholder ListItem
    //    等 refetchDocument 后用后端真实数据替换
    const placeholderId = pendingBlockId()
    const placeholder: BlockNode = {
      ...oldBlock,
      id: placeholderId,
      block_type: makeListItemType(),
      content: listContent,
      children: [],
    }

    ctx.setTreeSync((prev) => {
      const updated = updateBlockInTree(prev, params.blockId, {
        content: '',
        block_type: params.blockType,
      })
      return insertAsFirstChild(updated, params.blockId, placeholder)
    })

    requestAnimationFrame(() => focusBlock(placeholderId, listContent.length))

    // 2. 后台 API — 仅 updateBlock，后端 on_type_changed 自动创建 ListItem
    try {
      await updateBlock(params.blockId, {
        block_type: params.blockType,
        content: '',
        editor_id: ctx.editorId,
      })

      // refetch 获取后端自动创建的 ListItem，替换 placeholder
      await ctx.refetchDocument()

      // refetch 后 List 的第一个 ListItem 就是真实 ID，聚焦它
      const tree = ctx.getTree()
      const listBlock = findBlockById(tree, params.blockId)
      if (listBlock && listBlock.children.length > 0) {
        focusBlock(listBlock.children[0].id, listContent.length)
      }
    } catch (err) {
      console.error('[convert] 转换列表失败，回滚:', err)
      ctx.refetchDocument()
    }
    return
  }

  // ── ThematicBreak 转换特殊处理 ──
  // 分割线不可编辑，转换后立即在其后创建一个 Paragraph，保持连续输入体验。
  if (becomesThematicBreak) {
    const placeholderId = pendingBlockId()
    const placeholder: BlockNode = {
      ...oldBlock,
      id: placeholderId,
      block_type: { type: 'paragraph' },
      content: '',
      children: [],
    }

    ctx.setTreeSync((prev) => {
      const updated = updateBlockInTree(prev, params.blockId, {
        content: '',
        block_type: params.blockType,
      })
      return insertAfter(updated, params.blockId, placeholder)
    })
    requestAnimationFrame(() => focusBlock(placeholderId))

    try {
      const updated = await updateBlock(params.blockId, {
        block_type: params.blockType,
        content: '',
        editor_id: ctx.editorId,
      })
      const paragraph = await createBlock({
        parent_id: oldBlock.parent_id,
        after_id: params.blockId,
        block_type: { type: 'paragraph' },
        content: '',
        editor_id: ctx.editorId,
      })

      const realBlock: BlockNode = { ...paragraph, children: [] }
      ctx.setTreeSync((prev) => {
        const withReal = replaceBlockInTree(prev, placeholderId, realBlock)
        return updateBlockInTree(withReal, params.blockId, {
          content: updated.content,
          block_type: updated.block_type,
          version: updated.version,
          modified: updated.modified,
        })
      })
      focusBlock(paragraph.id)
    } catch (err) {
      console.error('[convert] 转换分割线失败，回滚:', err)
      ctx.refetchDocument()
    }
    return
  }

  // ── CodeBlock / Heading / 其他普通转换 ──
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
  if (target.position === 'before') {
    moveReq.before_id = target.blockId
  } else {
    moveReq.after_id = target.blockId
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
 * MoveHeadingTree — 折叠 heading / list 子树整体拖拽移动
 *
 * 乐观更新：先在 UI 中移动整棵子树（纯前端树操作），后台调 moveTree API，失败则 refetch 回滚。
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
  ctx.setTreeSync((prev) => moveHeadingTreeInTree(prev, blockId, target))
  focusBlock(blockId)

  // 构建 moveTree API 请求参数
  const moveReq: Record<string, string> = { editor_id: ctx.editorId ?? '' }
  if (target.position === 'before') {
    moveReq.before_id = target.blockId
  } else {
    moveReq.after_id = target.blockId
  }

  // ── 后台 API + refetch 修正 ──
  try {
    await moveTree(blockId, moveReq)
    await ctx.refetchDocument()
    focusBlock(blockId)
  } catch (err) {
    console.error('[move-tree] 移动子树失败，回滚:', err)
    ctx.refetchDocument()
  }
}

/**
 * ToggleListType — 切换 List 块的有序/无序类型
 */
export async function executeToggleListType(
  ctx: CommandContext,
  blockId: string,
): Promise<void> {
  const tree = ctx.getTree()
  const block = findBlockById(tree, blockId)
  if (!block || block.block_type.type !== 'list') return

  const currentOrdered = (block.block_type as ListBlockType).ordered
  const newType = makeListType(!currentOrdered)

  ctx.cancelPendingSave(blockId)

  // 乐观更新
  ctx.setTreeSync((prev) =>
    updateBlockInTree(prev, blockId, { block_type: newType }),
  )

  try {
    await updateBlock(blockId, {
      block_type: newType,
      editor_id: ctx.editorId,
    })
  } catch (err) {
    console.error('[toggle-list-type] 切换列表类型失败，回滚:', err)
    ctx.refetchDocument()
  }
}

/**
 * IndentListItem — 将当前 ListItem 缩进到前一个 ListItem 的子列表下。
 */
export async function executeIndentListItem(
  ctx: CommandContext,
  blockId: string,
): Promise<void> {
  const tree = ctx.getTree()
  const block = findBlockById(tree, blockId)
  if (!block || block.block_type.type !== 'listItem') return

  const parentList = findParentList(tree, blockId)
  if (!parentList || parentList.block_type.type !== 'list') return

  const index = parentList.children.findIndex((child) => child.id === blockId)
  if (index <= 0) return

  const previousItem = parentList.children[index - 1]
  const existingChildList = previousItem.children.find((child) => child.block_type.type === 'list')
  const ordered = (parentList.block_type as ListBlockType).ordered

  ctx.cancelPendingSave(blockId)

  try {
    let targetListId = existingChildList?.id
    if (!targetListId) {
      const childList = await createBlock({
        parent_id: previousItem.id,
        block_type: makeListType(ordered),
        content: '',
        editor_id: ctx.editorId,
      })
      targetListId = childList.id
    }

    await moveBlock(blockId, {
      target_parent_id: targetListId,
      editor_id: ctx.editorId ?? '',
    })

    await ctx.refetchDocument()
    focusBlock(blockId)
  } catch (err) {
    console.error('[indent-list-item] 缩进列表项失败，回滚:', err)
    ctx.refetchDocument()
  }
}

/**
 * OutdentListItem — 将当前 ListItem 提升到父级 ListItem 之后。
 */
export async function executeOutdentListItem(
  ctx: CommandContext,
  blockId: string,
): Promise<void> {
  const tree = ctx.getTree()
  const block = findBlockById(tree, blockId)
  if (!block || block.block_type.type !== 'listItem') return

  const parentList = findParentList(tree, blockId)
  if (!parentList) return

  const parentItem = findParentListItem(tree, parentList.id)
  ctx.cancelPendingSave(blockId)

  try {
    if (parentItem) {
      await moveBlock(blockId, {
        after_id: parentItem.id,
        editor_id: ctx.editorId ?? '',
      })
    } else {
      await executeExitList(ctx, blockId)
      return
    }

    await ctx.refetchDocument()
    focusBlock(blockId)
  } catch (err) {
    console.error('[outdent-list-item] 反缩进列表项失败，回滚:', err)
    ctx.refetchDocument()
  }
}

/**
 * ExitList — 空 ListItem 按 Enter：删除该项并在外层 List 后创建 Paragraph。
 */
export async function executeExitList(
  ctx: CommandContext,
  blockId: string,
): Promise<void> {
  const tree = ctx.getTree()
  const block = findBlockById(tree, blockId)
  if (!block || block.block_type.type !== 'listItem') return

  const parentList = findParentList(tree, blockId)
  if (!parentList) return

  ctx.cancelPendingSave(blockId)

  try {
    await deleteBlock(blockId, ctx.editorId)
    const paragraph = await createBlock({
      parent_id: parentList.parent_id,
      after_id: parentList.id,
      block_type: { type: 'paragraph' },
      content: '',
      editor_id: ctx.editorId,
    })

    await ctx.refetchDocument()
    focusBlock(paragraph.id)
  } catch (err) {
    console.error('[exit-list] 退出列表失败，回滚:', err)
    ctx.refetchDocument()
  }
}

/**
 * ExitCodeBlock — Ctrl/Cmd+Enter：在代码块后创建一个 Paragraph。
 */
export async function executeExitCodeBlock(
  ctx: CommandContext,
  blockId: string,
  content: string,
): Promise<void> {
  const tree = ctx.getTree()
  const block = findBlockById(tree, blockId)
  if (!block || block.block_type.type !== 'codeBlock') return

  ctx.cancelPendingSave(blockId)

  const placeholderId = pendingBlockId()
  const placeholder: BlockNode = {
    ...block,
    id: placeholderId,
    block_type: { type: 'paragraph' },
    content: '',
    children: [],
  }

  ctx.setTreeSync((prev) => {
    const updated = updateBlockInTree(prev, blockId, { content })
    return insertAfter(updated, blockId, placeholder)
  })
  requestAnimationFrame(() => focusBlock(placeholderId))

  try {
    const updated = await updateBlock(blockId, {
      content,
      editor_id: ctx.editorId,
    })
    const paragraph = await createBlock({
      parent_id: block.parent_id,
      after_id: blockId,
      block_type: { type: 'paragraph' },
      content: '',
      editor_id: ctx.editorId,
    })

    const realBlock: BlockNode = { ...paragraph, children: [] }
    ctx.setTreeSync((prev) => {
      const withReal = replaceBlockInTree(prev, placeholderId, realBlock)
      return updateBlockInTree(withReal, blockId, { content: updated.content })
    })
    focusBlock(paragraph.id)
  } catch (err) {
    console.error('[exit-code-block] 退出代码块失败，回滚:', err)
    ctx.refetchDocument()
  }
}

/**
 * DeleteRange — 批量删除选区中的多个块
 *
 * 先乐观更新 UI（立即消失），再并行发 API 删除。
 * 聚焦到被删范围之前的块。
 */
export async function executeDeleteRange(
  ctx: CommandContext,
  params: { blockIds: string[] },
): Promise<void> {
  const { blockIds } = params
  if (blockIds.length === 0) return

  // 记录删除范围之前的块
  const flatBefore = flattenTree(ctx.getTree())
  const firstDeletedIdx = flatBefore.findIndex((b) => b.id === blockIds[0])
  const blockBeforeRange = firstDeletedIdx > 0 ? flatBefore[firstDeletedIdx - 1] : null

  for (const id of blockIds) {
    ctx.cancelPendingSave(id)
  }

  // 乐观更新：立即从 UI 移除
  ctx.setTreeSync((prev) => removeBlocks(prev, new Set(blockIds)))

  // 聚焦到被删范围之前的块
  if (blockBeforeRange) {
    const afterDelete = findBlockById(ctx.getTree(), blockBeforeRange.id)
    if (afterDelete) {
      focusBlock(afterDelete.id, (afterDelete.content ?? '').length)
    } else {
      const flat = flattenTree(ctx.getTree())
      if (flat.length > 0) focusBlock(flat[0].id, 0)
    }
  } else {
    const flat = flattenTree(ctx.getTree())
    if (flat.length > 0) focusBlock(flat[0].id, 0)
  }

  // 后台并行删除
  await Promise.all(blockIds.map((id) => deleteBlock(id, ctx.editorId).catch(() => {})))
}

/**
 * AddBlockAfter — 在指定块之后插入新段落（gutter "+" 按钮触发）
 */
export async function executeAddBlockAfter(
  ctx: CommandContext,
  params: { afterBlockId: string; documentId: string },
): Promise<void> {
  const created = await createBlock({
    parent_id: params.documentId,
    block_type: { type: 'paragraph' },
    content: '',
    after_id: params.afterBlockId,
    editor_id: ctx.editorId,
  })
  const newBlock: BlockNode = { ...created, children: [] }
  ctx.setTreeSync((prev) => insertAfter(prev, params.afterBlockId, newBlock))
  requestAnimationFrame(() => focusBlock(created.id))
}

/** 查找 ListItem 所属的 List 父容器 */
function findParentList(tree: BlockNode[], listItemBlockId: string): BlockNode | null {
  for (const node of tree) {
    const found = findParentListInSubtree(node, listItemBlockId)
    if (found) return found
  }
  return null
}

function findParentListInSubtree(parent: BlockNode, targetId: string): BlockNode | null {
  for (const child of parent.children) {
    if (child.id === targetId && parent.block_type.type === 'list') {
      return parent
    }
    const found = findParentListInSubtree(child, targetId)
    if (found) return found
  }
  return null
}

/** 查找指定块的直接父块 */
function findParentBlock(tree: BlockNode[], childId: string): BlockNode | null {
  for (const node of tree) {
    if (node.children.some((child) => child.id === childId)) return node
    const found = findParentBlock(node.children, childId)
    if (found) return found
  }
  return null
}

function findParentListItem(tree: BlockNode[], listBlockId: string): BlockNode | null {
  const parent = findParentBlock(tree, listBlockId)
  return parent?.block_type.type === 'listItem' ? parent : null
}

/** 查找指定块在父级 children 中的前一个兄弟 */
function findPrevSibling(tree: BlockNode[], blockId: string): BlockNode | null {
  const parent = findParentBlock(tree, blockId)
  const siblings = parent ? parent.children : tree
  const idx = siblings.findIndex((s) => s.id === blockId)
  return idx > 0 ? siblings[idx - 1] : null
}
