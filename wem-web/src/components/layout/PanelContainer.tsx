/**
 * PanelContainer — 面板容器
 *
 * 每个功能面板（文件树、目录等）都包裹在 PanelContainer 中。
 * 提供：
 * - 面板标题栏（从 panelRegistry 获取标题和图标）
 * - 内容区 overflow 管理
 * - 面板显示/隐藏由 ActivityBar 统一控制
 */

import type { ReactNode } from 'react'
import type { PanelConfig } from '@/stores/layoutStore'
import { getPanelDefinition } from './panelRegistry'

// ─── Types ───

export interface PanelContainerProps {
  panel: PanelConfig
  /** 面板内容 */
  children: ReactNode
}

// ─── Component ───

export function PanelContainer({ panel, children }: PanelContainerProps) {
  const def = getPanelDefinition(panel.type)
  const title = def?.title ?? panel.id
  const Icon = def?.icon

  return (
    <div className="flex flex-col flex-1 wem-panel" data-panel-id={panel.id}>
      {/* 面板标题栏 */}
      <div className="flex items-center gap-2 px-3 h-8 shrink-0 border-b border-border/30 text-muted-foreground/70 select-none">
        {Icon && <Icon className="h-3.5 w-3.5 shrink-0" />}
        <span className="text-xs font-medium tracking-wide uppercase truncate">{title}</span>
      </div>
      {/* 面板内容 */}
      <div className="flex-1 overflow-y-auto overflow-x-hidden">
        {children}
      </div>
    </div>
  )
}
