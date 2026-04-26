/**
 * TocPanel — 文档目录（Table of Contents）
 *
 * 从编辑器的 heading 块中提取标题结构，显示为可点击的目录树。
 * 放置在编辑器右侧面板中。
 */

import type { BlockNode } from '@/types/api'

// ─── Types ───

export interface TocItem {
  id: string
  level: number
  text: string
}

// ─── Helpers ───

/** 从块树中提取所有 heading，返回扁平列表 */
export function extractTocItems(tree: BlockNode[]): TocItem[] {
  const items: TocItem[] = []

  function walk(nodes: BlockNode[]) {
    for (const node of nodes) {
      if (node.block_type.type === 'heading') {
        const level = (node.block_type as { level: number }).level
        // 从 content 中提取纯文本（content 是 HTML）
        const text = stripHtml(node.content || '')
        items.push({ id: node.id, level, text: text || '无标题' })
      }
      if (node.children.length > 0) {
        walk(node.children)
      }
    }
  }

  walk(tree)
  return items
}

/** 简单的 HTML → 纯文本（strip tags） */
function stripHtml(html: string): string {
  return html.replace(/<[^>]*>/g, '').trim()
}

// ─── Props ───

interface TocPanelProps {
  items: TocItem[]
  activeHeadingId?: string
  onHeadingClick: (blockId: string) => void
}

// ─── Component ───

export function TocPanel({ items, activeHeadingId, onHeadingClick }: TocPanelProps) {
  if (items.length === 0) {
    return (
      <div className="p-4 text-sm text-muted-foreground">
        <p>暂无标题</p>
      </div>
    )
  }

  return (
    <nav className="py-2">
      {items.map((item) => {
        const isActive = item.id === activeHeadingId
        // 根据层级缩进：H1=0, H2=1, H3=2, ...
        const indent = Math.max(0, item.level - 1) * 12

        return (
          <button
            key={item.id}
            className={`
              w-full text-left px-3 py-1 text-sm truncate transition-colors cursor-pointer
              ${isActive
                ? 'text-foreground font-medium bg-accent/60'
                : 'text-muted-foreground hover:text-foreground hover:bg-accent/30'
              }
            `}
            style={{ paddingLeft: `${indent + 12}px` }}
            onClick={() => onHeadingClick(item.id)}
            title={item.text}
          >
            {item.text}
          </button>
        )
      })}
    </nav>
  )
}
