/**
 * BlockNode 树的不可变操作 — 纯函数
 *
 * 所有函数接受 BlockNode[] 并返回新的 BlockNode[]，不修改原数组。
 * 利用 reference equality 检测子树是否变化，避免不必要的重渲染。
 */

import type { BlockNode } from '@/types/api'

// ─── 查找 ───

/** 获取扁平化的深度优先列表 */
export function flattenTree(tree: BlockNode[]): BlockNode[] {
  const result: BlockNode[] = []
  for (const block of tree) {
    result.push(block)
    result.push(...flattenTree(block.children))
  }
  return result
}

/** 查找前一个块（深度优先遍历） */
export function findPrevBlock(tree: BlockNode[], blockId: string): BlockNode | null {
  const flat = flattenTree(tree)
  const idx = flat.findIndex((b) => b.id === blockId)
  return idx > 0 ? flat[idx - 1] : null
}

/** 查找后一个块（深度优先遍历） */
export function findNextBlock(tree: BlockNode[], blockId: string): BlockNode | null {
  const flat = flattenTree(tree)
  const idx = flat.findIndex((b) => b.id === blockId)
  return idx >= 0 && idx < flat.length - 1 ? flat[idx + 1] : null
}

// ─── 插入 ───

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


