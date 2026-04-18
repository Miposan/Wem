/**
 * BlockNode 树的不可变操作 — 纯函数
 *
 * 所有函数接受 BlockNode[] 并返回新的 BlockNode[]，不修改原数组。
 * 利用 reference equality 检测子树是否变化，避免不必要的重渲染。
 */

import type { BlockNode } from '@/types/api'

// ─── 查找 ───

/** 获取扁平化的深度优先列表（累加器模式，O(n)） */
export function flattenTree(tree: BlockNode[]): BlockNode[] {
  const result: BlockNode[] = []
  function walk(blocks: BlockNode[]) {
    for (const block of blocks) {
      result.push(block)
      walk(block.children)
    }
  }
  walk(tree)
  return result
}

/** 通过 ID 查找块（DFS，提前返回，不创建中间数组） */
export function findBlockById(tree: BlockNode[], blockId: string): BlockNode | null {
  for (const block of tree) {
    if (block.id === blockId) return block
    const found = findBlockById(block.children, blockId)
    if (found) return found
  }
  return null
}

/** 查找前一个块（深度优先前序，单趟遍历） */
export function findPrevBlock(tree: BlockNode[], blockId: string): BlockNode | null {
  let prev: BlockNode | null = null
  function walk(blocks: BlockNode[]): boolean {
    for (const block of blocks) {
      if (block.id === blockId) return true
      prev = block
      if (walk(block.children)) return true
    }
    return false
  }
  walk(tree)
  return prev
}

/** 查找后一个块（深度优先前序，单趟遍历） */
export function findNextBlock(tree: BlockNode[], blockId: string): BlockNode | null {
  let found = false
  let result: BlockNode | null = null
  function walk(blocks: BlockNode[]): boolean {
    for (const block of blocks) {
      if (result) return true
      if (found) {
        result = block
        return true
      }
      if (block.id === blockId) found = true
      walk(block.children)
    }
    return false
  }
  walk(tree)
  return result
}

// ─── 插入 ───

/** 在指定父块下插入为第一个子块 */
export function insertAsFirstChild(tree: BlockNode[], parentId: string, newBlock: BlockNode): BlockNode[] {
  return tree.map((block) => {
    if (block.id === parentId) {
      return { ...block, children: [newBlock, ...block.children] }
    }
    const updated = insertAsFirstChild(block.children, parentId, newBlock)
    if (updated !== block.children) {
      return { ...block, children: updated }
    }
    return block
  })
}

/** 在指定块之后插入新块（顶层或嵌套） */
export function insertAfter(tree: BlockNode[], afterId: string | null, newBlock: BlockNode): BlockNode[] {
  if (afterId === null) {
    return [newBlock, ...tree]
  }

  for (let i = 0; i < tree.length; i++) {
    if (tree[i].id === afterId) {
      return [...tree.slice(0, i + 1), newBlock, ...tree.slice(i + 1)]
    }
  }

  // 未在当前层级找到，递归搜索子节点
  return tree.map((block) => {
    const updated = insertAfter(block.children, afterId, newBlock)
    if (updated !== block.children) {
      return { ...block, children: updated }
    }
    return block
  })
}

// ─── 删除 ───

/** 从树中移除指定块（及其子块） */
export function removeBlock(tree: BlockNode[], blockId: string): BlockNode[] {
  const filtered = tree.filter((b) => b.id !== blockId)
  if (filtered.length !== tree.length) return filtered

  return tree.map((block) => {
    const updated = removeBlock(block.children, blockId)
    if (updated !== block.children) {
      return { ...block, children: updated }
    }
    return block
  })
}

// ─── 更新 ───

/** 更新指定块的属性 */
export function updateBlockInTree(
  tree: BlockNode[],
  blockId: string,
  updates: Partial<Pick<BlockNode, 'content' | 'block_type' | 'properties' | 'version' | 'modified'>>,
): BlockNode[] {
  return tree.map((block) => {
    if (block.id === blockId) {
      return { ...block, ...updates }
    }
    const updatedChildren = updateBlockInTree(block.children, blockId, updates)
    if (updatedChildren !== block.children) {
      return { ...block, children: updatedChildren }
    }
    return block
  })
}

