/**
 * EmbedBlock — 嵌入块（iframe / 代码嵌入）
 *
 * iframe 类型：url 存储在 block_type.url 中
 * embed 类型：content 存储 embed 代码（HTML snippet）
 */

import { useState } from 'react'
import type { BlockNode } from '@/types/api'
import { CodeIcon, ExternalLinkIcon } from 'lucide-react'

interface EmbedBlockProps {
  block: BlockNode
  readonly: boolean
}

export function EmbedBlock({ block }: EmbedBlockProps) {
  const blockType = block.block_type
  const [collapsed, setCollapsed] = useState(false)

  // iframe 类型
  if (blockType.type === 'iframe') {
    const url = blockType.url
    if (!url) {
      return (
        <div className="wem-embedblock wem-embedblock-empty">
          <ExternalLinkIcon className="h-8 w-8 text-muted-foreground/40" />
          <span className="text-sm text-muted-foreground">嵌入 URL 为空</span>
        </div>
      )
    }

    return (
      <div className="wem-embedblock">
        <div className="wem-embedblock-wrapper">
          <iframe
            src={url}
            className="wem-embedblock-iframe"
            sandbox="allow-scripts allow-same-origin allow-popups allow-forms"
            loading="lazy"
          />
        </div>
      </div>
    )
  }

  // embed 类型 — content 存储 HTML snippet
  const content = block.content ?? ''
  if (!content) {
    return (
      <div className="wem-embedblock wem-embedblock-empty">
        <CodeIcon className="h-8 w-8 text-muted-foreground/40" />
        <span className="text-sm text-muted-foreground">嵌入内容为空</span>
      </div>
    )
  }

  return (
    <div className="wem-embedblock">
      <div className="wem-embedblock-header">
        <span className="text-xs text-muted-foreground">嵌入代码</span>
        <button
          className="text-xs text-muted-foreground hover:text-foreground transition-colors cursor-pointer"
          onClick={() => setCollapsed(!collapsed)}
        >
          {collapsed ? '显示' : '隐藏'}
        </button>
      </div>
      {!collapsed && (
        <div
          className="wem-embedblock-content"
          dangerouslySetInnerHTML={{ __html: content }}
        />
      )}
    </div>
  )
}
