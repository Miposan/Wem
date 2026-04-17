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


