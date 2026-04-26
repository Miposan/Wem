import { createContext, useContext } from 'react'
import type { BlockNode } from '@/types/api'

/**
 * HeadingNumberingContext — 标题虚拟编号
 *
 * 提供一个 blockId → 编号字符串的只读映射。
 * 编号在渲染时实时计算，不持久化。
 */

const HeadingNumberingContext = createContext<ReadonlyMap<string, string> | null>(null)

export function useHeadingNumber(blockId: string): string | undefined {
  const map = useContext(HeadingNumberingContext)
  return map?.get(blockId)
}

export function HeadingNumberingProvider({
  map,
  children,
}: {
  map: ReadonlyMap<string, string> | null
  children: React.ReactNode
}) {
  return (
    <HeadingNumberingContext.Provider value={map}>
      {children}
    </HeadingNumberingContext.Provider>
  )
}

// ─── 编号计算 ───

/** 从 BlockNode 树计算所有标题的虚拟编号 */
export function computeHeadingNumbers(
  blocks: BlockNode[],
): Map<string, string> {
  const map = new Map<string, string>()
  // counters[0] → H1, counters[1] → H2, ... counters[5] → H6
  const counters = [0, 0, 0, 0, 0, 0]

  walk(blocks, counters, map)
  return map
}

function walk(
  blocks: BlockNode[],
  counters: number[],
  map: Map<string, string>,
) {
  for (const block of blocks) {
    if (block.block_type.type === 'heading') {
      const level = (block.block_type.level as number) ?? 2
      // 当前级别 +1，所有更深层级归零
      counters[level - 1]++
      for (let i = level; i < 6; i++) counters[i] = 0

      // 拼接编号：1 / 1.2 / 1.2.3
      const parts: string[] = []
      for (let i = 0; i < level; i++) {
        if (counters[i] > 0) parts.push(String(counters[i]))
      }
      map.set(block.id, parts.join('.'))
    }

    if (block.children.length > 0) {
      walk(block.children, counters, map)
    }
  }
}