/** 替换指定 ID 的块为另一个块（保持子树或替换） */
export function replaceBlockInTree(
  tree: BlockNode[],
  targetId: string,
  replacement: BlockNode,
): BlockNode[] {
  for (let i = 0; i < tree.length; i++) {
    if (tree[i].id === targetId) {
      return [...tree.slice(0, i), replacement, ...tree.slice(i + 1)]
    }
  }
  return tree.map((block) => {
    const updated = replaceBlockInTree(block.children, targetId, replacement)
    if (updated !== block.children) {
      return { ...block, children: updated }
    }
    return block
  })
}

// ─── 移动（拖拽乐观更新） ───

/** 在指定块之前插入新块（顶层或嵌套） */
export function insertBefore(tree: BlockNode[], beforeId: string, newBlock: BlockNode): BlockNode[] {
  for (let i = 0; i < tree.length; i++) {
    if (tree[i].id === beforeId) {
      return [...tree.slice(0, i), newBlock, ...tree.slice(i)]
    }
  }
  return tree.map((block) => {
    const updated = insertBefore(block.children, beforeId, newBlock)
    if (updated !== block.children) {
      return { ...block, children: updated }
    }
    return block
  })
}

/** 将块替换为其子块列表（子块提升到父级） */
export function replaceWithChildren(tree: BlockNode[], blockId: string): BlockNode[] {
  for (let i = 0; i < tree.length; i++) {
    if (tree[i].id === blockId) {
      return [...tree.slice(0, i), ...tree[i].children, ...tree.slice(i + 1)]
    }
  }
  return tree.map((block) => {
    const updated = replaceWithChildren(block.children, blockId)
    if (updated !== block.children) {
      return { ...block, children: updated }
    }
    return block
  })
}

/**
 * 移动单个块（move-block 乐观更新）
 *
 * 对于有子块的 heading：子块留在原位（detach），heading 不带子块移到目标位置。
 * 对于其他块：直接移动（非 heading 通常无子块）。
 */
export function moveBlockInTree(
  tree: BlockNode[],
  blockId: string,
  target: { blockId: string; position: 'before' | 'after' | 'child' },
): BlockNode[] {
  const block = findBlockById(tree, blockId)
  if (!block) return tree

  const targetBlock = findBlockById(tree, target.blockId)
  if (!targetBlock) return tree

  if (blockId === target.blockId) return tree

  // heading 有子块时 detach（子块留在 heading 原位置）
  const detachChildren = block.block_type.type === 'heading' && block.children.length > 0
  const movedBlock: BlockNode = detachChildren ? { ...block, children: [] } : block

  // 从旧位置移除（heading detach 时用 replaceWithChildren 保留子块）
  let newTree = detachChildren
    ? replaceWithChildren(tree, blockId)
    : removeBlock(tree, blockId)

  // 插入到目标位置
  switch (target.position) {
    case 'before':
      return insertBefore(newTree, target.blockId, movedBlock)
    case 'after':
      return insertAfter(newTree, target.blockId, movedBlock)
    case 'child':
      return insertAsFirstChild(newTree, target.blockId, movedBlock)
  }
}

/**
 * 移动子树（move-heading-tree 乐观更新）
 *
 * heading 及其全部子块作为整体移动到目标位置。
 */
export function moveSubtreeInTree(
  tree: BlockNode[],
  blockId: string,
  target: { blockId: string; position: 'before' | 'after' | 'child' },
): BlockNode[] {
  const block = findBlockById(tree, blockId)
  if (!block) return tree

  const targetBlock = findBlockById(tree, target.blockId)
  if (!targetBlock) return tree

  if (blockId === target.blockId) return tree

  // 移除整棵子树
  const newTree = removeBlock(tree, blockId)

  // 整棵子树插入到目标位置
  switch (target.position) {
    case 'before':
      return insertBefore(newTree, target.blockId, block)
    case 'after':
      return insertAfter(newTree, target.blockId, block)
    case 'child':
      return insertAsFirstChild(newTree, target.blockId, block)
  }
}


